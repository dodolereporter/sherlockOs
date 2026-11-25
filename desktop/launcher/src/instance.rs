//! One launcher at a time. The first one holds a lock on `$XDG_RUNTIME_DIR/launcher.lock`
//! (`flock`) and writes its process id in it. A second one cannot take the lock: it sends
//! `SIGTERM` to the process id it reads there and stops, so launching it twice toggles it.
//!
//! The kernel drops the lock when its holder dies, crash included: a stale file never blocks.
//! The file is opened close-on-exec, so programs started from the launcher do not inherit it.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};

/// Name of the lock file.
const FILE: &str = "launcher.lock";

/// What starting a launcher found.
#[derive(Debug)]
pub enum Claim {
    /// No other launcher: this one runs, and keeps the lock for as long as it lives.
    First(File),
    /// Another launcher runs, now asked to close (its process id), when its id was readable.
    Second(Option<i32>),
}

/// `$XDG_RUNTIME_DIR`, where the lock lives; `None` when unset.
pub fn runtime_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
}

/// Takes the lock in `dir`, or closes the launcher that holds it.
pub fn claim(dir: &Path) -> io::Result<Claim> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
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

/// Sends `SIGTERM` to the process id written in `text`. Only a plain positive id other than
/// ours: 0, 1 or a negative number would reach a whole group, or init.
fn close(text: &str) -> Option<i32> {
    let pid: i32 = text.trim().parse().ok()?;
    let own = i32::try_from(std::process::id()).ok()?;
    if pid <= 1 || pid == own {
        return None;
    }
    // SAFETY: `kill` takes plain integers and touches no memory of ours; the id is a single
    // positive process, checked above.
    let sent = unsafe { libc::kill(pid, libc::SIGTERM) };
    (sent == 0).then_some(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("launcher-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("folder");
        dir
    }

    #[test]
    fn a_second_launcher_closes_the_first() {
        let dir = temp("instance");
        let lock = match claim(&dir).expect("claim") {
            Claim::First(file) => file,
            Claim::Second(_) => panic!("nobody held the lock"),
        };
        let written = std::fs::read_to_string(dir.join(FILE)).expect("lock file");
        assert_eq!(written, std::process::id().to_string());
        // Our own id is never signalled.
        assert!(matches!(claim(&dir), Ok(Claim::Second(None))));
        // A first launcher played by a child process that sleeps.
        let mut first = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        std::fs::write(dir.join(FILE), first.id().to_string()).expect("pid written");
        let closed = claim(&dir).expect("claim");
        assert!(matches!(closed, Claim::Second(Some(pid)) if pid as u32 == first.id()));
        let status = first.wait().expect("waited");
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(status.signal(), Some(libc::SIGTERM));
        // Once the first one is gone, the lock is free again.
        drop(lock);
        assert!(matches!(claim(&dir), Ok(Claim::First(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn odd_contents_signal_nobody() {
        for text in ["", "abc", "0", "1", "-5", " -1 "] {
            assert_eq!(close(text), None, "{text:?}");
        }
    }
}
