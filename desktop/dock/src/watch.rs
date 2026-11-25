//! Reloading the configuration when its file changes, without polling: inotify on its
//! directory, so that a file replaced by a rename (as `DockConfig::save` and most editors do)
//! is seen too.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use dock_config::DockConfig;
use iced::futures::Stream;
use iced::futures::channel::mpsc;

/// Events that may change the file: written, moved in or out, created, deleted.
const MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE;

/// Editors write in several steps: the events of a burst give one reload.
const SETTLE: Duration = Duration::from_millis(50);

/// The directory itself went away.
const GONE: u32 = libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_IGNORED;

/// The configuration at `path`, each time it changes. Problems are logged; an unreadable file
/// is logged and skipped. The thread stops when the stream is dropped.
pub fn watch(path: PathBuf) -> impl Stream<Item = Box<DockConfig>> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(String::from("watch-config"))
        .spawn(move || {
            if let Err(error) = run(&path, &sender) {
                tracing::warn!(%error, path = %path.display(), "cannot watch the configuration");
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "cannot start the configuration watcher");
    }
    receiver
}

/// Watches until the receiver is dropped (`Ok`) or inotify fails.
fn run(path: &Path, sender: &mpsc::UnboundedSender<Box<DockConfig>>) -> io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::other("no directory or file name"));
    };
    let mut last = load(path).ok();
    loop {
        if !dir.exists() {
            let (Some(parent), Some(dir_name)) = (dir.parent(), dir.file_name()) else {
                return Err(io::Error::other("no parent directory"));
            };
            let parent = Inotify::new(parent)?;
            while !dir.exists() {
                parent.wait(dir_name)?;
            }
        }
        // Watched from here on: nothing that comes after is missed, and what came before is
        // read just below.
        let watcher = Inotify::new(dir)?;
        loop {
            if let Ok(config) = load(path)
                && last.as_ref() != Some(&config)
            {
                tracing::info!(path = %path.display(), "configuration reloaded");
                last = Some(config.clone());
                if sender.unbounded_send(Box::new(config)).is_err() {
                    return Ok(());
                }
            }
            if !watcher.wait(name)? {
                break;
            }
            std::thread::sleep(SETTLE);
        }
    }
}

/// Reads the file and logs what it ignores.
pub fn load(path: &Path) -> io::Result<DockConfig> {
    match DockConfig::load(path) {
        Ok((config, problems)) => {
            for problem in problems {
                tracing::warn!(path = %path.display(), %problem, "configuration");
            }
            Ok(config)
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "cannot read the configuration");
            Err(error)
        }
    }
}

/// An inotify instance watching one directory.
struct Inotify {
    fd: OwnedFd,
}

impl Inotify {
    fn new(dir: &Path) -> io::Result<Self> {
        // SAFETY: inotify_init1 takes flags only and returns a new descriptor or -1.
        let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a descriptor we just opened and own alone; `OwnedFd` closes it.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let dir = CString::new(dir.as_os_str().as_bytes()).map_err(io::Error::other)?;
        // SAFETY: `fd` is open and `dir` a NUL-terminated string that outlives the call.
        if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), dir.as_ptr(), MASK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd })
    }

    /// Blocks until an event comes for `name`: `true`; or until the directory goes: `false`.
    fn wait(&self, name: &OsStr) -> io::Result<bool> {
        let mut buffer = [0u8; 4096];
        loop {
            // SAFETY: the buffer is ours, writable and `buffer.len()` long.
            let read = unsafe {
                libc::read(
                    self.fd.as_raw_fd(),
                    buffer.as_mut_ptr().cast::<libc::c_void>(),
                    buffer.len(),
                )
            };
            let Ok(read) = usize::try_from(read) else {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(error);
            };
            for (mask, event) in events(&buffer[..read]) {
                if mask & GONE != 0 {
                    return Ok(false);
                }
                if event == name.as_bytes() {
                    return Ok(true);
                }
            }
        }
    }
}

/// The mask and file name of each inotify event in `bytes`: a header (`wd`, `mask`, `cookie`,
/// `len`, four `u32`s) followed by `len` bytes of name, padded with NULs.
fn events(bytes: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    const HEADER: usize = 16;
    let mut offset = 0;
    std::iter::from_fn(move || {
        let header = bytes.get(offset..offset + HEADER)?;
        let field = |at: usize| {
            u32::from_ne_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        let len = field(12) as usize;
        let name = bytes.get(offset + HEADER..offset + HEADER + len)?;
        offset += HEADER + len;
        let end = name.iter().position(|b| *b == 0).unwrap_or(name.len());
        Some((field(4), &name[..end]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dock_config::Position;
    use iced::futures::StreamExt;

    #[test]
    fn event_names_are_read_from_the_buffer() {
        let mut bytes = Vec::new();
        for name in [&b"dock.conf\0\0\0\0\0\0\0"[..], &b"other\0\0\0"[..]] {
            bytes.extend_from_slice(&[0u8; 12]);
            bytes.extend_from_slice(&(name.len() as u32).to_ne_bytes());
            bytes.extend_from_slice(name);
        }
        let found: Vec<&[u8]> = events(&bytes).map(|(_, name)| name).collect();
        assert_eq!(found, [&b"dock.conf"[..], &b"other"[..]]);
    }

    #[test]
    fn a_saved_file_is_reloaded() {
        let dir = std::env::temp_dir().join(format!("dock-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // `dir/dock` does not exist yet: the watcher waits for it.
        std::fs::create_dir_all(&dir).expect("directory");
        let path = dir.join("dock/dock.conf");
        let mut stream = watch(path.clone());
        std::thread::sleep(Duration::from_millis(100));
        let mut config = DockConfig::default();
        config.position = Position::Left;
        config.save(&path).expect("saved");
        let changed = iced::futures::executor::block_on(stream.next()).expect("reloaded");
        assert_eq!(changed.position, Position::Left);
        // Replaced by a rename, as editors do.
        std::fs::write(dir.join("dock/new"), "[Dock]\nIconSize=40\n").expect("written");
        std::fs::rename(dir.join("dock/new"), &path).expect("renamed");
        let changed = iced::futures::executor::block_on(stream.next()).expect("reloaded");
        assert_eq!(changed.icon_size, 40.0);
    }
}
