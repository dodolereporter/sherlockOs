//! The user's trash, following the freedesktop "Trash" 1.0 specification: each discarded
//! item goes to `$XDG_DATA_HOME/Trash/files`, along with a
//! `Trash/info/<name>.trashinfo` record that keeps its original path and deletion date.
//! Plasma, GNOME and `gio trash` read the same trash.
//!
//! Only the home folder's trash is handled: a file on another disk is refused
//! (`Error::OtherDisk`), with no silent copy.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::clock;
use crate::ops::{Error, Result};

const INFO_EXTENSION: &str = ".trashinfo";

/// An item of the trash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Name of the item in `Trash/files`, unique within the trash.
    pub id: String,
    /// Path where it is stored in `Trash/files`.
    pub path: PathBuf,
    /// Original path, where to restore it.
    pub original: PathBuf,
    /// Deletion date: the record's.
    pub deleted: Option<SystemTime>,
}

/// The trash, located in a `Trash` folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trash {
    files: PathBuf,
    info: PathBuf,
}

impl Trash {
    /// Trash in `root` (the `Trash` folder itself). Tests put one in a temporary folder.
    pub fn at(root: &Path) -> Self {
        Self {
            files: root.join("files"),
            info: root.join("info"),
        }
    }

    /// The user's trash: `$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`.
    pub fn user() -> Option<Self> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::home_dir().map(|home| home.join(".local/share")))?;
        Some(Self::at(&data.join("Trash")))
    }

    /// The `files` folder, where discarded items live.
    pub fn files_dir(&self) -> &Path {
        &self.files
    }

    fn ensure(&self) -> io::Result<()> {
        // The specification requires folders only the user can read.
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(&self.files)?;
        builder.create(&self.info)
    }

    fn info_path(&self, id: &str) -> PathBuf {
        self.info.join(format!("{id}{INFO_EXTENSION}"))
    }

    /// Moves an item to the trash. Returns its identifier.
    pub fn put(&self, path: &Path) -> Result<String> {
        self.ensure()?;
        let path = std::path::absolute(path)?;
        let meta = fs::symlink_metadata(&path)?;
        if meta.dev() != fs::metadata(&self.files)?.dev() {
            return Err(Error::OtherDisk);
        }
        let name = path
            .file_name()
            .ok_or(Error::InvalidName)?
            .to_string_lossy()
            .into_owned();
        let deleted = clock::now().map(clock::iso).unwrap_or_default();
        let record = format!(
            "[Trash Info]\nPath={}\nDeletionDate={deleted}\n",
            encode(&path)
        );
        // The record is created first, exclusively: it is what reserves the name.
        for n in 1usize.. {
            let id = match n {
                1 => name.clone(),
                n => format!("{name}.{n}"),
            };
            let info = self.info_path(&id);
            let mut file = match fs::File::options().write(true).create_new(true).open(&info) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            };
            let target = self.files.join(&id);
            if fs::symlink_metadata(&target).is_ok() {
                // An item without a record holds this name: keep the reserved record so the
                // name is not reused, as gio does.
                continue;
            }
            let moved = file
                .write_all(record.as_bytes())
                .and_then(|()| fs::rename(&path, &target));
            if let Err(error) = moved {
                let _ = fs::remove_file(&info);
                return Err(match error.kind() {
                    io::ErrorKind::CrossesDevices => Error::OtherDisk,
                    _ => error.into(),
                });
            }
            return Ok(id);
        }
        Err(Error::InvalidName)
    }

    /// The items of the trash. A record without an item, or the reverse, is skipped.
    pub fn list(&self) -> io::Result<Vec<Item>> {
        let entries = match fs::read_dir(&self.info) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut items = Vec::new();
        for entry in entries.filter_map(io::Result::ok) {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = file_name.strip_suffix(INFO_EXTENSION) else {
                continue;
            };
            let path = self.files.join(id);
            if fs::symlink_metadata(&path).is_err() {
                continue;
            }
            let Some(original) = fs::read_to_string(entry.path())
                .ok()
                .and_then(|s| parse(&s))
            else {
                continue;
            };
            items.push(Item {
                id: id.to_owned(),
                path,
                original,
                deleted: entry.metadata().and_then(|m| m.modified()).ok(),
            });
        }
        Ok(items)
    }

    /// Puts an item back at its original place, recreating its parent folders if needed.
    /// Refuses if another item has taken its place.
    pub fn restore(&self, id: &str) -> Result<PathBuf> {
        let info = self.info_path(id);
        let original = fs::read_to_string(&info)
            .ok()
            .and_then(|s| parse(&s))
            .ok_or(Error::NotInTrash)?;
        let path = self.files.join(id);
        if fs::symlink_metadata(&path).is_err() {
            return Err(Error::NotInTrash);
        }
        if let Some(parent) = original.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::ops::move_path(&path, &original)?;
        fs::remove_file(&info)?;
        Ok(original)
    }

    /// Empties the trash for good. Returns the number of items deleted.
    pub fn empty(&self) -> Result<usize> {
        let mut count = 0;
        if let Ok(entries) = fs::read_dir(&self.files) {
            for entry in entries {
                let path = entry?.path();
                if fs::symlink_metadata(&path)?.is_dir() {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_file(&path)?;
                }
                count += 1;
            }
        }
        if let Ok(entries) = fs::read_dir(&self.info) {
            for entry in entries {
                fs::remove_file(entry?.path())?;
            }
        }
        Ok(count)
    }
}

