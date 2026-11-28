//! Following the theme live, without polling. One thread waits with `poll` on two descriptors:
//!
//! - inotify, on the directory of the file, so that a file replaced by a rename (as most
//!   editors do) is seen too; while the directory is missing, on its closest existing ancestor;
//! - a timerfd on the real-time clock, armed on the absolute instant of the next switch of the
//!   automatic mode. It fires on wake-up when the instant passed during a suspend, and a change
//!   of the clock cancels it (`ECANCELED`): either way, the thread computes the theme again.

use std::ffi::{CString, OsStr, OsString};
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{AppearanceConfig, Problem, Theme, TimeOfDay, epoch_now};

/// Events that may change the file: written, moved in or out, created, deleted; and the end of
/// the watched directory itself.
const MASK: u32 = libc::IN_CLOSE_WRITE
    | libc::IN_MOVED_TO
    | libc::IN_MOVED_FROM
    | libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// The watched directory went away (deleted, moved, its watch removed), or events were lost.
const GONE: u32 =
    libc::IN_DELETE_SELF | libc::IN_MOVE_SELF | libc::IN_IGNORED | libc::IN_Q_OVERFLOW;

/// Editors write in several steps: the events of a burst give one reload.
const SETTLE: Duration = Duration::from_millis(50);

/// What `follow` reports.
#[derive(Debug)]
pub enum Event {
    /// The theme to show from now on.
    Theme(Theme),
    /// The Accessibility settings changed (lot D-26p): already applied to the process
    /// (`Accessibility::apply`); the theme comes again after it, for the program to redraw.
    Accessibility(crate::Accessibility),
    /// A value of the file was ignored.
    Problem(Problem),
    /// The file, the local time or the timer cannot be read, or the file cannot be watched.
    Error(io::Error),
}

/// Calls `on_event` with the theme of the file at `path`, then each time it changes: the file
/// is edited, or the automatic mode reaches its switch time. Runs on a thread of its own, which
/// stops the next time it reports something after `on_event` returned `false`. Fails only when
/// the timer or the thread cannot be created.
pub fn follow(
    path: PathBuf,
    on_event: impl FnMut(Event) -> bool + Send + 'static,
) -> io::Result<()> {
    let timer = Timer::new()?;
    std::thread::Builder::new()
        .name(String::from("follow-appearance"))
        .spawn(move || run(&path, &timer, on_event))
        .map(|_| ())
}

/// `follow`, for a program that only wants the themes: `on_theme` gets each one, and the
/// problems of the file and the errors go to the log.
pub fn follow_themes(
    path: PathBuf,
    mut on_theme: impl FnMut(Theme) -> bool + Send + 'static,
) -> io::Result<()> {
    follow(path, move |event| match event {
        Event::Theme(theme) => on_theme(theme),
        Event::Accessibility(_) => true,
        Event::Problem(problem) => {
            tracing::warn!(%problem, "appearance file");
            true
        }
        Event::Error(error) => {
            tracing::warn!(%error, "appearance file");
            true
        }
    })
}

