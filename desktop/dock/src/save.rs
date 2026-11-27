//! Writing the pinned list off the interface thread: reading the file again, writing it and
//! flushing it to the disk take as long as the disk wants (a busy btrfs, a network home), and
//! the dock must keep drawing meanwhile. One thread of its own writes the changes in their
//! order; when several are waiting, only the last one is written.

use std::path::PathBuf;
use std::sync::mpsc;

use dock_config::DockConfig;

/// The writer of the pinned list into `dock.conf`.
#[derive(Debug, Default)]
pub struct Saver {
    sender: Option<mpsc::Sender<(PathBuf, Vec<String>)>>,
}

impl Saver {
    /// Writes `pinned` into the file at `path`, later and on the writer's thread, started at
    /// the first change. The file is read again first and only its list changes
    /// (`DockConfig::update`): a file that cannot be read is never written over.
    pub fn save(&mut self, path: PathBuf, pinned: Vec<String>) {
        let mut change = (path, pinned);
        if let Some(sender) = &self.sender {
            match sender.send(change) {
                Ok(()) => return,
                // The thread is gone: a new one takes the change.
                Err(mpsc::SendError(back)) => change = back,
            }
        }
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(String::from("dock-save"))
            .spawn(move || write_all(&receiver));
        match spawned {
            Ok(_) => {
                // The thread just started holds the receiver: this cannot fail.
                let _ = sender.send(change);
                self.sender = Some(sender);
            }
            Err(error) => tracing::warn!(%error, "cannot start the thread that saves the dock"),
        }
    }
}

/// Writes each change until the dock is gone; of several waiting, the last.
fn write_all(receiver: &mpsc::Receiver<(PathBuf, Vec<String>)>) {
    while let Ok(mut change) = receiver.recv() {
        while let Ok(newer) = receiver.try_recv() {
            change = newer;
        }
        let (path, pinned) = change;
        if let Err(error) = DockConfig::update(&path, |config| config.pinned = Some(pinned)) {
            tracing::warn!(%error, path = %path.display(), "cannot save the pinned applications");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn changes_are_written_in_their_order() {
        let dir = std::env::temp_dir().join(format!("dock-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock.conf");
        let mut saver = Saver::default();
        for last in ["a", "b", "c"] {
            saver.save(path.clone(), vec![String::from(last)]);
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let read = || DockConfig::load(&path).ok().and_then(|(c, _)| c.pinned);
        while read() != Some(vec![String::from("c")]) {
            assert!(Instant::now() < deadline, "{:?}", read());
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
