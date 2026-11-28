//! The thumbnails of the grid and of the gallery's strip (lot E-14): asked for the items in
//! sight first, a few at a time, off the interface thread (`crate::thumbs`), and kept in
//! memory for the last ones shown.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::SystemTime;

use iced::Task;
use iced::widget::image::Handle;

use super::{App, Message};
use crate::listing::Entry;
use crate::views::ViewMode;

/// Thumbnails made at the same time at most.
pub const JOBS: usize = 3;
/// Thumbnails kept in memory at most: the oldest goes first.
pub const KEPT: usize = 600;

/// A file as it was when its thumbnail was asked: a change makes another one.
type Key = (PathBuf, Option<SystemTime>);

/// The thumbnails known: ready, being made, or that cannot be.
#[derive(Debug, Default)]
pub struct Thumbnails {
    ready: VecDeque<(Key, Handle)>,
    making: HashSet<Key>,
    none: HashSet<Key>,
}

impl Thumbnails {
    /// The thumbnail of `entry` as it is now, if ready.
    pub fn get(&self, entry: &Entry) -> Option<&Handle> {
        self.ready
            .iter()
            .find(|((p, m), _)| *p == entry.path && *m == entry.modified)
            .map(|(_, h)| h)
    }

    fn known(&self, key: &Key) -> bool {
        self.making.contains(key)
            || self.none.contains(key)
            || self.ready.iter().any(|(k, _)| k == key)
    }

    fn keep(&mut self, key: Key, handle: Option<Handle>) {
        self.making.remove(&key);
        match handle {
            Some(handle) => {
                self.ready.retain(|(k, _)| k.0 != key.0);
                self.ready.push_back((key, handle));
                while self.ready.len() > KEPT {
                    self.ready.pop_front();
                }
            }
            None => {
                self.none.insert(key);
            }
        }
    }
}

impl App {
    /// The items in sight in the views that show thumbnails, the nearest to the top first.
    fn items_in_sight(&self) -> Vec<usize> {
        match self.view_mode {
            ViewMode::Icons => {
                let columns = self.grid_columns();
                let (_, line) = self.grid_line(0);
                let first = ((self.scroll.0 / line).floor().max(0.0) as usize) * columns;
                let last =
                    (((self.scroll.0 + self.size.height) / line).ceil() as usize + 1) * columns;
                (first..last.min(self.entries.len())).collect()
            }
            ViewMode::Gallery => {
                let (first, last) = self.strip_range();
                (first..last).collect()
            }
            ViewMode::List | ViewMode::Columns => Vec::new(),
        }
    }

    /// Asks for the thumbnails of the items in sight that are not known yet, while fewer than
    /// `JOBS` are being made: what is in sight goes first, what scrolled away is not asked.
    pub(super) fn thumbnail_tasks(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        for i in self.items_in_sight() {
            if self.thumbnails.making.len() >= JOBS {
                break;
            }
            let Some(entry) = self.entries.get(i) else {
                continue;
            };
            let key = (entry.path.clone(), entry.modified);
            if crate::thumbs::source(&entry.path).is_none() || self.thumbnails.known(&key) {
                continue;
            }
            let cache = self.config.thumbnails.clone();
            self.thumbnails.making.insert(key.clone());
            tasks.push(Task::perform(
                async move {
                    let (path, modified) = &key;
                    let handle = crate::thumbs::thumbnail(path, *modified, cache.as_deref());
                    (key, handle)
                },
                |((path, modified), handle)| Message::Thumbnail(path, modified, handle),
            ));
        }
        Task::batch(tasks)
    }

    pub(super) fn thumbnail_made(
        &mut self,
        path: PathBuf,
        modified: Option<SystemTime>,
        handle: Option<Handle>,
    ) {
        self.thumbnails.keep((path, modified), handle);
    }
}
