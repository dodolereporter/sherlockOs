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

/// Events that may change the file: written, moved in or out, created, deleted; and the end of
/// the watched directory itself.
const MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

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
    // Nothing read yet: the first state goes to the dock too, so that a change between its
    // own reading at startup and this thread is not missed.
    let mut last = None;
    // Sends the file's state when it changed; `false` once the dock is gone.
    let send = |last: &mut Option<DockConfig>| {
        if let Ok(config) = load(path)
            && last.as_ref() != Some(&config)
        {
            tracing::info!(path = %path.display(), "configuration read");
            *last = Some(config.clone());
            return sender.unbounded_send(Box::new(config)).is_ok();
        }
        true
    };
    loop {
        // Also when the folder is missing: an absent file is the default configuration.
        if !send(&mut last) {
            return Ok(());
        }
        wait_for_dir(dir)?;
        // Watched from here on: nothing that comes after is missed, and what came before is
        // read just below.
        let watcher = Inotify::new(dir)?;
        loop {
            if !send(&mut last) {
                return Ok(());
            }
            if !watcher.wait(name)? {
                // The folder went away: wait for it to come back.
                break;
            }
            std::thread::sleep(SETTLE);
        }
    }
}

/// Blocks until `dir` exists. While it does not, watches its closest existing ancestor for the
/// next missing folder on the way, as many levels up as needed.
fn wait_for_dir(dir: &Path) -> io::Result<()> {
    while !dir.exists() {
        let Some((ancestor, next)) = dir
            .ancestors()
            .zip(dir.ancestors().skip(1))
            .find(|(_, parent)| parent.exists())
            .and_then(|(child, parent)| Some((parent, child.file_name()?)))
        else {
            return Err(io::Error::other(
                "no existing folder above the configuration",
            ));
        };
        let watcher = Inotify::new(ancestor)?;
        // Created between the check above and the watch: go on at once.
        if ancestor.join(next).exists() {
            continue;
        }
        let _ = watcher.wait(next)?;
    }
    Ok(())
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
            // The whole buffer: a deleted directory sends the deletion of its files and its own
            // end in the same read, and the end must not be missed.
            let (mut gone, mut named) = (false, false);
            for (mask, event) in events(&buffer[..read]) {
                gone |= mask & GONE != 0;
                named |= event == name.as_bytes();
            }
            if gone {
                return Ok(false);
            }
            if named {
                return Ok(true);
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
    fn the_configuration_is_followed_through_deletions() {
        let root = std::env::temp_dir().join(format!("dock-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("directory");
        // Two missing levels: `config/dock`. The watcher climbs to `root`.
        let path = root.join("config/dock/dock.conf");
        let mut stream = watch(path.clone());
        let mut next = || iced::futures::executor::block_on(stream.next()).expect("read");
        assert_eq!(*next(), DockConfig::default(), "the first state goes out");

        let save = |position| {
            let mut config = DockConfig::default();
            config.position = position;
            config.save(&path).expect("saved");
        };
        save(Position::Left);
        assert_eq!(next().position, Position::Left);
        // Replaced by a rename, as editors do.
        let dir = root.join("config/dock");
        std::fs::write(dir.join("new"), "[Dock]\nIconSize=40\n").expect("written");
        std::fs::rename(dir.join("new"), &path).expect("renamed");
        assert_eq!(next().icon_size, 40.0);
        // The file deleted: back to the defaults.
        std::fs::remove_file(&path).expect("deleted");
        assert_eq!(*next(), DockConfig::default());
        // The folder deleted with its file, then made again: still followed.
        save(Position::Right);
        assert_eq!(next().position, Position::Right);
        std::fs::remove_dir_all(root.join("config")).expect("folder deleted");
        assert_eq!(*next(), DockConfig::default());
        save(Position::Top);
        assert_eq!(next().position, Position::Top);
    }
}
