//! Files and folders of the personal folder whose name matches the query. The folder is walked
//! once, in the background, when the launcher opens: breadth first, 4 levels and 20 000
//! entries at most, without hidden entries nor the known heavy folders, never following a
//! symbolic link nor entering another file system. Enter opens a result with its default application, from `mime-apps`. A real
//! index comes later.

use std::collections::VecDeque;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

use iced::futures::channel::oneshot;
use mime_apps::Catalog;
use sherlock_ui::launcher::{Glyph, text};

use crate::search::{SUBSTRING, fold, score};
use crate::source::{Action, Command, Hit, Query, Source, Tile};

/// Levels walked under the personal folder: its own entries are level 1.
pub const MAX_DEPTH: usize = 4;
/// Entries kept at most.
pub const MAX_ENTRIES: usize = 20_000;
/// Folders never entered: build outputs, dependencies, caches. Hidden ones (`.git`,
/// `.cache`…) are left out anyway.
pub const SKIPPED: [&str; 5] = [
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    "site-packages",
];

/// A file or folder found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: PathBuf,
    pub is_dir: bool,
    /// Its level under the personal folder, from 1.
    pub depth: usize,
    /// Its name, folded.
    name: String,
}

/// Walks `home`, breadth first: when the limit is met, the shallow entries are in. The walk
/// stays on the device of `home`: another file system mounted inside (a USB disk, a network
/// share that may hang) is listed, never entered.
pub fn walk(home: &Path) -> Vec<Entry> {
    match std::fs::metadata(home) {
        Ok(meta) => walk_on(home, meta.dev()),
        Err(_) => Vec::new(),
    }
}

/// Walks `home`, entering only the folders of device `device`.
pub fn walk_on(home: &Path, device: u64) -> Vec<Entry> {
    let mut found = Vec::new();
    let mut pending = VecDeque::from([(home.to_path_buf(), 1)]);
    while let Some((dir, depth)) = pending.pop_front() {
        let Ok(reader) = std::fs::read_dir(&dir) else {
            continue;
        };
        // Read no more than the room left: a folder of a million entries stops early.
        let room = MAX_ENTRIES - found.len();
        let mut children = Vec::new();
        for child in reader.flatten() {
            if children.len() == room {
                break;
            }
            let name = child.file_name().to_string_lossy().into_owned();
            // `file_type` does not follow a symbolic link: a link is listed, never entered.
            let is_dir = child.file_type().is_ok_and(|t| t.is_dir());
            if name.starts_with('.') || (is_dir && SKIPPED.contains(&name.as_str())) {
                continue;
            }
            children.push((name, child, is_dir));
        }
        // Readable in the same order on every run.
        children.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, child, is_dir) in children {
            let path = child.path();
            let enter =
                is_dir && depth < MAX_DEPTH && child.metadata().is_ok_and(|m| m.dev() == device);
            if enter {
                pending.push_back((path.clone(), depth + 1));
            }
            found.push(Entry {
                name: fold(&name),
                path,
                is_dir,
                depth,
            });
        }
        if found.len() == MAX_ENTRIES {
            break;
        }
    }
    found
}

/// What the walk gives: the entries and the applications that open them.
#[derive(Debug, Default)]
struct Index {
    entries: Vec<Entry>,
    catalog: Catalog,
}

/// The files of the personal folder, a source; empty until the walk is done.
#[derive(Debug, Clone, Default)]
pub struct Files {
    home: PathBuf,
    index: Arc<OnceLock<Index>>,
}

impl Files {
    /// Walks `home` and reads the catalog on a thread of its own; the receiver gets a value
    /// once the source can answer.
    pub fn start(
        home: PathBuf,
        catalog: impl FnOnce() -> Catalog + Send + 'static,
    ) -> (Self, oneshot::Receiver<usize>) {
        let files = Self {
            home,
            index: Arc::default(),
        };
        let (ready, receiver) = oneshot::channel();
        let (home, index) = (files.home.clone(), Arc::clone(&files.index));
        let walker = std::thread::Builder::new()
            .name(String::from("files"))
            .spawn(move || {
                let started = std::time::Instant::now();
                let entries = walk(&home);
                let count = entries.len();
                let _ = index.set(Index {
                    entries,
                    catalog: catalog(),
                });
                tracing::info!(
                    entries = count,
                    ms = started.elapsed().as_millis(),
                    "files read"
                );
                let _ = ready.send(count);
            });
        if let Err(error) = walker {
            tracing::warn!(%error, "cannot walk the personal folder");
        }
        (files, receiver)
    }

    /// The source on a walk already done (tests).
    pub fn ready(home: PathBuf, catalog: Catalog) -> Self {
        let entries = walk(&home);
        let index = OnceLock::new();
        let _ = index.set(Index { entries, catalog });
        Self {
            home,
            index: Arc::new(index),
        }
    }

    /// The entries found, once the walk is done.
    pub fn entries(&self) -> Option<&[Entry]> {
        self.index.get().map(|i| i.entries.as_slice())
    }

    /// The line under a result: its folder under the personal one, « Projets, compositeur ».
    fn place(&self, entry: &Entry) -> String {
        let parent = entry.path.parent().unwrap_or(&entry.path);
        let parts: Vec<String> = parent
            .strip_prefix(&self.home)
            .unwrap_or(parent)
            .components()
            .filter_map(|c| match c {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        if parts.is_empty() {
            text::HOME.to_owned()
        } else {
            parts.join(", ")
        }
    }
}

/// The command opening `entry` with its default application, when there is one.
fn open(catalog: &Catalog, entry: &Entry) -> Option<Command> {
    let mime = catalog.mime.type_of(&entry.path, entry.is_dir);
    let app = catalog.default_for(&mime)?;
    let (program, args) = catalog.command(app, std::slice::from_ref(&entry.path))?;
    Some(Command {
        program,
        args,
        dir: app.working_dir.clone(),
    })
}

impl Source for Files {
    fn section(&self) -> &'static str {
        text::FILES
    }

    /// Every term inside the name (no scattered letters). Best score first, then the
    /// shallowest, then by name and path.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        let Some(index) = self.index.get() else {
            return Vec::new();
        };
        if query.terms.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(u32, &Entry)> = index
            .entries
            .iter()
            .filter_map(|entry| {
                let total = query.terms.iter().try_fold(0u32, |total, term| {
                    score(term, &entry.name)
                        .filter(|s| *s >= SUBSTRING)
                        .map(|s| total + s)
                })?;
                Some((total, entry))
            })
            .collect();
        found.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then(a.1.depth.cmp(&b.1.depth))
                .then_with(|| a.1.name.cmp(&b.1.name))
                .then_with(|| a.1.path.cmp(&b.1.path))
        });
        found
            .into_iter()
            .take(limit)
            .map(|(_, entry)| {
                let action = open(&index.catalog, entry).map_or(Action::Disabled, Action::Start);
                let glyph = if entry.is_dir {
                    Glyph::Folder
                } else {
                    Glyph::File
                };
                Hit {
                    title: entry
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    subtitle: Some(self.place(entry)),
                    tile: Tile::Glyph(glyph),
                    action,
                }
            })
            .collect()
    }
}
