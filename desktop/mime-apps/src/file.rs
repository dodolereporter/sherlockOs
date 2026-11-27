//! Writing a settings file safely: `mimeapps.list` here, and every file Control Room and the
//! dock write (lot D-26a). The one implementation, so that every program keeps the same
//! promises: a reader sees the old file or the new one, never half of one; a file kept
//! elsewhere and linked into the configuration (dotfiles in a repository) is written where it
//! lives, even when the link is still dangling; the file keeps its permissions (a 0600 file
//! stays 0600); and a crash leaves the old file or the new one.

use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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

/// Writes `text` to `path`: to a temporary file beside the file the links name, created with
/// that file's permissions, flushed to the disk, renamed over it; then the folder is flushed
/// too, so that the rename survives a crash.
pub fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let target = resolve_links(path);
    let path = target.as_path();
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let count = WRITES.fetch_add(1, Ordering::Relaxed);
    let temporary = dir.join(format!(".{name}.{}.{count}.tmp", std::process::id()));
    // The permissions of the file replaced, from the start: its content is never readable by
    // more people than before, even for a moment.
    let mode = std::fs::metadata(path).ok().map(|m| m.permissions().mode());
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        if let Some(mode) = mode {
            options.mode(mode);
        }
        let mut file = options.open(&temporary)?;
        if let Some(mode) = mode {
            // `mode` above goes through the umask: set it exactly.
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written?;
    std::fs::File::open(dir)?.sync_all()
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
}
