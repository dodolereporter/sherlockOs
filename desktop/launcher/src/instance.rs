//! One launcher at a time. The first one holds a lock on `$XDG_RUNTIME_DIR/launcher.lock`
//! (`flock`) and writes its process id in it. A second one cannot take the lock: it asks the
//! process whose id it reads there to close (`SIGTERM`) and stops, so launching it twice
//! toggles it.
//!
//! The id read may be stale: a launcher killed by `SIGTERM` leaves its id behind, and a second
//! launcher may read the file before the first one rewrote it. So the signal only goes to a
//! launcher: the process is pinned with `pidfd_open`, its program (`/proc/<id>/exe`) must be
//! ours (`/proc/self/exe`), and the signal goes through the pidfd (`pidfd_send_signal`). Should
//! the id be reused between the check and the signal, the pidfd still names the dead process
//! and nothing is sent.
//!
//! The kernel drops the lock when its holder dies, crash included: a stale file never blocks.
//! The file is opened close-on-exec, so programs started from the launcher do not inherit it,
//! and without following a symbolic link. A launcher that closes normally empties it.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Name of the lock file.
const FILE: &str = "launcher.lock";

/// What starting a launcher found.
#[derive(Debug)]
pub enum Claim {
    /// No other launcher: this one runs, and keeps the lock for as long as it lives.
    First(File),
    /// Another launcher runs, now asked to close (its process id) when the id read in the
    /// file was that launcher's.
    Second(Option<i32>),
}

/// `$XDG_RUNTIME_DIR`, where the lock lives; `None` when unset or relative (the XDG
/// specification ignores a relative path).
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_absolute())
}

/// Takes the lock in `dir`, or closes the launcher that holds it.
pub fn claim(dir: &Path) -> io::Result<Claim> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(FILE))?;
    match file.try_lock() {
        Ok(()) => {
            file.set_len(0)?;
            file.rewind()?;
            write!(file, "{}", std::process::id())?;
            Ok(Claim::First(file))
        }
        Err(TryLockError::WouldBlock) => {
            let mut text = String::new();
            file.read_to_string(&mut text)?;
            Ok(Claim::Second(close(&text)))
        }
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// Empties the lock file of a launcher that closes normally, then lets the lock go.
pub fn release(lock: File) {
    if let Err(error) = lock.set_len(0) {
        tracing::warn!(%error, "cannot empty the launcher lock");
    }
}

/// Asks the launcher whose process id is written in `text` to close. Only a plain id above 1
/// and other than ours, and only a process running our own program.
fn close(text: &str) -> Option<i32> {
    let pid: i32 = text.trim().parse().ok()?;
    let own = i32::try_from(std::process::id()).ok()?;
    if pid <= 1 || pid == own {
        return None;
    }
    // SAFETY: `pidfd_open` takes a process id and flags, touches no memory of ours, and returns
    // a new descriptor or -1.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    let raw = i32::try_from(raw).ok().filter(|fd| *fd >= 0)?;
    // SAFETY: `raw` is a descriptor the kernel just opened for us and nothing else owns.
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw) };
    let theirs = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let ours = std::fs::read_link("/proc/self/exe").ok()?;
    if theirs != ours {
        tracing::warn!(pid, program = %theirs.display(), "the lock names another program");
        return None;
    }
    // SAFETY: the pidfd is open for the whole call; no `siginfo` is passed (null) and the
    // flags are 0, as the call requires.
    let sent = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    (sent == 0).then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    /// Set for the copy of this test program that plays a running launcher.
    const STAND_IN: &str = "LAUNCHER_TEST_STAND_IN";

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("folder");
        dir
    }

    /// Not a test: the body of the stand-in launcher, a copy of this very program, so that
    /// `/proc/<id>/exe` matches. It waits for its signal.
    #[test]
    #[ignore = "run by a_second_launcher_closes_the_first only"]
    fn stand_in() {
        if std::env::var_os(STAND_IN).is_some() {
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
    }

    fn holds(dir: &Path) -> File {
        match claim(dir).expect("claim") {
            Claim::First(file) => file,
            Claim::Second(_) => panic!("nobody held the lock"),
        }
    }

    #[test]
    fn a_second_launcher_closes_the_first() {
        let dir = temp("instance");
        let lock = holds(&dir);
        let written = std::fs::read_to_string(dir.join(FILE)).expect("lock file");
        assert_eq!(written, std::process::id().to_string());
        // Our own id is never signalled.
        assert!(matches!(claim(&dir), Ok(Claim::Second(None))));
        // A first launcher played by a copy of this program.
        let mut first = std::process::Command::new(std::env::current_exe().expect("program"))
            .args(["--ignored", "--exact", "instance::tests::stand_in"])
            .env(STAND_IN, "1")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("stand-in");
        std::fs::write(dir.join(FILE), first.id().to_string()).expect("pid written");
        let closed = claim(&dir).expect("claim");
        assert!(matches!(closed, Claim::Second(Some(pid)) if pid as u32 == first.id()));
        let status = first.wait().expect("waited");
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        // A normal close empties the file; the lock is free again.
        release(lock);
        assert_eq!(std::fs::read_to_string(dir.join(FILE)).expect("file"), "");
        assert!(matches!(claim(&dir), Ok(Claim::First(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_live_process_that_is_not_a_launcher_is_never_signalled() {
        let dir = temp("stranger");
        let _lock = holds(&dir);
        let mut stranger = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        std::fs::write(dir.join(FILE), stranger.id().to_string()).expect("pid written");
        assert!(matches!(claim(&dir), Ok(Claim::Second(None))));
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            stranger.try_wait().expect("state").is_none(),
            "still running"
        );
        stranger.kill().expect("killed");
        let _ = stranger.wait();
        // A dead id: nothing to open, nobody signalled.
        std::fs::write(dir.join(FILE), stranger.id().to_string()).expect("pid written");
        assert!(matches!(claim(&dir), Ok(Claim::Second(None))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_linked_lock_file_is_refused() {
        let dir = temp("link");
        let target = dir.join("target");
        std::fs::write(&target, "keep").expect("target");
        std::os::unix::fs::symlink(&target, dir.join(FILE)).expect("link");
        assert!(claim(&dir).is_err());
        assert_eq!(std::fs::read_to_string(&target).expect("target"), "keep");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn odd_contents_signal_nobody() {
        for text in ["", "abc", "0", "1", "-5", " -1 "] {
            assert_eq!(close(text), None, "{text:?}");
        }
    }
}
