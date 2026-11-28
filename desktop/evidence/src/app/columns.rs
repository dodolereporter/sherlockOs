//! The columns view (lot E-13b), as in the Finder: the folders above the one shown, each in a
//! column with the way down lit, then the folder shown, then what its selected item holds:
//! a folder's content, or a file's preview. Every column but the folder shown is read off the
//! interface thread, bounded, and only for the columns that fit.

use std::path::{Path, PathBuf};

use iced::Task;
use sherlock_ui::evidence as more;

use super::{App, Location, Message};
use crate::listing::{self, Entry, Kind};
use crate::views::ViewMode;

/// Items read at most in a column other than the folder shown.
pub const MAX_COLUMN_ITEMS: usize = 500;

/// A folder of the columns, and its items, sorted as the folder shown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DirColumn {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
}

/// Reads `dir` for a column: bounded, an unreadable folder is an empty column.
fn read_column(dir: &Path, hidden: bool, sort: listing::Sort) -> DirColumn {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|item| hidden || !listing::is_hidden(&item.file_name().to_string_lossy()))
        .take(MAX_COLUMN_ITEMS)
        .filter_map(|item| Entry::read(&item.path(), hidden).ok())
        .collect();
    listing::sort(&mut entries, sort);
    DirColumn {
        dir: dir.to_path_buf(),
        entries,
    }
}

impl App {
    /// How many folders above the one shown have room, beside it and the column on its right.
    pub fn ancestor_room(&self) -> usize {
        let room = self.content_width() / more::COLUMN_WIDTH;
        (room.floor() as usize).saturating_sub(2)
    }

    /// The folders above the one shown that the columns show, from the highest.
    fn ancestors(&self) -> Vec<PathBuf> {
        let Location::Folder(dir) = &self.location else {
            return Vec::new();
        };
        let mut above: Vec<PathBuf> = dir
            .ancestors()
            .skip(1)
            .take(self.ancestor_room())
            .map(Path::to_path_buf)
            .collect();
        above.reverse();
        above
    }

    /// Reads the folders above the one shown, when the columns show them.
    pub(super) fn load_columns(&mut self) -> Task<Message> {
        if self.view_mode != ViewMode::Columns {
            self.column_dirs.clear();
            return Task::none();
        }
        let dirs = self.ancestors();
        if self.column_dirs.iter().map(|c| &c.dir).eq(dirs.iter()) {
            return Task::none();
        }
        let (hidden, sort, generation) = (self.show_hidden, self.sort, self.generation);
        Task::perform(
            async move {
                dirs.iter()
                    .map(|dir| read_column(dir, hidden, sort))
                    .collect()
            },
            move |columns| Message::Columns(generation, columns),
        )
    }

    /// The content of the folder selected alone, in the column on the right, read when the
    /// selection changes.
    pub(super) fn peek_task(&mut self) -> Task<Message> {
        let wanted = (self.view_mode == ViewMode::Columns)
            .then(|| self.single())
            .flatten()
            .filter(|e| e.kind == Kind::Folder)
            .map(|e| e.path.clone());
        if wanted.as_ref() == self.peek.as_ref().map(|p| &p.dir) {
            return Task::none();
        }
        let Some(dir) = wanted else {
            self.peek = None;
            return Task::none();
        };
        self.peek = Some(DirColumn {
            dir: dir.clone(),
            entries: Vec::new(),
        });
        let (hidden, sort) = (self.show_hidden, self.sort);
        Task::perform(async move { read_column(&dir, hidden, sort) }, |column| {
            Message::Peeked(column)
        })
    }

    pub(super) fn columns_read(&mut self, generation: u64, columns: Vec<DirColumn>) {
        if generation == self.generation {
            self.column_dirs = columns;
        }
    }

    pub(super) fn peeked(&mut self, column: DirColumn) {
        if self.peek.as_ref().is_some_and(|p| p.dir == column.dir) {
            self.peek = Some(column);
        }
    }

    /// A click in a column above the folder shown: a folder opens, a file shows selected in
    /// its own folder.
    pub(super) fn column_click(&mut self, depth: usize, index: usize) -> Task<Message> {
        let Some(column) = self.column_dirs.get(depth) else {
            return Task::none();
        };
        let Some(entry) = column.entries.get(index) else {
            return Task::none();
        };
        match entry.kind {
            Kind::Folder => {
                let dir = entry.path.clone();
                self.go(Location::Folder(dir))
            }
            _ => {
                let (dir, path) = (column.dir.clone(), entry.path.clone());
                self.go_select(Location::Folder(dir), vec![path])
            }
        }
    }

    /// A click in the column on the right: a folder opens, as a file opens.
    pub(super) fn peek_click(&mut self, index: usize) -> Task<Message> {
        let Some(entry) = self.peek.as_ref().and_then(|p| p.entries.get(index)) else {
            return Task::none();
        };
        let (dir, path, folder) = (
            entry.path.parent().map(Path::to_path_buf),
            entry.path.clone(),
            entry.kind == Kind::Folder,
        );
        match (folder, dir) {
            (true, _) => self.go(Location::Folder(path)),
            (false, Some(dir)) => self.go_select(Location::Folder(dir), vec![path]),
            (false, None) => Task::none(),
        }
    }

    /// Left of the column of the folder shown, in the listing.
    pub fn shown_column_left(&self) -> f32 {
        self.column_dirs.len() as f32 * more::COLUMN_WIDTH
    }
}
