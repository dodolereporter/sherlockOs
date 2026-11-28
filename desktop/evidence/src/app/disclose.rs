//! Folders unfolded in the list (lot E-13d), as with the Finder's triangles: a folder's items
//! show under it, a step to the right, read off the interface thread, bounded. Folded again,
//! its unfolded subfolders fold with it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use iced::Task;

use super::{App, Message};
use crate::listing::{self, Entry, Kind};
use crate::views::ViewMode;

/// Items read at most under an unfolded folder.
pub const MAX_UNFOLDED: usize = 2000;
/// Depth at most: a folder deeper does not unfold.
pub const MAX_DEPTH: u8 = 16;

/// The unfolded folders, with their depth, and their items, read or not yet.
pub type Unfolded = BTreeMap<PathBuf, (u8, Option<Vec<Entry>>)>;

/// Reads the items of `dir`, one step deeper than `depth`, sorted as the list.
fn read_children(dir: &Path, depth: u8, hidden: bool, sort: listing::Sort) -> Vec<Entry> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|item| hidden || !listing::is_hidden(&item.file_name().to_string_lossy()))
        .take(MAX_UNFOLDED)
        .filter_map(|item| Entry::read(&item.path(), hidden).ok())
        .map(|mut entry| {
            entry.depth = depth + 1;
            entry
        })
        .collect();
    listing::sort(&mut entries, sort);
    entries
}

/// `entries` with the items of the unfolded folders under each, recursively.
pub fn flatten(entries: Vec<Entry>, unfolded: &Unfolded) -> Vec<Entry> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let children = unfolded.get(&entry.path).and_then(|(_, c)| c.clone());
        out.push(entry);
        if let Some(children) = children {
            out.extend(flatten(children, unfolded));
        }
    }
    out
}

impl App {
    /// Whether folder `path` is unfolded.
    pub fn is_unfolded(&self, path: &Path) -> bool {
        self.unfolded.contains_key(path)
    }

    /// Unfolds or folds the folder at `path`.
    pub(super) fn disclose(&mut self, path: PathBuf, open: bool) -> Task<Message> {
        if self.view_mode != ViewMode::List {
            return Task::none();
        }
        let Some(entry) = self.entries.iter().find(|e| e.path == path) else {
            return Task::none();
        };
        if entry.kind != Kind::Folder || open == self.is_unfolded(&path) {
            return Task::none();
        }
        if !open {
            // Its unfolded subfolders fold with it.
            self.unfolded.retain(|dir, _| !dir.starts_with(&path));
            self.refilter();
            return Task::none();
        }
        if entry.depth >= MAX_DEPTH {
            return Task::none();
        }
        let depth = entry.depth;
        self.unfolded.insert(path.clone(), (depth, None));
        self.read_unfolded(path, depth)
    }

    fn read_unfolded(&self, dir: PathBuf, depth: u8) -> Task<Message> {
        let (hidden, sort) = (self.show_hidden, self.sort);
        Task::perform(
            async move {
                let children = read_children(&dir, depth, hidden, sort);
                (dir, children)
            },
            |(dir, children)| Message::Unfolded(dir, children),
        )
    }

    pub(super) fn unfolded_read(&mut self, dir: PathBuf, children: Vec<Entry>) {
        if let Some((_, slot)) = self.unfolded.get_mut(&dir) {
            *slot = Some(children);
            self.refilter();
        }
    }

    /// Reads the unfolded folders again, with the folder shown.
    pub(super) fn reread_unfolded(&self) -> Task<Message> {
        Task::batch(
            self.unfolded
                .iter()
                .map(|(dir, (depth, _))| self.read_unfolded(dir.clone(), *depth)),
        )
    }

    /// Right unfolds the folder selected alone, Left folds it, or goes to its parent row.
    pub(super) fn disclose_key(&mut self, open: bool) -> Task<Message> {
        let Some(entry) = self.single().cloned() else {
            return Task::none();
        };
        if !open && !self.is_unfolded(&entry.path) && entry.depth > 0 {
            let parent = entry.path.parent().map(Path::to_path_buf);
            if let Some(i) = parent.and_then(|p| self.entries.iter().position(|e| e.path == p)) {
                return self.move_cursor(i, false);
            }
        }
        self.disclose(entry.path, open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfolded_items_follow_their_folder() {
        let tmp = crate::testing::TempDir::new("unfold");
        std::fs::create_dir_all(tmp.path().join("a/b")).expect("folders");
        std::fs::write(tmp.path().join("a/z.txt"), "z").expect("write");
        std::fs::write(tmp.path().join("c.txt"), "c").expect("write");
        let sort = listing::Sort::default();
        let top = read_children(tmp.path(), 0, false, sort);
        let top: Vec<Entry> = top
            .into_iter()
            .map(|mut e| {
                e.depth = 0;
                e
            })
            .collect();
        let mut unfolded = Unfolded::new();
        unfolded.insert(
            tmp.path().join("a"),
            (
                0,
                Some(read_children(&tmp.path().join("a"), 0, false, sort)),
            ),
        );
        let flat = flatten(top, &unfolded);
        let shown: Vec<(&str, u8)> = flat.iter().map(|e| (e.name.as_str(), e.depth)).collect();
        assert_eq!(shown, [("a", 0), ("b", 1), ("z.txt", 1), ("c.txt", 0)]);
    }
}
