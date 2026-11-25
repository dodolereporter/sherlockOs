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
        // The record is named after the item plus `.N.trashinfo`: a long name is cut so that
        // the record's name still fits the 255-byte limit of most filesystems.
        let name = cut_to_bytes(&name, ID_MAX_BYTES);
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
                // An item without a record holds this name: give the name back and try the
                // next one, without leaving an empty record behind.
                drop(file);
                let _ = fs::remove_file(&info);
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
            let Ok(record) = fs::read_to_string(entry.path()) else {
                continue;
            };
            let Some(original) = parse(&record) else {
                continue;
            };
            let deleted = deletion_date(&record);
            items.push(Item {
                id: id.to_owned(),
                path,
                original,
                // The date written in the record; the record's own time if it cannot be read.
                deleted: deleted.or_else(|| entry.metadata().and_then(|m| m.modified()).ok()),
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
        crate::ops::move_path(&path, &original, &mut crate::ops::Report::default())?;
        fs::remove_file(&info)?;
        Ok(original)
    }

    /// Deletes for good the given items, and only them: what arrived in the trash after the
    /// user was asked stays. Returns how many were deleted.
    pub fn remove(&self, ids: &[String]) -> Result<usize> {
        let mut count = 0;
        for id in ids {
            // An id is a plain name inside the trash, never a path elsewhere.
            if id.is_empty() || id.contains('/') || id == "." || id == ".." {
                continue;
            }
            let path = self.files.join(id);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => fs::remove_dir_all(&path)?,
                Ok(_) => fs::remove_file(&path)?,
                Err(_) => continue,
            }
            let _ = fs::remove_file(self.info_path(id));
            count += 1;
        }
        Ok(count)
    }
}

/// Longest id kept from a name, in bytes: room is left for `.N.trashinfo`.
const ID_MAX_BYTES: usize = 200;

/// Cuts a name to at most `max` bytes, on a character boundary.
fn cut_to_bytes(name: &str, max: usize) -> String {
    let mut end = name.len().min(max);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// Reads `DeletionDate=` (local time, `2026-11-24T21:46:00`) from a record.
fn deletion_date(record: &str) -> Option<SystemTime> {
    let value = record
        .lines()
        .find_map(|line| line.trim().strip_prefix("DeletionDate="))?;
    let number = |range: std::ops::Range<usize>| value.get(range)?.parse::<u16>().ok();
    let time = sherlock_ui::texts::LocalTime {
        year: i32::from(number(0..4)?),
        month: u8::try_from(number(5..7)?).ok()?,
        day: u8::try_from(number(8..10)?).ok()?,
        hour: u8::try_from(number(11..13)?).ok()?,
        minute: u8::try_from(number(14..16)?).ok()?,
        second: u8::try_from(number(17..19)?).ok()?,
    };
    clock::to_system(time)
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

        let ids: Vec<String> = trash
            .list()
            .expect("list")
            .into_iter()
            .map(|i| i.id)
            .collect();
        // Something trashed after the question stays.
        let late = tmp.path().join("late.txt");
        fs::write(&late, "").expect("write");
        trash.put(&late).expect("trashed");
        assert_eq!(trash.remove(&ids).expect("emptied"), 1);
        assert_eq!(trash.list().expect("list").len(), 1);
        let late_id: Vec<String> = trash
            .list()
            .expect("list")
            .into_iter()
            .map(|i| i.id)
            .collect();
        assert_eq!(trash.remove(&late_id).expect("emptied"), 1);
        assert!(trash.list().expect("list").is_empty());
        assert!(keep.exists());
        assert_eq!(fs::read_dir(trash.files_dir()).expect("files").count(), 0);
    }

    #[test]
    fn encoding_round_trips() {
        let path = Path::new("/home/utilisateur/Téléchargements/a b%c#.txt");
        let encoded = encode(path);
        assert!(!encoded.contains(' ') && !encoded.contains('#'));
        assert_eq!(decode(&encoded).as_deref(), Some(path));
        assert_eq!(decode("/a%2"), None);
        assert_eq!(parse("[Trash Info]\nPath=relatif\n"), None);
    }

    #[test]
    fn long_names_orphans_and_the_deletion_date() {
        let (tmp, trash) = setup("details");
        // A name near the 255-byte limit still goes to the trash.
        let long = tmp.path().join("é".repeat(120));
        fs::write(&long, "").expect("write");
        let id = trash.put(&long).expect("trashed");
        assert!(id.len() <= ID_MAX_BYTES);
        assert_eq!(trash.restore(&id).expect("restored"), long);
        // An orphan item holding the name: no empty record is left behind.
        let file = tmp.path().join("b.txt");
        fs::write(&file, "").expect("write");
        fs::write(trash.files_dir().join("b.txt"), "orphan").expect("write");
        let id = trash.put(&file).expect("trashed");
        assert_eq!(id, "b.txt.2");
        assert!(!trash.info_path("b.txt").exists());
        // The listed date is the one written in the record.
        let record = format!(
            "[Trash Info]\nPath={}\nDeletionDate=2001-09-09T03:46:40\n",
            encode(&file)
        );
        fs::write(trash.info_path(&id), record).expect("write");
        let item = trash
            .list()
            .expect("list")
            .into_iter()
            .find(|i| i.id == id)
            .expect("item");
        let local = item.deleted.and_then(clock::local).expect("date");
        assert_eq!(
            (local.year, local.month, local.day, local.hour),
            (2001, 9, 9, 3)
        );
    }
}
