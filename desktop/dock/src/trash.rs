//! The trash at the end of the dock (lot D-21j), as on macOS: full or empty as the user's
//! trash is (`$XDG_DATA_HOME/Trash`, the freedesktop trash that Evidence fills), followed with
//! inotify; a click opens it in Evidence; « Vider la corbeille » deletes what it holds, after a
//! second click. Reads and deletions run on threads of their own.

use std::io;
use std::path::{Path, PathBuf};

use iced::futures::Stream;
use iced::futures::channel::mpsc;

use crate::watch::{Inotify, SETTLE, wait_for_dir};

/// Entries deleted at most by one « Vider la corbeille »: a trash is emptied in batches, never
/// in one walk without end.
pub const MAX_EMPTIED: usize = 100_000;

/// The user's trash folder: `$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`.
pub fn folder() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")))?;
    Some(data.join("Trash"))
}

/// Whether `trash` holds something: an entry in its `files` folder.
pub fn full(trash: &Path) -> bool {
    std::fs::read_dir(trash.join("files")).is_ok_and(|mut dir| dir.next().is_some())
}

/// Deletes what `trash` holds, files and their records, never following a link out of it.
/// Returns how many entries went.
pub fn empty(trash: &Path) -> io::Result<usize> {
    let files = trash.join("files");
    let info = trash.join("info");
    let mut count = 0;
    let Ok(dir) = std::fs::read_dir(&files) else {
        return Ok(0);
    };
    for entry in dir.flatten().take(MAX_EMPTIED) {
        let path = entry.path();
        // `symlink_metadata`: a link is removed itself, never what it points to.
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path)?,
            Ok(_) => std::fs::remove_file(&path)?,
            Err(_) => continue,
        }
        let mut record = entry.file_name();
        record.push(".trashinfo");
        let _ = std::fs::remove_file(info.join(record));
        count += 1;
    }
    Ok(count)
}

/// Whether the trash at `trash` is full, each time it changes; the thread stops when the
/// stream is dropped.
pub fn watch(trash: PathBuf) -> impl Stream<Item = bool> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(String::from("watch-trash"))
        .spawn(move || {
            if let Err(error) = run(&trash.join("files"), &trash, &sender) {
                tracing::warn!(%error, "cannot follow the trash");
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "cannot start the trash watcher");
    }
    receiver
}

fn run(files: &Path, trash: &Path, sender: &mpsc::UnboundedSender<bool>) -> io::Result<()> {
    let mut last = None;
    let send = |last: &mut Option<bool>| {
        let now = full(trash);
        if *last == Some(now) {
            return true;
        }
        *last = Some(now);
        sender.unbounded_send(now).is_ok()
    };
    loop {
        if !send(&mut last) {
            return Ok(());
        }
        wait_for_dir(files)?;
        let watcher = Inotify::new(files)?;
        loop {
            if !send(&mut last) {
                return Ok(());
            }
            if !watcher.wait_any()? {
                break;
            }
            std::thread::sleep(SETTLE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::StreamExt;

    #[test]
    fn the_trash_fills_is_followed_and_empties() {
        let root = std::env::temp_dir().join(format!("dock-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let trash = root.join("Trash");
        let mut stream = watch(trash.clone());
        let mut next = || iced::futures::executor::block_on(stream.next()).expect("state");
        assert!(!next(), "no trash yet: empty");
        std::fs::create_dir_all(trash.join("files/folder")).unwrap();
        std::fs::create_dir_all(trash.join("info")).unwrap();
        std::fs::write(trash.join("files/folder/inside"), "x").unwrap();
        std::fs::write(trash.join("info/folder.trashinfo"), "[Trash Info]\n").unwrap();
        // A link out of the trash: removed, never what it points to.
        let outside = root.join("kept");
        std::fs::write(&outside, "keep me").unwrap();
        std::os::unix::fs::symlink(&outside, trash.join("files/link")).unwrap();
        assert!(next(), "full");
        assert_eq!(empty(&trash).unwrap(), 2);
        assert!(!next(), "emptied");
        assert!(outside.exists() && !trash.join("info/folder.trashinfo").exists());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
