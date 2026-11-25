//! Following the theme live, without polling: inotify on the directory of the file, so that a
//! file replaced by a rename (as most editors do) is seen too, and a timer that wakes up at
//! the next switch of the automatic mode.

use std::ffi::{CString, OsStr, c_char, c_int};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::{AppearanceConfig, Problem, Theme, TimeOfDay};

// The two inotify calls of the C library, declared here: the crate uses the standard library
// only. Values from `<sys/inotify.h>`.
unsafe extern "C" {
    fn inotify_init1(flags: c_int) -> c_int;
    fn inotify_add_watch(fd: c_int, path: *const c_char, mask: u32) -> c_int;
}
const IN_CLOEXEC: c_int = 0o2_000_000;
/// Events that may change the file: written, moved in or out, created, deleted.
const MASK: u32 = 0x008 | 0x040 | 0x080 | 0x100 | 0x200;
/// The directory itself went away: deleted, moved, watch removed.
const GONE: u32 = 0x400 | 0x800 | 0x8000;

/// Editors write in several steps: the events of a burst give one reload.
const SETTLE: Duration = Duration::from_millis(50);

/// What `follow` reports.
#[derive(Debug)]
pub enum Event {
    /// The theme to show from now on.
    Theme(Theme),
    /// A value of the file was ignored.
    Problem(Problem),
    /// The file or the clock cannot be read, or the file can no longer be watched.
    Error(io::Error),
}

/// Calls `on_event` with the theme of the file at `path`, then each time it changes: the file
/// is edited, or the automatic mode reaches its switch time. Runs on two threads of its own,
/// which stop once `on_event` returns `false`. Fails only when no thread can start.
pub fn follow(
    path: PathBuf,
    mut on_event: impl FnMut(Event) -> bool + Send + 'static,
) -> io::Result<()> {
    let (wake, woken) = mpsc::channel();
    let watched = path.clone();
    let watcher = std::thread::Builder::new()
        .name(String::from("watch-appearance"))
        .spawn(move || {
            let error = watch(&watched, &wake).err();
            let _ = wake.send(error);
        });
    let mut watching = watcher.is_ok();
    std::thread::Builder::new()
        .name(String::from("follow-appearance"))
        .spawn(move || {
            let mut last = None;
            let mut reload = true;
            let mut config = AppearanceConfig::default();
            loop {
                if reload {
                    match AppearanceConfig::load(&path) {
                        Ok((read, problems)) => {
                            config = read;
                            for problem in problems {
                                if !on_event(Event::Problem(problem)) {
                                    return;
                                }
                            }
                        }
                        Err(error) => {
                            if !on_event(Event::Error(error)) {
                                return;
                            }
                        }
                    }
                }
                // Without the clock, the automatic mode keeps the theme it has.
                let (theme, wait) = match TimeOfDay::now() {
                    Ok(now) => config.theme_at(now),
                    Err(error) => {
                        if !on_event(Event::Error(error)) {
                            return;
                        }
                        (last.unwrap_or_default(), None)
                    }
                };
                if last != Some(theme) {
                    last = Some(theme);
                    if !on_event(Event::Theme(theme)) {
                        return;
                    }
                }
                let signal = match (wait, watching) {
                    (Some(wait), true) => woken.recv_timeout(wait),
                    (None, true) => woken.recv().map_err(|_| RecvTimeoutError::Disconnected),
                    (Some(wait), false) => {
                        std::thread::sleep(wait);
                        Err(RecvTimeoutError::Timeout)
                    }
                    (None, false) => return,
                };
                reload = match signal {
                    Ok(None) => true,
                    Ok(Some(error)) => {
                        watching = false;
                        if !on_event(Event::Error(error)) {
                            return;
                        }
                        false
                    }
                    Err(RecvTimeoutError::Timeout) => false,
                    Err(RecvTimeoutError::Disconnected) => {
                        watching = false;
                        false
                    }
                };
            }
        })
        .map(|_| ())
}

/// Sends `None` each time the file may have changed, until the receiver is dropped (`Ok`) or
/// inotify fails. A missing directory is waited for in its parent.
fn watch(path: &Path, wake: &Sender<Option<io::Error>>) -> io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::other("no directory or file name"));
    };
    loop {
        if !dir.exists() {
            let (Some(parent), Some(dir_name)) = (dir.parent(), dir.file_name()) else {
                return Err(io::Error::other("no parent directory"));
            };
            let mut parent = Inotify::new(parent)?;
            while !dir.exists() {
                parent.wait(dir_name)?;
            }
        }
        let mut watcher = Inotify::new(dir)?;
        // Watched from here on: the reload that follows sees what came before.
        if wake.send(None).is_err() {
            return Ok(());
        }
        while watcher.wait(name)? {
            std::thread::sleep(SETTLE);
            if wake.send(None).is_err() {
                return Ok(());
            }
        }
    }
}

/// An inotify instance watching one directory.
struct Inotify {
    file: File,
}

impl Inotify {
    fn new(dir: &Path) -> io::Result<Self> {
        // SAFETY: inotify_init1 takes flags only and returns a new descriptor or -1.
        let raw = unsafe { inotify_init1(IN_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a descriptor we just opened and own alone; `OwnedFd` closes it.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let file = File::from(fd);
        let dir = CString::new(dir.as_os_str().as_bytes()).map_err(io::Error::other)?;
        // SAFETY: the descriptor is open and `dir` a NUL-terminated string that outlives the
        // call.
        if unsafe { inotify_add_watch(raw, dir.as_ptr(), MASK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { file })
    }

    /// Blocks until an event comes for `name`: `true`; or until the directory goes: `false`.
    fn wait(&mut self, name: &OsStr) -> io::Result<bool> {
        let mut buffer = [0u8; 4096];
        loop {
            let read = match self.file.read(&mut buffer) {
                Ok(read) => read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
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

    #[test]
    fn event_names_are_read_from_the_buffer() {
        let mut bytes = Vec::new();
        for name in [&b"appearance.conf\0"[..], &b"other\0\0\0"[..]] {
            bytes.extend_from_slice(&[0u8; 12]);
            bytes.extend_from_slice(&(name.len() as u32).to_ne_bytes());
            bytes.extend_from_slice(name);
        }
        let found: Vec<&[u8]> = events(&bytes).map(|(_, name)| name).collect();
        assert_eq!(found, [&b"appearance.conf"[..], &b"other"[..]]);
    }

    #[test]
    fn an_edited_file_changes_the_theme_live() {
        let dir = std::env::temp_dir().join(format!("appearance-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // `dir/appearance` does not exist yet: the watcher waits for it.
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("appearance/appearance.conf");
        let (sender, themes) = mpsc::channel();
        follow(path.clone(), move |event| match event {
            Event::Theme(theme) => sender.send(theme).is_ok(),
            _ => true,
        })
        .unwrap();
        let next = || themes.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(next(), Theme::Dark);
        std::thread::sleep(Duration::from_millis(100));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[Appearance]\nTheme=light\n").unwrap();
        assert_eq!(next(), Theme::Light);
        // Replaced by a rename, as editors do.
        let new = dir.join("appearance/new");
        std::fs::write(&new, "[Appearance]\nTheme=dark\n").unwrap();
        std::fs::rename(&new, &path).unwrap();
        assert_eq!(next(), Theme::Dark);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
