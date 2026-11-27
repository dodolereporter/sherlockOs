//! The log written to disk (lot D-25f). The session's log (`session.log`) gets the lines of the
//! compositor through a pipe; a forced restart loses what the system had not written yet, which
//! hid the end of a frozen lock screen. The lock asks for it at each lock and unlock, and every
//! `lock::LOG_SYNC` while locked: a thread of its own waits a little for the pipe's reader to
//! append the last lines, then has the file written to disk. The loop never waits for the disk.

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

/// The variable naming the log, set by `sherlockos-session`.
pub const LOG_VAR: &str = "SHERLOCKOS_LOG";
/// The time the pipe's reader has to append the last lines.
const DELAY: Duration = Duration::from_millis(300);

/// What asks for the log to be written to disk.
#[derive(Default)]
pub struct LogSync {
    sender: Option<mpsc::Sender<()>>,
    requests: u64,
}

impl LogSync {
    /// Writes `path` to disk on request, from a thread of its own; without a path, does nothing.
    pub fn new(path: Option<PathBuf>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("log-sync".into())
            .spawn(move || {
                while receiver.recv().is_ok() {
                    std::thread::sleep(DELAY);
                    // Requests come in bursts: one write for them all.
                    while receiver.try_recv().is_ok() {}
                    if let Err(err) = sync_file(&path) {
                        tracing::debug!("log: {}: {err}", path.display());
                    }
                }
            });
        match spawned {
            Ok(_) => Self {
                sender: Some(sender),
                requests: 0,
            },
            Err(err) => {
                tracing::warn!("log: not written to disk on lock: {err}");
                Self::default()
            }
        }
    }

    /// Asks for the log to be written to disk.
    pub fn request(&mut self) {
        self.requests += 1;
        if let Some(sender) = &self.sender {
            let _ = sender.send(());
        }
    }

    /// Requests made so far.
    pub fn requests(&self) -> u64 {
        self.requests
    }
}

/// Writes the file at `path` to disk.
pub fn sync_file(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_log_is_written_to_disk_on_request() {
        let path = std::env::temp_dir().join(format!("log-sync-{}", std::process::id()));
        std::fs::write(&path, b"session locked\n").unwrap();
        assert!(sync_file(&path).is_ok());
        let mut sync = LogSync::new(Some(path.clone()));
        sync.request();
        sync.request();
        assert_eq!(sync.requests(), 2);
        std::thread::sleep(DELAY * 2);
        std::fs::remove_file(&path).unwrap();
        // A file gone is no error of the loop: only a line of debug.
        sync.request();
        assert!(sync_file(&path).is_err());
        // Without a log, nothing runs.
        let mut none = LogSync::new(None);
        none.request();
        assert_eq!(none.requests(), 1);
    }
}
