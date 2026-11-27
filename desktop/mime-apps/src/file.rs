//! Writing a settings file safely: `mimeapps.list` here, and every file Control Room and the
//! dock write (lot D-26a). The one implementation, so that every program keeps the same
//! promises: a reader sees the old file or the new one, never half of one; a file kept
//! elsewhere and linked into the configuration (dotfiles in a repository) is written where it
//! lives, even when the link is still dangling; the file keeps its permissions (a 0600 file
//! stays 0600); and a crash leaves the old file or the new one.

use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Follows symbolic links to the file they name, even if it does not exist yet; gives up
/// after 40 links, like the kernel.
pub fn resolve_links(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    for _ in 0..40 {
        match std::fs::read_link(&current) {
            Ok(target) => {
                current = match current.parent() {
                    Some(parent) if target.is_relative() => parent.join(target),
                    _ => target,
                };
            }
            Err(_) => return current,
        }
    }
    current
}

/// Writes made by this process, for temporary names of their own: two writes at the same time,
/// from two threads, never share one.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// The largest settings file read: real ones hold a few kilobytes. A link to `/dev/zero` or a
/// huge file in the configuration folder would otherwise take all the memory of the program
/// that reads it (the compositor, the bar, the dock).
pub const MAX_SETTINGS_SIZE: u64 = 1024 * 1024;

/// The text of the settings file at `path`, `max` bytes at most, in UTF-8. Only a regular
/// file is read, once its links are followed: a pipe or a device is refused without being
/// opened for long. Errors keep their kind, so a missing file is still `NotFound`; one that
/// is too large or not UTF-8 is `InvalidData`.
pub fn read_bounded(path: &Path, max: u64) -> io::Result<String> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(max + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file too large"));
    }
    String::from_utf8(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "not UTF-8"))
}

/// Whether `value` would end its line in a settings file and start another: a value with a
/// line break is refused by every writer, never written.
pub fn breaks_line(value: &str) -> bool {
    value.contains(['\n', '\r'])
}

/// Writes `text` to `path`: to a temporary file beside the file the links name, created with
/// that file's permissions, flushed to the disk, renamed over it; then the folder is flushed
/// too, so that the rename survives a crash.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    write_bytes_atomically(path, text.as_bytes(), None)
}

/// Writes `bytes` to `path` the same way. A file that is there keeps its permissions and,
/// when this process may give it, its owner; a new file gets `new_mode` (through the umask),
/// or the usual 0666 through the umask. Temporary files left by a write that was stopped (a
/// crash, a power cut) are removed first. Every settings file of the desktop is written here.
pub fn write_bytes_atomically(path: &Path, bytes: &[u8], new_mode: Option<u32>) -> io::Result<()> {
    let target = resolve_links(path);
    let path = target.as_path();
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    remove_leftovers(dir, &name);
    let count = WRITES.fetch_add(1, Ordering::Relaxed);
    let temporary = dir.join(format!(".{name}.{}.{count}.tmp", std::process::id()));
    // The permissions of the file replaced, from the start: its content is never readable by
    // more people than before, even for a moment.
    let existing = std::fs::metadata(path).ok();
    let mode = existing.as_ref().map(|m| m.permissions().mode() & 0o7777);
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        if let Some(mode) = mode.or(new_mode) {
            options.mode(mode);
        }
        let mut file = options.open(&temporary)?;
        if let Some(existing) = &existing {
            // The owner too (root editing a user's file keeps it the user's); only a
            // privileged process may give a file away, so a refusal is not an error.
            let _ = std::os::unix::fs::fchown(&file, Some(existing.uid()), Some(existing.gid()));
        }
        if let Some(mode) = mode {
            // `mode` above goes through the umask: set it exactly.
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written?;
    // The file is replaced already: a folder that cannot be flushed (some network file
    // systems refuse it) makes the write less durable, not failed.
    if let Ok(folder) = std::fs::File::open(dir) {
        let _ = folder.sync_all();
    }
    Ok(())
}

/// Removes the temporary files of `name` in `dir` whose writer is gone: `.<name>.<pid>.<n>.tmp`
/// whose process no longer runs. Those of a living process are writes going on.
fn remove_leftovers(dir: &Path, name: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!(".{name}.");
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(rest) = file_name
            .to_str()
            .and_then(|n| n.strip_prefix(&prefix))
            .and_then(|n| n.strip_suffix(".tmp"))
        else {
            continue;
        };
        let Some((pid, count)) = rest.split_once('.') else {
            continue;
        };
        let numbers = pid.parse::<u32>().is_ok() && count.parse::<u64>().is_ok();
        if numbers && !Path::new("/proc").join(pid).exists() {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn permissions_and_links_are_kept() {
        let tmp = TempDir::new("file");
        let secret = tmp.path().join("secret.conf");
        std::fs::write(&secret, "old\n").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
        write_atomically(&secret, "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), "new\n");
        let mode = std::fs::metadata(&secret).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // A dangling link: the file is created where it points, the link stays a link.
        let link = tmp.path().join("linked.conf");
        std::os::unix::fs::symlink("dotfiles/linked.conf", &link).unwrap();
        write_atomically(&link, "made\n").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let target = tmp.path().join("dotfiles/linked.conf");
        assert_eq!(std::fs::read_to_string(target).unwrap(), "made\n");
        // Nothing left beside.
        let left: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(left.is_empty());
    }

    #[test]
    fn leftovers_of_a_stopped_write_are_removed() {
        let tmp = TempDir::new("leftovers");
        let path = tmp.path().join("a.conf");
        // A write stopped by a crash: its process is gone (no pid is above the kernel's
        // largest). The temporary file of a living process, this one, is a write going on.
        let dead = tmp.path().join(".a.conf.4194305.0.tmp");
        let alive = tmp
            .path()
            .join(format!(".a.conf.{}.999.tmp", std::process::id()));
        let other = tmp.path().join(".b.conf.4194305.0.tmp");
        for file in [&dead, &alive, &other] {
            std::fs::write(file, "half").unwrap();
        }
        write_atomically(&path, "whole\n").unwrap();
        assert!(!dead.exists() && alive.exists() && other.exists());
        // A new file takes the mode asked for.
        let script = tmp.path().join("run");
        write_bytes_atomically(&script, b"#!/bin/sh\n", Some(0o700)).unwrap();
        let mode = std::fs::metadata(&script).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn reads_are_bounded() {
        let tmp = TempDir::new("bounded");
        let path = tmp.path().join("big.conf");
        std::fs::write(&path, "x".repeat(100)).unwrap();
        assert_eq!(read_bounded(&path, 100).unwrap().len(), 100);
        let error = read_bounded(&path, 99).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        // A device behind a link is not read.
        let link = tmp.path().join("zero.conf");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert_eq!(
            read_bounded(&link, MAX_SETTINGS_SIZE).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let missing = tmp.path().join("missing.conf");
        assert_eq!(
            read_bounded(&missing, 10).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        std::fs::write(&path, b"caf\xe9").unwrap();
        assert_eq!(
            read_bounded(&path, 10).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(breaks_line("a\nb") && breaks_line("a\r") && !breaks_line("a b"));
    }
}
