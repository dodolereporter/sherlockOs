//! Copies and moves with their progress, a way to stop them, and a choice when a name is taken
//! (lot E-11, audit I-11). The work runs on one thread; `Progress` is shared with the interface,
//! which reads it at each frame and can ask it to stop. A stopped copy removes the item it was
//! writing, as a failed one does (`ops`). No iced here.

use std::cell::RefCell;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::ops::{self, Report};
use crate::trash::Trash;

/// Entries counted at most before a copy, to know its size: beyond, the bar shows what is
/// known.
const MAX_COUNTED: usize = 200_000;
/// Bytes copied between two looks at `Progress`.
const CHUNK: usize = 1024 * 1024;

/// How far an operation is, shared between its thread and the interface.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
    stop: AtomicBool,
}

impl Progress {
    /// Asks the operation to stop at its next chunk.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// From 0 to 1; 0 while the size is unknown.
    pub fn fraction(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        (self.done.load(Ordering::Relaxed) as f64 / total as f64).clamp(0.0, 1.0) as f32
    }
}

thread_local! {
    /// The progress of the operation running on this thread, read by `copy_bytes`.
    static CURRENT: RefCell<Option<Arc<Progress>>> = const { RefCell::new(None) };
}

/// Runs `job` with `progress` as the progress of this thread.
pub fn with_progress<T>(progress: Arc<Progress>, job: impl FnOnce() -> T) -> T {
    CURRENT.with(|c| *c.borrow_mut() = Some(progress));
    let out = job();
    CURRENT.with(|c| *c.borrow_mut() = None);
    out
}

