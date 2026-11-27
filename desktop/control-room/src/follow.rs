//! Changes made elsewhere while Control Room is open (audit C14): its settings files, changed
//! by hand or by their program (a pin in the dock), and the sound devices (a headset plugged
//! in). Each is followed on a thread of its own, which ends once the application stops
//! listening.

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use iced::futures::Stream;
use iced::futures::channel::mpsc;

/// How often the settings files are looked at: their modification time and length only.
pub const FILES_EVERY: Duration = Duration::from_secs(2);

/// Modification time and length of a file, if it exists.
type Stamp = Option<(SystemTime, u64)>;

fn stamp(path: &PathBuf) -> Stamp {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The files followed: the key of their subscription.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Files(pub Vec<PathBuf>);

/// A unit each time one of `files` changes, appears or goes away.
pub fn files(files: &Files) -> impl Stream<Item = ()> + use<> {
    let paths = files.0.clone();
    let (sender, receiver) = mpsc::unbounded();
    let started = std::thread::Builder::new()
        .name("follow-files".into())
        .spawn(move || {
            let mut last: Vec<Stamp> = paths.iter().map(stamp).collect();
            while !sender.is_closed() {
                std::thread::sleep(FILES_EVERY);
                let now: Vec<Stamp> = paths.iter().map(stamp).collect();
                if now != last {
                    last = now;
                    if sender.unbounded_send(()).is_err() {
                        return;
                    }
                }
            }
        });
    if let Err(error) = started {
        tracing::warn!(%error, "Control Room will not follow its files");
    }
    receiver
}

/// A unit each time the sound devices or their defaults change, through `pactl subscribe`
/// (`system_services::watch::sound`).
pub fn sound(pactl: &OsString) -> impl Stream<Item = ()> + use<> {
    let (sender, receiver) = mpsc::unbounded();
    let wanted = sender.clone();
    let watched = system_services::watch::sound(
        pactl.clone(),
        move || sender.unbounded_send(()).is_ok(),
        move || !wanted.is_closed(),
    );
    if let Err(error) = watched {
        tracing::warn!(%error, "Control Room will not follow the sound");
    }
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_changed_is_seen() {
        let dir = std::env::temp_dir().join(format!("follow-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("touchpad.conf");
        assert_eq!(stamp(&path), None);
        fs::write(&path, "speed = 0.4\n").unwrap();
        let first = stamp(&path);
        assert!(first.is_some());
        fs::write(&path, "speed = 0.45\n").unwrap();
        assert_ne!(stamp(&path), first, "the length changed");
        fs::remove_dir_all(&dir).unwrap();
    }
}
