//! The applications the view shows: those the launcher finds (`launcher::apps`), in the order of
//! their names, filtered by the launcher's search.

use std::ffi::OsString;
use std::path::PathBuf;

use launcher::apps::Apps;
use launcher::search::{Haystack, terms};
use launcher::source::Command;
use mime_apps::{DesktopEntry, Dirs};

/// An application of the grid.
#[derive(Debug, Clone)]
pub struct Tile {
    pub entry: DesktopEntry,
    /// What starts it, without a shell.
    pub command: Command,
    /// Its names, folded for the search.
    names: Haystack,
}

impl Tile {
    pub fn name(&self) -> &str {
        &self.entry.name
    }

    /// Its desktop file id without `.desktop`, as the dock pins it.
    pub fn id(&self) -> &str {
        let id = self.entry.id.as_str();
        id.strip_suffix(".desktop").unwrap_or(id)
    }
}

/// Every application, sorted by name.
#[derive(Debug, Default)]
pub struct Catalog {
    apps: Apps,
    tiles: Vec<Tile>,
}

impl Catalog {
    /// The applications of `apps` that something starts, sorted by folded name, then by desktop
    /// id: the order never depends on the order of the files.
    pub fn new(apps: Apps) -> Self {
        let mut tiles: Vec<Tile> = apps
            .entries()
            .filter_map(|(entry, names)| {
                Some(Tile {
                    command: apps.start(entry)?,
                    entry: entry.clone(),
                    names: names.clone(),
                })
            })
            .collect();
        tiles.sort_by(|a, b| {
            a.names
                .name()
                .cmp(b.names.name())
                .then_with(|| a.entry.id.cmp(&b.entry.id))
        });
        Self { apps, tiles }
    }

    /// Reads the applications under `dirs`, as the launcher does: names in `locales`, no
    /// `NoDisplay`, `Hidden` or missing `TryExec`, a terminal for `Terminal=true`.
    pub fn load(dirs: &Dirs, locales: &[String], terminal: Option<Vec<OsString>>) -> Self {
        Self::new(Apps::load(dirs, locales, terminal))
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// The icon file of `tile`, looked up the first time it is drawn.
    pub fn icon(&self, tile: &Tile) -> Option<PathBuf> {
        self.apps.icon(&tile.entry)
    }

    /// The tiles that match `query`, by their place in `tiles`: all of them in order for an empty
    /// query; otherwise the launcher's fuzzy search, best score first, then by name.
    pub fn filter(&self, query: &str) -> Vec<usize> {
        let terms = terms(query);
        if terms.is_empty() {
            return (0..self.tiles.len()).collect();
        }
        let mut found: Vec<(u32, usize)> = self
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(index, tile)| Some((tile.names.score(&terms)?, index)))
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        found.into_iter().map(|(_, index)| index).collect()
    }
}
