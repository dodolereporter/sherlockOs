//! File operations: copy, move, rename, duplicate, new folder.
//!
//! Nothing ever overwrites an existing file. A copy whose name is taken becomes
//! "name (copie)"; a move or a rename to a taken name is refused.

use std::ffi::{CString, OsStr, OsString};
use std::fmt;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
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
    /// Stopped by the user (lot E-11); what was being written is removed.
    Stopped,
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
            Self::Stopped => f.write_str(texts::STOPPED),
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

fn file_name(path: &Path) -> Result<&OsStr> {
    path.file_name().ok_or(Error::InvalidName)
}

fn exists_error(path: &Path) -> Error {
    // Only the message shows the name, so a lossy form is fine here.
    Error::Exists(
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

/// First free name in `dir`: `name` if it is, otherwise "name (copie)", "name (copie 2)"…
/// A file's extension stays at the end; a folder's is not split off. Names are kept as
/// raw bytes: a name that is not UTF-8 keeps its exact bytes.
pub fn free_name(dir: &Path, name: &OsStr, is_dir: bool) -> OsString {
    if !exists(&dir.join(name)) {
        return name.to_owned();
    }
    let path = Path::new(name);
    let (stem, extension) = match (is_dir, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(ext)) => (stem, Some(ext)),
        _ => (name, None),
    };
    (1..)
        .map(|n| {
            let mut candidate = stem.to_owned();
            candidate.push(texts::copy_suffix(n));
            if let Some(extension) = extension {
                candidate.push(".");
                candidate.push(extension);
            }
            candidate
        })
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

fn c_path(path: &Path) -> io::Result<CString> {
    Ok(CString::new(path.as_os_str().as_bytes())?)
}

/// `rename` that never replaces an existing target, even if one appears at the last moment
/// (`renameat2` with `RENAME_NOREPLACE`). A filesystem that does not support the flag falls
/// back to a check then a plain rename.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    let (from_c, to_c) = (c_path(from)?, c_path(to)?);
    // SAFETY: both pointers are NUL-terminated strings owned by `from_c` and `to_c`, alive
    // for the whole call; `AT_FDCWD` resolves relative paths like rename(2). The call only
    // reads the two strings.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from_c.as_ptr(),
            libc::AT_FDCWD,
            to_c.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EINVAL | libc::ENOSYS) => {
            if exists(to) {
                Err(io::ErrorKind::AlreadyExists.into())
            } else {
                fs::rename(from, to)
            }
        }
        _ => Err(error),
    }
}

/// What a copy did besides the copied items.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// The created (or moved) paths.
    pub paths: Vec<PathBuf>,
    /// Sockets and device files found in copied folders: never read, never recreated.
    pub skipped: usize,
}

/// Times of a source, to keep them on the copy.
fn times(meta: &fs::Metadata) -> fs::FileTimes {
    let mut times = fs::FileTimes::new();
    if let Ok(modified) = meta.modified() {
        times = times.set_modified(modified);
    }
    if let Ok(accessed) = meta.accessed() {
        times = times.set_accessed(accessed);
    }
    times
}

/// Copies one regular file into a new file that must not exist yet.
fn copy_file(from: &Path, to: &Path, meta: &fs::Metadata) -> io::Result<()> {
    // O_NONBLOCK: if the source was swapped for a FIFO since it was checked, opening it
    // does not wait for a writer.
    let mut source = fs::File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(from)?;
    if !source.metadata()?.is_file() {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut target = fs::File::options()
        .write(true)
        .create_new(true)
        .mode(meta.mode() & 0o7777)
        .open(to)?;
    // By chunks, counted and stoppable (lot E-11). A copy cut short leaves nothing behind.
    if let Err(error) = crate::transfer_job::copy_bytes(&mut source, &mut target) {
        drop(target);
        let _ = fs::remove_file(to);
        return Err(error);
    }
    target.set_permissions(meta.permissions())?;
    target.set_times(times(meta))
}

/// Recreates a named pipe without reading it.
fn make_fifo(to: &Path, mode: u32) -> io::Result<()> {
    let to_c = c_path(to)?;
    // SAFETY: `to_c` is a NUL-terminated string alive for the whole call; mkfifo only reads
    // it and creates the node, failing if the name exists.
    if unsafe { libc::mkfifo(to_c.as_ptr(), mode & 0o7777) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Recursive copy into `to`, which must not exist. Links are copied as links, named pipes
/// are recreated, sockets and devices are skipped: nothing special is ever read. `created`
/// tells whether `to` itself was created, so a failure can remove it.
fn copy_recursive(
    from: &Path,
    to: &Path,
    report: &mut Report,
    created: &mut bool,
) -> io::Result<()> {
    let meta = fs::symlink_metadata(from)?;
    let kind = meta.file_type();
    if kind.is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(from)?, to)?;
        *created = true;
    } else if kind.is_dir() {
        fs::create_dir(to)?;
        *created = true;
        for item in fs::read_dir(from)? {
            let item = item?;
            copy_recursive(&item.path(), &to.join(item.file_name()), report, &mut false)?;
        }
        fs::set_permissions(to, meta.permissions())?;
        fs::File::open(to)?.set_times(times(&meta))?;
    } else if kind.is_file() {
        copy_file(from, to, &meta)?;
        *created = true;
    } else if kind.is_fifo() {
        make_fifo(to, meta.mode())?;
        *created = true;
    } else {
        report.skipped += 1;
    }
    Ok(())
}

fn remove_tree(path: &Path) {
    let removed = match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
        Err(_) => Ok(()),
    };
    if let Err(error) = removed {
        tracing::warn!(%error, path = %path.display(), "partial copy not removed");
    }
}

