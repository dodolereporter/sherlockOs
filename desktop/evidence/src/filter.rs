//! What the search keeps (lot E-15), as the Finder's tokens: kinds of items, a date, a size,
//! and whether it looks in the folder only or in its subfolders too. The walk through the
//! subfolders is bounded in items, depth, results and time, can be stopped, and never follows
//! a link.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::listing::{self, Entry, Kind};

/// Items looked at, at most, in a walk through the subfolders.
pub const MAX_VISITED: usize = 50_000;
/// Levels under the folder, at most.
pub const MAX_DEPTH: usize = 12;
/// Results kept, at most.
pub const MAX_RESULTS: usize = 2000;
/// Time a walk gets.
pub const MAX_TIME: Duration = Duration::from_secs(5);

/// A kind of item, as the « Type » token offers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KindFilter {
    Image,
    Document,
    Video,
    Audio,
    Archive,
    Folder,
}

impl KindFilter {
    pub const ALL: [Self; 6] = [
        Self::Image,
        Self::Document,
        Self::Video,
        Self::Audio,
        Self::Archive,
        Self::Folder,
    ];

    /// Whether `entry` is of this kind, by its extension.
    pub fn holds(self, entry: &Entry) -> bool {
        if self == Self::Folder {
            return entry.kind == Kind::Folder;
        }
        if entry.kind != Kind::File {
            return false;
        }
        let Some(extension) = entry.path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        let extension = extension.to_lowercase();
        let known: &[&str] = match self {
            Self::Image => &[
                "png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "tif", "tiff", "heic", "avif",
            ],
            Self::Document => &[
                "pdf", "txt", "md", "rtf", "odt", "ods", "odp", "doc", "docx", "xls", "xlsx",
                "ppt", "pptx", "csv", "epub",
            ],
            Self::Video => &["mp4", "mkv", "webm", "mov", "avi", "m4v", "ogv"],
            Self::Audio => &["mp3", "flac", "ogg", "oga", "wav", "m4a", "opus", "aac"],
            Self::Archive => &["zip", "tar", "gz", "tgz", "xz", "bz2", "7z", "rar", "zst"],
            Self::Folder => &[],
        };
        known.contains(&extension.as_str())
    }
}

/// How recent, as the « Date » token offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateFilter {
    Today,
    Week,
    Month,
    Year,
}

impl DateFilter {
    pub const ALL: [Self; 4] = [Self::Today, Self::Week, Self::Month, Self::Year];

    /// Whether a date is recent enough, `now` being now and `midnight` today's start.
    pub fn holds(
        self,
        modified: Option<SystemTime>,
        now: SystemTime,
        midnight: SystemTime,
    ) -> bool {
        let Some(modified) = modified else {
            return false;
        };
        let since = match self {
            Self::Today => midnight,
            Self::Week => now - Duration::from_secs(7 * 86_400),
            Self::Month => now - Duration::from_secs(30 * 86_400),
            Self::Year => now - Duration::from_secs(365 * 86_400),
        };
        modified >= since
    }
}

/// How large, as the « Taille » token offers it; files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeFilter {
    Small,
    Medium,
    Large,
}

impl SizeFilter {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn holds(self, entry: &Entry) -> bool {
        const MB: u64 = 1_000_000;
        entry.kind == Kind::File
            && match self {
                Self::Small => entry.size < MB,
                Self::Medium => (MB..=100 * MB).contains(&entry.size),
                Self::Large => entry.size > 100 * MB,
            }
    }
}

/// Where the search looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    #[default]
    Folder,
    Subfolders,
}

/// The tokens chosen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub kinds: BTreeSet<KindFilter>,
    pub date: Option<DateFilter>,
    pub size: Option<SizeFilter>,
    pub scope: Scope,
}

impl Filters {
    /// Whether a token narrows what is shown.
    pub fn any(&self) -> bool {
        !self.kinds.is_empty() || self.date.is_some() || self.size.is_some()
    }

    /// Whether `entry` passes the tokens; `now` and `midnight` for the date.
    pub fn keep(&self, entry: &Entry, now: SystemTime, midnight: SystemTime) -> bool {
        (self.kinds.is_empty() || self.kinds.iter().any(|k| k.holds(entry)))
            && self
                .date
                .is_none_or(|d| d.holds(entry.modified, now, midnight))
            && self.size.is_none_or(|s| s.holds(entry))
    }
}