fn run(path: &Path, timer: &Timer, mut on_event: impl FnMut(Event) -> bool) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        let _ = on_event(Event::Error(io::Error::other("no directory or file name")));
        return;
    };
    let mut watch: Option<Watch> = None;
    let mut config = AppearanceConfig::default();
    let mut reload = true;
    let mut last = None;
    let mut accessibility = None;
    loop {
        if watch.is_none() {
            match Watch::new(dir, name) {
                // Watched from here on: the reload below sees what came before.
                Ok(new) => (watch, reload) = (Some(new), true),
                Err(error) => {
                    if !on_event(Event::Error(error)) {
                        return;
                    }
                }
            }
        }
        if reload {
            reload = false;
            match AppearanceConfig::load(path) {
                Ok((read, problems)) => {
                    config = read;
                    // Applied to the whole process, then told: the theme follows, so that the
                    // program redraws with them (lot D-26p).
                    if accessibility != Some(config.accessibility) {
                        accessibility = Some(config.accessibility);
                        config.accessibility.apply();
                        last = None;
                        if !on_event(Event::Accessibility(config.accessibility)) {
                            return;
                        }
                    }
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
        let now = epoch_now();
        let (theme, deadline) = match TimeOfDay::at(now) {
            Ok(time) => {
                let (theme, next) = config.theme_at(time);
                (theme, next.map(|at| at.next_after(now)).transpose())
            }
            // Without the local time, the theme stays as it is.
            Err(error) => (last.unwrap_or_default(), Err(error)),
        };
        if last != Some(theme) {
            last = Some(theme);
            if !on_event(Event::Theme(theme)) {
                return;
            }
        }
        if let Err(error) = deadline.and_then(|at| timer.arm(at))
            && !on_event(Event::Error(error))
        {
            return;
        }
        match wait(watch.as_mut(), timer) {
            Ok(Wake::Clock | Wake::Nothing) => {}
            Ok(Wake::File) => {
                std::thread::sleep(SETTLE);
                reload = true;
            }
            Ok(Wake::Rewatch) => watch = None,
            Err(error) => {
                watch = None;
                if !on_event(Event::Error(error)) {
                    return;
                }
            }
        }
    }
}

/// Why the thread woke up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// The switch time came, or the clock changed.
    Clock,
    /// The file may have changed.
    File,
    /// The watched directory went away, the missing one appeared, or events were lost.
    Rewatch,
    /// Events about other files.
    Nothing,
}

/// Blocks until the timer or inotify has something.
fn wait(watch: Option<&mut Watch>, timer: &Timer) -> io::Result<Wake> {
    let fd = |raw| libc::pollfd {
        fd: raw,
        events: libc::POLLIN,
        revents: 0,
    };
    // A negative descriptor is ignored by `poll`: without a watch, the timer only.
    let watched = watch.as_ref().map_or(-1, |w| w.inotify.as_raw_fd());
    let mut fds = [fd(timer.file.as_raw_fd()), fd(watched)];
    loop {
        // SAFETY: `fds` is a valid array of two `pollfd` that outlives the call.
        if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } >= 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    if fds[0].revents != 0 {
        timer.clear()?;
        return Ok(Wake::Clock);
    }
    match watch {
        Some(watch) if fds[1].revents != 0 => watch.read(),
        _ => Ok(Wake::Nothing),
    }
}

/// What inotify watches: the directory of the file (`target` is then the file), or, while that
/// directory is missing, its closest existing ancestor (`target` is the next missing folder).
struct Watch {
    inotify: File,
    target: OsString,
    on_dir: bool,
}

impl Watch {
    fn new(dir: &Path, file: &OsStr) -> io::Result<Self> {
        loop {
            let (watched, target, on_dir) = if dir.exists() {
                (dir, file, true)
            } else {
                let Some((parent, child)) = dir
                    .ancestors()
                    .zip(dir.ancestors().skip(1))
                    .find(|(_, parent)| parent.exists())
                    .and_then(|(child, parent)| Some((parent, child.file_name()?)))
                else {
                    return Err(io::Error::other("no existing folder above the file"));
                };
                (parent, child, false)
            };
            let inotify = match inotify(watched) {
                Ok(inotify) => inotify,
                // Removed between the check and the watch: look again.
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            // The missing folder was created between the check and the watch: watch it.
            if !on_dir && watched.join(target).exists() {
                continue;
            }
            return Ok(Self {
                inotify,
                target: target.to_owned(),
                on_dir,
            });
        }
    }

    /// Reads the pending events, all of them.
    fn read(&mut self) -> io::Result<Wake> {
        let mut buffer = [0u8; 4096];
        loop {
            match self.inotify.read(&mut buffer) {
                Ok(read) => return Ok(classify(&buffer[..read], &self.target, self.on_dir)),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}

/// What a buffer of events means, read in full: a deleted directory sends the deletion of its
/// files and its own end in the same read, and the end must not be missed.
fn classify(bytes: &[u8], target: &OsStr, on_dir: bool) -> Wake {
    let (mut gone, mut named) = (false, false);
    for (mask, name) in events(bytes) {
        gone |= mask & GONE != 0;
        named |= name == target.as_bytes();
    }
    if gone || (named && !on_dir) {
        Wake::Rewatch
    } else if named {
        Wake::File
    } else {
        Wake::Nothing
    }
}

/// A new inotify instance watching the directory `dir`.
fn inotify(dir: &Path) -> io::Result<File> {
    // SAFETY: inotify_init1 takes flags only and returns a new descriptor or -1.
    let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a descriptor we just opened and own alone; `File` closes it.
    let file = File::from(unsafe { OwnedFd::from_raw_fd(raw) });
    let dir = CString::new(dir.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // SAFETY: the descriptor is open and `dir` a NUL-terminated string that outlives the call.
    if unsafe { libc::inotify_add_watch(raw, dir.as_ptr(), MASK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
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

/// A timer on the real-time clock.
struct Timer {
    file: File,
}

impl Timer {
    fn new() -> io::Result<Self> {
        let flags = libc::TFD_CLOEXEC | libc::TFD_NONBLOCK;
        // SAFETY: timerfd_create takes a clock and flags, and returns a new descriptor or -1.
        let raw = unsafe { libc::timerfd_create(libc::CLOCK_REALTIME, flags) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a descriptor we just opened and own alone; `File` closes it.
        let file = File::from(unsafe { OwnedFd::from_raw_fd(raw) });
        Ok(Self { file })
    }

    /// Fires at `at` (seconds since 1970), or disarms with `None`. Armed, a change of the
    /// real-time clock cancels it.
    fn arm(&self, at: Option<i64>) -> io::Result<()> {
        // SAFETY: `itimerspec` is plain data; all zeroes is a disarmed timer.
        let mut spec: libc::itimerspec = unsafe { std::mem::zeroed() };
        let mut flags = 0;
        if let Some(at) = at {
            spec.it_value.tv_sec = libc::time_t::try_from(at).map_err(io::Error::other)?;
            flags = libc::TFD_TIMER_ABSTIME | libc::TFD_TIMER_CANCEL_ON_SET;
        }
        let fd = self.file.as_raw_fd();
        // SAFETY: `fd` is our open timerfd and `spec` is valid; no old value is asked for.
        if unsafe { libc::timerfd_settime(fd, flags, &spec, std::ptr::null_mut()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Takes the expiry out of the descriptor. `ECANCELED` means the clock changed: like an
    /// expiry, the theme is computed again.
    fn clear(&self) -> io::Result<()> {
        let mut count = [0u8; 8];
        match (&self.file).read(&mut count) {
            Ok(_) => Ok(()),
            Err(error)
                if error.raw_os_error() == Some(libc::ECANCELED)
                    || error.kind() == io::ErrorKind::WouldBlock =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// One inotify event, as the kernel writes it: the name padded with NULs.
    fn event(mask: u32, name: &[u8]) -> Vec<u8> {
        let padded = name.len().div_ceil(16) * 16;
        let mut bytes = vec![0u8; 4];
        bytes.extend_from_slice(&mask.to_ne_bytes());
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&(padded as u32).to_ne_bytes());
        bytes.extend_from_slice(name);
        bytes.resize(16 + padded, 0);
        bytes
    }

    #[test]
    fn every_event_of_a_read_counts() {
        let file = OsStr::new("appearance.conf");
        let both = [event(0, b"appearance.conf"), event(0, b"x")].concat();
        let names: Vec<&[u8]> = events(&both).map(|(_, name)| name).collect();
        assert_eq!(names, [&b"appearance.conf"[..], &b"x"[..]]);
        // A deleted directory: its file, then its own end, in the same read.
        let deleted = [
            event(libc::IN_DELETE, b"appearance.conf"),
            event(libc::IN_DELETE_SELF, b""),
            event(libc::IN_IGNORED, b""),
        ]
        .concat();
        assert_eq!(classify(&deleted, file, true), Wake::Rewatch);
        let moved = event(libc::IN_MOVE_SELF, b"");
        assert_eq!(classify(&moved, file, true), Wake::Rewatch);
        let lost = event(libc::IN_Q_OVERFLOW, b"");
        assert_eq!(classify(&lost, file, true), Wake::Rewatch);
        let written = [
            event(libc::IN_CREATE, b"other"),
            event(libc::IN_CLOSE_WRITE, b"appearance.conf"),
        ]
        .concat();
        assert_eq!(classify(&written, file, true), Wake::File);
        let other = event(libc::IN_CLOSE_WRITE, b"other");
        assert_eq!(classify(&other, file, true), Wake::Nothing);
        // Waiting in an ancestor: the missing folder appears.
        let folder = OsStr::new("appearance");
        let created = event(libc::IN_CREATE, b"appearance");
        assert_eq!(classify(&created, folder, false), Wake::Rewatch);
    }

    /// Whether `file` has something to read within `timeout` milliseconds.
    fn ready(file: &File, timeout: i32) -> bool {
        let mut fds = [libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: `fds` is a valid array of one `pollfd`.
        unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout) == 1 }
    }

    #[test]
    fn a_switch_time_already_passed_fires_at_once() {
        // As after a suspend: the instant passed while the machine slept.
        let timer = Timer::new().unwrap();
        timer.arm(Some(epoch_now() - 60)).unwrap();
        assert!(ready(&timer.file, 1000));
        timer.clear().unwrap();
        assert!(!ready(&timer.file, 0));
        // Far ahead, or disarmed: nothing.
        timer.arm(Some(epoch_now() + 3600)).unwrap();
        assert!(!ready(&timer.file, 0));
        timer.arm(None).unwrap();
        assert!(!ready(&timer.file, 0));
    }

    #[test]
    fn the_theme_follows_the_file_and_its_folder() {
        let dir = std::env::temp_dir().join(format!("appearance-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // `config/appearance` does not exist yet: two missing levels.
        let folder = dir.join("config/appearance");
        let path = folder.join("appearance.conf");
        let (sender, themes) = mpsc::channel();
        follow(path.clone(), move |event| match event {
            Event::Theme(theme) => sender.send(theme).is_ok(),
            _ => true,
        })
        .unwrap();
        let next = || themes.recv_timeout(Duration::from_secs(5)).unwrap();
        let write = |theme: &str| {
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::write(&path, format!("[Appearance]\nTheme={theme}\n")).unwrap();
        };
        assert_eq!(next(), Theme::Dark);
        write("light");
        assert_eq!(next(), Theme::Light);
        // Replaced by a rename, as editors do.
        let new = folder.join("new");
        std::fs::write(&new, "[Appearance]\nTheme=dark\n").unwrap();
        std::fs::rename(&new, &path).unwrap();
        assert_eq!(next(), Theme::Dark);
        write("light");
        assert_eq!(next(), Theme::Light);
        // The folder deleted: the default theme, then the folder made again.
        std::fs::remove_dir_all(&folder).unwrap();
        assert_eq!(next(), Theme::Dark);
        write("light");
        assert_eq!(next(), Theme::Light);
        // The folder moved away: the default theme, then a new folder.
        std::fs::rename(&folder, dir.join("config/old")).unwrap();
        assert_eq!(next(), Theme::Dark);
        write("light");
        assert_eq!(next(), Theme::Light);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