/// Copies `from` to `to`; on failure, removes what it created at `to`.
fn copy_tree(from: &Path, to: &Path, report: &mut Report) -> Result<()> {
    let mut created = false;
    match copy_recursive(from, to, report, &mut created) {
        Ok(()) => Ok(()),
        Err(error) => {
            if created {
                remove_tree(to);
            }
            if error.kind() == io::ErrorKind::AlreadyExists && !created {
                return Err(exists_error(to));
            }
            if error.kind() == io::ErrorKind::Interrupted {
                return Err(Error::Stopped);
            }
            Err(error.into())
        }
    }
}

/// Copies each source into `dest`.
pub fn copy_into(sources: &[PathBuf], dest: &Path) -> Result<Report> {
    let mut report = Report::default();
    for source in sources {
        let is_dir = fs::symlink_metadata(source)?.is_dir();
        if is_dir {
            check_not_inside(source, dest)?;
        }
        let target = dest.join(free_name(dest, file_name(source)?, is_dir));
        copy_tree(source, &target, &mut report)?;
        report.paths.push(target);
    }
    Ok(report)
}

/// Copies `source` to `target`, which must not exist (lot E-11: the target is chosen by the
/// caller, after the user's choice on taken names).
pub fn copy_into_as(source: &Path, target: &Path) -> Result<Report> {
    let mut report = Report::default();
    if fs::symlink_metadata(source)?.is_dir()
        && let Some(dest) = target.parent()
    {
        check_not_inside(source, dest)?;
    }
    copy_tree(source, target, &mut report)?;
    report.paths.push(target.to_path_buf());
    Ok(report)
}

/// Copies each item next to itself: "name (copie)".
pub fn duplicate(sources: &[PathBuf]) -> Result<Report> {
    let mut report = Report::default();
    for source in sources {
        let parent = source.parent().ok_or(Error::InvalidName)?;
        let one = copy_into(std::slice::from_ref(source), parent)?;
        report.paths.extend(one.paths);
        report.skipped += one.skipped;
    }
    Ok(report)
}