/// Copies a file's bytes by chunks, counting them and stopping when asked: `Interrupted`.
pub fn copy_bytes(source: &mut impl Read, target: &mut impl Write) -> io::Result<u64> {
    let progress = CURRENT.with(|c| c.borrow().clone());
    let mut buffer = vec![0u8; CHUNK];
    let mut copied = 0;
    loop {
        if progress.as_ref().is_some_and(|p| p.stopped()) {
            return Err(io::ErrorKind::Interrupted.into());
        }
        let read = match source.read(&mut buffer) {
            Ok(0) => return Ok(copied),
            Ok(n) => n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        target.write_all(&buffer[..read])?;
        copied += read as u64;
        if let Some(p) = &progress {
            p.done.fetch_add(read as u64, Ordering::Relaxed);
        }
    }
}

/// The bytes of regular files under `paths`, at most `MAX_COUNTED` entries looked at.
pub fn size_of(paths: &[PathBuf]) -> u64 {
    let mut total = 0;
    let mut seen = 0;
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(path) = stack.pop() {
        seen += 1;
        if seen > MAX_COUNTED {
            break;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir()
            && let Ok(items) = fs::read_dir(&path)
        {
            stack.extend(items.filter_map(Result::ok).map(|i| i.path()));
        }
    }
    total
}

/// What to do with a name already taken in the destination, for all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    /// The item there goes to the trash, the new one takes its place.
    Replace,
    /// The new one gets another name: « nom (copie) ».
    KeepBoth,
    /// The new one is left aside.
    Skip,
}

/// The names of `sources` already taken in `dest` (where they would go), not counting an item
/// over itself.
pub fn taken(sources: &[PathBuf], dest: &Path) -> Vec<String> {
    sources
        .iter()
        .filter_map(|source| {
            let name = source.file_name()?;
            let target = dest.join(name);
            (target != *source && fs::symlink_metadata(&target).is_ok())
                .then(|| name.to_string_lossy().into_owned())
        })
        .collect()
}

/// Where `source` goes in `dest`, given the choice for taken names: none when it is skipped.
fn target(
    source: &Path,
    dest: &Path,
    taken: Taken,
    trash: Option<&Trash>,
) -> ops::Result<Option<PathBuf>> {
    let name = source.file_name().ok_or(ops::Error::InvalidName)?;
    let target = dest.join(name);
    if target == source || fs::symlink_metadata(&target).is_err() {
        return Ok(Some(target));
    }
    match taken {
        Taken::Skip => Ok(None),
        Taken::KeepBoth => {
            let is_dir = fs::symlink_metadata(source)?.is_dir();
            Ok(Some(dest.join(ops::free_name(dest, name, is_dir))))
        }
        // Never deleted for good: the item replaced goes to the trash.
        Taken::Replace => {
            let trash = trash.ok_or(ops::Error::Exists(name.to_string_lossy().into_owned()))?;
            trash.put(&target)?;
            Ok(Some(target))
        }
    }
}

/// Copies `sources` into `dest` with `progress`, taken names handled by `taken`.
pub fn copy(
    sources: &[PathBuf],
    dest: &Path,
    taken: Taken,
    trash: Option<&Trash>,
    progress: Arc<Progress>,
) -> ops::Result<Report> {
    progress.total.store(size_of(sources), Ordering::Relaxed);
    with_progress(progress, || {
        let mut report = Report::default();
        for source in sources {
            let Some(target) = target(source, dest, taken, trash)? else {
                continue;
            };
            let one = ops::copy_into_as(source, &target)?;
            report.paths.extend(one.paths);
            report.skipped += one.skipped;
        }
        Ok(report)
    })
}

/// Moves `sources` into `dest` with `progress` (a copy between disks counts), taken names
/// handled by `taken`.
pub fn move_(
    sources: &[PathBuf],
    dest: &Path,
    taken: Taken,
    trash: Option<&Trash>,
    progress: Arc<Progress>,
) -> ops::Result<Report> {
    progress.total.store(size_of(sources), Ordering::Relaxed);
    with_progress(progress, || {
        let mut report = Report::default();
        for source in sources {
            let Some(target) = target(source, dest, taken, trash)? else {
                continue;
            };
            if target != *source {
                ops::move_path(source, &target, &mut report)?;
            }
            report.paths.push(target);
        }
        Ok(report)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_copy_counts_its_bytes_and_stops_when_asked() {
        let tmp = TempDir::new("progress");
        let (src, dest) = (tmp.path().join("src"), tmp.path().join("dest"));
        fs::create_dir_all(&src).expect("folder");
        fs::create_dir_all(&dest).expect("folder");
        fs::write(src.join("a"), vec![1u8; 3 * CHUNK + 10]).expect("write");
        fs::write(src.join("b"), b"bb").expect("write");
        let progress = Arc::new(Progress::default());
        let sources = vec![src.join("a"), src.join("b")];
        copy(&sources, &dest, Taken::KeepBoth, None, progress.clone()).expect("copy");
        assert_eq!(
            progress.total.load(Ordering::Relaxed),
            3 * CHUNK as u64 + 12
        );
        assert_eq!(progress.fraction(), 1.0);
        // Asked to stop: nothing half-written is left.
        let stopped = Arc::new(Progress::default());
        stopped.stop();
        let other = tmp.path().join("other");
        fs::create_dir_all(&other).expect("folder");
        assert!(copy(&sources, &other, Taken::KeepBoth, None, stopped).is_err());
        assert!(!other.join("a").exists());
    }

    #[test]
    fn taken_names_are_replaced_kept_both_or_skipped() {
        let tmp = TempDir::new("taken");
        let trash = Trash::at(&tmp.path().join("donnees/Trash"));
        let (src, dest) = (tmp.path().join("src"), tmp.path().join("dest"));
        fs::create_dir_all(&src).expect("folder");
        fs::create_dir_all(&dest).expect("folder");
        fs::write(src.join("a.txt"), "nouveau").expect("write");
        fs::write(dest.join("a.txt"), "ancien").expect("write");
        let sources = vec![src.join("a.txt")];
        assert_eq!(taken(&sources, &dest), ["a.txt"]);
        let p = || Arc::new(Progress::default());
        copy(&sources, &dest, Taken::Skip, Some(&trash), p()).expect("skip");
        assert_eq!(
            fs::read_to_string(dest.join("a.txt")).expect("read"),
            "ancien"
        );
        copy(&sources, &dest, Taken::KeepBoth, Some(&trash), p()).expect("both");
        assert_eq!(fs::read_dir(&dest).expect("list").count(), 2);
        copy(&sources, &dest, Taken::Replace, Some(&trash), p()).expect("replace");
        assert_eq!(
            fs::read_to_string(dest.join("a.txt")).expect("read"),
            "nouveau"
        );
        // The replaced one is in the trash, not lost.
        assert_eq!(trash.list().expect("trash").len(), 1);
        // A move onto a taken name, kept both.
        fs::write(src.join("b.txt"), "b").expect("write");
        fs::write(dest.join("b.txt"), "x").expect("write");
        move_(&[src.join("b.txt")], &dest, Taken::KeepBoth, None, p()).expect("move");
        assert!(!src.join("b.txt").exists());
        assert_eq!(fs::read_dir(&dest).expect("list").count(), 4);
    }
}
