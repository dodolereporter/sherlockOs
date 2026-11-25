//! File operations: copy, move, rename, duplicate, new folder.
//!
//! Nothing ever overwrites an existing file. A copy whose name is taken becomes
//! "name (copie)"; a move or a rename to a taken name is refused.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sherlock_ui::texts::evidence as texts;

/// What can prevent an operation. The message is meant for the user.
#[derive(Debug)]
pub enum Error {
    /// An item with this name already exists at the destination.
    Exists(String),
    /// Empty name, ".", ".." or containing "/".
    InvalidName,
    /// Copying or moving a folder into itself.
    IntoItself,
    /// The user's trash is not on the same disk.
    OtherDisk,
    /// The item is no longer in the trash.
    NotInTrash,
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exists(name) => f.write_str(&texts::already_exists(name)),
            Self::InvalidName => f.write_str(texts::INVALID_NAME),
            Self::IntoItself => f.write_str(texts::INTO_ITSELF),
            Self::OtherDisk => f.write_str(texts::OTHER_DISK),
            Self::NotInTrash => f.write_str(texts::NOT_IN_TRASH),
            Self::Io(error) => match error.kind() {
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
                    f.write_str(texts::NOT_ALLOWED)
                }
                io::ErrorKind::NotFound => f.write_str(texts::NOT_FOUND),
                io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => {
                    f.write_str(texts::DISK_FULL)
                }
                _ => f.write_str(&texts::system_error(&error.to_string())),
            },
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Refuses names a file system cannot hold.
pub fn validate_name(name: &str) -> Result<()> {
    let invalid =
        name.trim().is_empty() || name == "." || name == ".." || name.contains(['/', '\0']);
    if invalid {
        Err(Error::InvalidName)
    } else {
        Ok(())
    }
}

fn exists(path: &Path) -> bool {
    // A broken link also takes up its name.
    fs::symlink_metadata(path).is_ok()
}

fn file_name(path: &Path) -> Result<String> {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or(Error::InvalidName)
}

/// First free name in `dir`: `name` if it is, otherwise "name (copie)", "name (copie 2)"…
/// A file's extension stays at the end; a folder's is not split off.
pub fn free_name(dir: &Path, name: &str, is_dir: bool) -> String {
    if !exists(&dir.join(name)) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let (stem, extension) = match (is_dir, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(ext)) => (
            stem.to_string_lossy().into_owned(),
            Some(ext.to_string_lossy().into_owned()),
        ),
        _ => (name.to_owned(), None),
    };
    (1..)
        .map(|n| texts::copy_name(&stem, n, extension.as_deref()))
        .find(|candidate| !exists(&dir.join(candidate)))
        .unwrap_or_else(|| name.to_owned())
}

/// Refuses to put a folder into itself or into one of its subfolders.
fn check_not_inside(source: &Path, dest: &Path) -> Result<()> {
    let source = fs::canonicalize(source)?;
    let dest = fs::canonicalize(dest)?;
    if dest.starts_with(&source) {
        Err(Error::IntoItself)
    } else {
        Ok(())
    }
}

/// Recursive copy. Symbolic links are copied as links, not followed.
fn copy_recursive(from: &Path, to: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)
    } else if meta.is_dir() {
        fs::create_dir(to)?;
        for item in fs::read_dir(from)? {
            let item = item?;
            copy_recursive(&item.path(), &to.join(item.file_name()))?;
        }
        fs::set_permissions(to, meta.permissions())
    } else {
        fs::copy(from, to).map(|_| ())
    }
}

/// Copies each source into `dest`. Returns the created paths.
pub fn copy_into(sources: &[PathBuf], dest: &Path) -> Result<Vec<PathBuf>> {
    let mut created = Vec::with_capacity(sources.len());
    for source in sources {
        let is_dir = fs::symlink_metadata(source)?.is_dir();
        if is_dir {
            check_not_inside(source, dest)?;
        }
        let target = dest.join(free_name(dest, &file_name(source)?, is_dir));
        copy_recursive(source, &target)?;
        created.push(target);
    }
    Ok(created)
}

/// Copies each item next to itself: "name (copie)".
pub fn duplicate(sources: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut created = Vec::with_capacity(sources.len());
    for source in sources {
        let parent = source.parent().ok_or(Error::InvalidName)?;
        created.extend(copy_into(std::slice::from_ref(source), parent)?);
    }
    Ok(created)
}