/// The start of today, local time, from `now`; a day before `now` when the time zone is
/// unknown.
pub fn midnight(now: SystemTime) -> SystemTime {
    crate::clock::local(now)
        .and_then(|t| {
            crate::clock::to_system(sherlock_ui::texts::LocalTime {
                hour: 0,
                minute: 0,
                second: 0,
                ..t
            })
        })
        .unwrap_or_else(|| now - Duration::from_secs(86_400))
}

/// The items under `dir`, its subfolders included, whose name holds `needle` (lower case;
/// everything when empty), within the bounds; stopped when `stop` is set.
pub fn walk(dir: &Path, needle: &str, hidden: bool, stop: &Arc<AtomicBool>) -> Vec<Entry> {
    let start = Instant::now();
    let mut found = Vec::new();
    let mut visited = 0;
    let mut pending: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((folder, depth)) = pending.pop() {
        let Ok(items) = std::fs::read_dir(&folder) else {
            continue;
        };
        for item in items.flatten() {
            visited += 1;
            if visited > MAX_VISITED
                || found.len() >= MAX_RESULTS
                || start.elapsed() > MAX_TIME
                || stop.load(Ordering::Relaxed)
            {
                return found;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            if !hidden && listing::is_hidden(&name) {
                continue;
            }
            // `file_type` does not follow links: a link to a folder is never walked into.
            let is_dir = item.file_type().is_ok_and(|t| t.is_dir());
            if is_dir && depth + 1 < MAX_DEPTH {
                pending.push((item.path(), depth + 1));
            }
            if (needle.is_empty() || name.to_lowercase().contains(needle))
                && let Ok(entry) = Entry::read(&item.path(), hidden)
            {
                found.push(entry);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_tokens_keep_what_they_name() {
        let tmp = TempDir::new("filters");
        std::fs::create_dir_all(tmp.path().join("photos")).expect("folder");
        std::fs::write(tmp.path().join("vacances.JPG"), vec![0u8; 10]).expect("write");
        std::fs::write(tmp.path().join("film.mkv"), vec![0u8; 2_000_000]).expect("write");
        std::fs::write(tmp.path().join("notes.md"), "x").expect("write");
        let read = |name: &str| Entry::read(&tmp.path().join(name), false).expect("item");
        let (photo, film, notes, folder) = (
            read("vacances.JPG"),
            read("film.mkv"),
            read("notes.md"),
            read("photos"),
        );
        assert!(KindFilter::Image.holds(&photo) && !KindFilter::Image.holds(&film));
        assert!(KindFilter::Folder.holds(&folder) && KindFilter::Document.holds(&notes));
        assert!(SizeFilter::Medium.holds(&film) && SizeFilter::Small.holds(&notes));
        assert!(
            !SizeFilter::Small.holds(&folder),
            "a folder has no size here"
        );
        let now = SystemTime::now();
        let filters = Filters {
            kinds: [KindFilter::Image, KindFilter::Video].into(),
            date: Some(DateFilter::Today),
            ..Filters::default()
        };
        let today = midnight(now);
        assert!(filters.keep(&photo, now, today) && filters.keep(&film, now, today));
        assert!(!filters.keep(&notes, now, today));
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert!(!DateFilter::Year.holds(Some(old), now, today));
        assert!(
            today <= now && now.duration_since(today).expect("after").as_secs() < 86_400 + 3600
        );
    }

    #[test]
    fn the_walk_finds_in_the_subfolders_without_following_links() {
        let tmp = TempDir::new("walk");
        std::fs::create_dir_all(tmp.path().join("a/b/c")).expect("folders");
        std::fs::write(tmp.path().join("a/b/c/rapport.pdf"), "x").expect("write");
        std::fs::write(tmp.path().join("a/rapport.txt"), "x").expect("write");
        std::os::unix::fs::symlink(tmp.path(), tmp.path().join("a/boucle")).expect("link");
        let stop = Arc::new(AtomicBool::new(false));
        let mut names: Vec<String> = walk(tmp.path(), "rapport", false, &stop)
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        assert_eq!(names, ["rapport.pdf", "rapport.txt"]);
        stop.store(true, Ordering::Relaxed);
        assert!(walk(tmp.path(), "", false, &stop).is_empty(), "stopped");
    }
}