/// Moves an item to `target`, even across disks (copy then delete), never over an
/// existing item.
pub fn move_path(source: &Path, target: &Path, report: &mut Report) -> Result<()> {
    match rename_noreplace(source, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(exists_error(target)),
        Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
            copy_tree(source, target, report)?;
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

/// Moves each source into `dest`. A source already in `dest` does not move.
pub fn move_into(sources: &[PathBuf], dest: &Path) -> Result<Report> {
    let mut report = Report::default();
    for source in sources {
        let target = dest.join(file_name(source)?);
        if source.parent() == Some(dest) {
            report.paths.push(target);
            continue;
        }
        if fs::symlink_metadata(source)?.is_dir() {
            check_not_inside(source, dest)?;
        }
        move_path(source, &target, &mut report)?;
        report.paths.push(target);
    }
    Ok(report)
}

/// Renames an item within its folder. Returns its new path.
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_name(new_name)?;
    let parent = path.parent().ok_or(Error::InvalidName)?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(target);
    }
    match rename_noreplace(path, &target) {
        Ok(()) => Ok(target),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            Err(Error::Exists(new_name.to_owned()))
        }
        Err(error) => Err(error.into()),
    }
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

        let created = copy_into(std::slice::from_ref(&src), &dest)
            .expect("copy")
            .paths;
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
        let first = duplicate(std::slice::from_ref(&file)).expect("copy").paths;
        let second = duplicate(std::slice::from_ref(&file)).expect("copy").paths;
        assert_eq!(first, [tmp.path().join("notes (copie).txt")]);
        assert_eq!(second, [tmp.path().join("notes (copie 2).txt")]);
        assert_eq!(read(&file), "original");
        assert_eq!(read(&second[0]), "original");

        let dir = tmp.path().join("v1.2");
        fs::create_dir(&dir).expect("folder");
        assert_eq!(
            duplicate(&[dir]).expect("copy").paths,
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

        let moved = move_into(&[a.join("x.txt")], &b).expect("move").paths;
        assert_eq!(moved, [b.join("x.txt")]);
        assert!(!a.join("x.txt").exists());
        assert_eq!(read(&b.join("x.txt")), "x");

        let refused = move_into(&[a.join("y.txt")], &b);
        assert!(matches!(refused, Err(Error::Exists(ref name)) if name == "y.txt"));
        assert_eq!(read(&a.join("y.txt")), "y");
        assert_eq!(read(&b.join("y.txt")), "autre");
        // Moving to where the item already is does nothing.
        assert_eq!(
            move_into(&[b.join("x.txt")], &b).expect("in place").paths,
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

    #[test]
    fn named_pipes_are_recreated_and_never_read() {
        let tmp = TempDir::new("fifo");
        let src = tmp.path().join("with-pipe");
        write(&src.join("a.txt"), "a");
        make_fifo(&src.join("pipe"), 0o644).expect("fifo");
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).expect("folder");
        // Would hang forever if the pipe were opened for reading.
        let report = copy_into(std::slice::from_ref(&src), &dest).expect("copy");
        let copied = fs::symlink_metadata(dest.join("with-pipe/pipe")).expect("pipe copied");
        assert!(copied.file_type().is_fifo());
        assert_eq!(report.skipped, 0);
        assert_eq!(read(&dest.join("with-pipe/a.txt")), "a");
    }

    #[test]
    fn names_that_are_not_utf8_keep_their_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let tmp = TempDir::new("bytes");
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        fs::create_dir_all(&a).expect("folder");
        fs::create_dir_all(&b).expect("folder");
        let name = OsString::from_vec(b"caf\xE9.txt".to_vec());
        let other = OsString::from_vec(b"caf\xE8.txt".to_vec());
        fs::write(a.join(&name), "1").expect("write");
        fs::write(a.join(&other), "2").expect("write");
        let copied = copy_into(&[a.join(&name)], &b).expect("copy").paths;
        assert_eq!(copied, [b.join(&name)]);
        // Two names that differ by an invalid byte stay two names.
        let moved = move_into(&[a.join(&other)], &b).expect("move").paths;
        assert_eq!(moved, [b.join(&other)]);
        let again = copy_into(&[a.join(&name)], &b).expect("copy").paths;
        let mut expected = OsString::from_vec(b"caf\xE9".to_vec());
        expected.push(" (copie).txt");
        assert_eq!(again, [b.join(expected)]);
    }

    #[test]
    fn a_copy_keeps_modification_times() {
        let tmp = TempDir::new("times");
        let src = tmp.path().join("old");
        write(&src.join("a.txt"), "a");
        let past =
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        let file = fs::File::options()
            .write(true)
            .open(src.join("a.txt"))
            .expect("open");
        file.set_times(fs::FileTimes::new().set_modified(past))
            .expect("times");
        fs::File::open(&src)
            .expect("open")
            .set_times(fs::FileTimes::new().set_modified(past))
            .expect("times");
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).expect("folder");
        copy_into(std::slice::from_ref(&src), &dest).expect("copy");
        let modified = |p: &Path| fs::metadata(p).expect("meta").modified().expect("time");
        assert_eq!(modified(&dest.join("old/a.txt")), past);
        assert_eq!(modified(&dest.join("old")), past);
    }

    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new("failed");
        let src = tmp.path().join("src");
        write(&src.join("a.txt"), "a");
        write(&src.join("z-locked.txt"), "secret");
        fs::set_permissions(src.join("z-locked.txt"), fs::Permissions::from_mode(0o000))
            .expect("chmod");
        if fs::File::open(src.join("z-locked.txt")).is_ok() {
            // Running as root: nothing can be unreadable, the case does not exist.
            return;
        }
        let dest = tmp.path().join("dest");
        fs::create_dir(&dest).expect("folder");
        assert!(copy_into(std::slice::from_ref(&src), &dest).is_err());
        assert_eq!(fs::read_dir(&dest).expect("dest").count(), 0);
        fs::set_permissions(src.join("z-locked.txt"), fs::Permissions::from_mode(0o644))
            .expect("chmod");
    }

    #[test]
    fn rename_never_replaces() {
        let tmp = TempDir::new("noreplace");
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        write(&a, "a");
        write(&b, "b");
        let error = rename_noreplace(&a, &b).expect_err("refused");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(read(&b), "b");
    }
}