/// Encodes a path as the specification requires (RFC 2396): every byte outside the safe
/// characters becomes `%XX`.
pub fn encode(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Decodes a path encoded by `encode`.
pub fn decode(text: &str) -> Option<PathBuf> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(out)))
}

/// Reads the original path from a `.trashinfo` record.
fn parse(record: &str) -> Option<PathBuf> {
    let mut in_group = false;
    for line in record.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Trash Info]";
        } else if in_group && let Some(path) = line.strip_prefix("Path=") {
            return decode(path).filter(|p| p.is_absolute());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn setup(name: &str) -> (TempDir, Trash) {
        let tmp = TempDir::new(name);
        let trash = Trash::at(&tmp.path().join("donnees/Trash"));
        (tmp, trash)
    }

    #[test]
    fn trashing_then_restoring_puts_the_file_back() {
        let (tmp, trash) = setup("trash");
        let file = tmp.path().join("dossier/été 2026.txt");
        fs::create_dir_all(file.parent().expect("parent")).expect("folder");
        fs::write(&file, "souvenirs").expect("write");

        let id = trash.put(&file).expect("moved to trash");
        assert!(!file.exists());
        let info = fs::read_to_string(trash.info_path(&id)).expect("record");
        assert!(info.starts_with("[Trash Info]\nPath="));
        assert!(
            info.contains("/dossier/%C3%A9t%C3%A9%202026.txt\n"),
            "{info}"
        );
        assert!(info.contains("\nDeletionDate=2"));

        let items = trash.list().expect("list");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].original, file);
        assert!(items[0].deleted.is_some());

        // The original folder vanished in the meantime: it is recreated.
        fs::remove_dir(file.parent().expect("parent")).expect("empty folder");
        assert_eq!(trash.restore(&id).expect("restored"), file);
        assert_eq!(fs::read_to_string(&file).expect("read"), "souvenirs");
        assert!(trash.list().expect("list").is_empty());
        assert!(matches!(trash.restore(&id), Err(Error::NotInTrash)));
    }

    #[test]
    fn two_items_with_the_same_name_coexist() {
        let (tmp, trash) = setup("namesakes");
        let file = tmp.path().join("a.txt");
        fs::write(&file, "un").expect("write");
        let first = trash.put(&file).expect("first");
        fs::write(&file, "deux").expect("write");
        let second = trash.put(&file).expect("second");
        assert_ne!(first, second);
        assert_eq!(trash.list().expect("list").len(), 2);

        // The place is taken again: restoring refuses to overwrite.
        fs::write(&file, "trois").expect("write");
        assert!(matches!(trash.restore(&first), Err(Error::Exists(_))));
        assert_eq!(fs::read_to_string(&file).expect("read"), "trois");
    }

    #[test]
    fn emptying_deletes_everything_and_only_the_trash() {
        let (tmp, trash) = setup("empty");
        let dir = tmp.path().join("projet");
        fs::create_dir_all(dir.join("src")).expect("folder");
        fs::write(dir.join("src/main.rs"), "").expect("write");
        let keep = tmp.path().join("garde.txt");
        fs::write(&keep, "").expect("write");
        trash.put(&dir).expect("folder trashed");

        assert_eq!(trash.empty().expect("emptied"), 1);
        assert!(trash.list().expect("list").is_empty());
        assert!(keep.exists());
        assert_eq!(fs::read_dir(trash.files_dir()).expect("files").count(), 0);
    }

    #[test]
    fn encoding_round_trips() {
        let path = Path::new("/home/dodo/Téléchargements/a b%c#.txt");
        let encoded = encode(path);
        assert!(!encoded.contains(' ') && !encoded.contains('#'));
        assert_eq!(decode(&encoded).as_deref(), Some(path));
        assert_eq!(decode("/a%2"), None);
        assert_eq!(parse("[Trash Info]\nPath=relatif\n"), None);
    }
}