/// Moves an item to `target`, even across disks (copy then delete).
pub fn move_path(source: &Path, target: &Path) -> Result<()> {
    if exists(target) {
        return Err(Error::Exists(file_name(target)?));
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_recursive(source, target)?;
            if fs::symlink_metadata(source)?.is_dir() {
                fs::remove_dir_all(source)?;
            } else {
                fs::remove_file(source)?;
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

/// Moves each source into `dest`. A source already in `dest` does not move. Returns the new
/// paths.
pub fn move_into(sources: &[PathBuf], dest: &Path) -> Result<Vec<PathBuf>> {
    let mut moved = Vec::with_capacity(sources.len());
    for source in sources {
        let target = dest.join(file_name(source)?);
        if source.parent() == Some(dest) {
            moved.push(target);
            continue;
        }
        if fs::symlink_metadata(source)?.is_dir() {
            check_not_inside(source, dest)?;
        }
        move_path(source, &target)?;
        moved.push(target);
    }
    Ok(moved)
}

/// Renames an item within its folder. Returns its new path.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_name(new_name)?;
    let parent = path.parent().ok_or(Error::InvalidName)?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(target);
    }
    if exists(&target) {
        return Err(Error::Exists(new_name.to_owned()));
    }
    fs::rename(path, &target)?;
    Ok(target)
}

/// Creates "Nouveau dossier", or "Nouveau dossier 2"… if the name is taken.
pub fn new_folder(dir: &Path) -> Result<PathBuf> {
    for n in 1.. {
        let path = dir.join(texts::new_folder_name(n));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::InvalidName)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent folder");
        }
        fs::write(path, content).expect("write");
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).expect("read")
    }

    #[test]
    fn copying_a_folder_copies_all_its_content() {
        let tmp = TempDir::new("copy");
        let (src, dest) = (tmp.path().join("projet"), tmp.path().join("dest"));
        write(&src.join("a.txt"), "a");
        write(&src.join("sous/b.txt"), "b");
        std::os::unix::fs::symlink("a.txt", src.join("lien")).expect("link");
        fs::create_dir(&dest).expect("folder");

        let created = copy_into(std::slice::from_ref(&src), &dest).expect("copy");
        assert_eq!(created, [dest.join("projet")]);
        assert_eq!(read(&dest.join("projet/sous/b.txt")), "b");
        assert_eq!(
            fs::read_link(dest.join("projet/lien")).expect("copied link"),
            Path::new("a.txt")
        );
        // The source is intact.
        assert_eq!(read(&src.join("a.txt")), "a");
    }

    #[test]
    fn a_copy_with_a_taken_name_becomes_copie_without_overwriting() {
        let tmp = TempDir::new("conflict");
        let file = tmp.path().join("notes.txt");
        write(&file, "original");
        let first = duplicate(std::slice::from_ref(&file)).expect("copy");
        let second = duplicate(std::slice::from_ref(&file)).expect("copy");
        assert_eq!(first, [tmp.path().join("notes (copie).txt")]);
        assert_eq!(second, [tmp.path().join("notes (copie 2).txt")]);
        assert_eq!(read(&file), "original");
        assert_eq!(read(&second[0]), "original");

        let dir = tmp.path().join("v1.2");
        fs::create_dir(&dir).expect("folder");
        assert_eq!(
            duplicate(&[dir]).expect("copy"),
            [tmp.path().join("v1.2 (copie)")]
        );
    }

    #[test]
    fn a_folder_does_not_go_into_itself() {
        let tmp = TempDir::new("itself");
        let dir = tmp.path().join("a");
        fs::create_dir_all(dir.join("b")).expect("folders");
        assert!(matches!(
            copy_into(std::slice::from_ref(&dir), &dir.join("b")),
            Err(Error::IntoItself)
        ));
        assert!(matches!(
            move_into(std::slice::from_ref(&dir), &dir),
            Err(Error::IntoItself)
        ));
    }

    #[test]
    fn moving_empties_the_source_and_refuses_a_taken_name() {
        let tmp = TempDir::new("move");
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        write(&a.join("x.txt"), "x");
        write(&a.join("y.txt"), "y");
        write(&b.join("y.txt"), "autre");

        let moved = move_into(&[a.join("x.txt")], &b).expect("move");
        assert_eq!(moved, [b.join("x.txt")]);
        assert!(!a.join("x.txt").exists());
        assert_eq!(read(&b.join("x.txt")), "x");

        let refused = move_into(&[a.join("y.txt")], &b);
        assert!(matches!(refused, Err(Error::Exists(ref name)) if name == "y.txt"));
        assert_eq!(read(&a.join("y.txt")), "y");
        assert_eq!(read(&b.join("y.txt")), "autre");
        // Moving to where the item already is does nothing.
        assert_eq!(
            move_into(&[b.join("x.txt")], &b).expect("in place"),
            [b.join("x.txt")]
        );
    }

    #[test]
    fn renaming_checks_the_name() {
        let tmp = TempDir::new("rename");
        let file = tmp.path().join("avant.txt");
        write(&file, "contenu");
        write(&tmp.path().join("pris.txt"), "");

        let renamed = rename(&file, "après.txt").expect("rename");
        assert_eq!(read(&renamed), "contenu");
        assert!(!file.exists());
        assert!(matches!(
            rename(&renamed, "pris.txt"),
            Err(Error::Exists(_))
        ));
        for bad in ["", "  ", ".", "..", "a/b"] {
            assert!(
                matches!(rename(&renamed, bad), Err(Error::InvalidName)),
                "{bad:?}"
            );
        }
        assert_eq!(rename(&renamed, "après.txt").expect("same name"), renamed);
    }

    #[test]
    fn a_new_folder_takes_a_free_name() {
        let tmp = TempDir::new("new");
        assert_eq!(
            new_folder(tmp.path()).expect("create"),
            tmp.path().join("Nouveau dossier")
        );
        assert_eq!(
            new_folder(tmp.path()).expect("create"),
            tmp.path().join("Nouveau dossier 2")
        );
    }

    #[test]
    fn errors_are_worded_in_french() {
        let missing = io::Error::from(io::ErrorKind::NotFound);
        assert_eq!(Error::from(missing).to_string(), texts::NOT_FOUND);
        assert_eq!(
            Error::Exists("a.txt".into()).to_string(),
            "« a.txt » existe déjà ici."
        );
    }
}
