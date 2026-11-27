//! Sources of results. Each one answers a query with its own results, shown under its own
//! section, in the order of `crate::sources`: calculator, applications, system, files, then
//! « Demander à Sherlock ».

use std::ffi::OsString;
use std::path::PathBuf;

use sherlock_ui::launcher::Glyph;

/// A program to start, without a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Working directory, when the source gives one.
    pub dir: Option<PathBuf>,
}

/// A session command, run through `crate::system::Session`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Lock,
    LogOut,
    Reboot,
    PowerOff,
}

impl Power {
    /// Everything but locking loses unsaved work: logging out, restarting and powering off
    /// ask for a second Enter.
    pub fn needs_confirmation(self) -> bool {
        !matches!(self, Self::Lock)
    }
}

/// What choosing a result does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(Command),
    /// Copies the text to the clipboard (the calculator).
    Copy(String),
    Session(Power),
    /// Shown, does nothing.
    Disabled,
}

/// What sits on the tile left of a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tile {
    /// An application: its icon file, or its initial without one.
    App(Option<PathBuf>),
    Glyph(Glyph),
    Sherlock,
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    /// The line under the title.
    pub subtitle: Option<String>,
    pub tile: Tile,
    pub action: Action,
    /// The desktop file id of an application, which its menu pins to the dock.
    pub app: Option<String>,
}

/// A query: as typed, and folded into terms (`crate::search::terms`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub raw: String,
    pub terms: Vec<String>,
}

impl Query {
    pub fn new(raw: &str) -> Self {
        Self {
            raw: raw.to_owned(),
            terms: crate::search::terms(raw),
        }
    }
}

/// A source of results. `Send`: a resident launcher rebuilds its sources off the interface's
/// thread.
pub trait Source: Send {
    /// Title of its section, in French, from sherlock-ui.
    fn section(&self) -> &'static str;
    /// Its results for `query`, best first, `limit` at most. An empty query has no results.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit>;
    /// Rows it always takes for a query that is not empty: the sources before it leave them.
    fn reserved(&self) -> usize {
        0
    }
}

/// The results of one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub hits: Vec<Hit>,
}

/// Asks the sources in their order, `per_section` results each and `limit` in all, leaving
/// the rows the later sources reserve; a source without results has no section.
pub fn search(
    sources: &[Box<dyn Source>],
    query: &str,
    per_section: usize,
    limit: usize,
) -> Vec<Section> {
    let query = Query::new(query);
    if query.terms.is_empty() {
        return Vec::new();
    }
    let mut left = limit;
    let mut sections = Vec::new();
    for (index, source) in sources.iter().enumerate() {
        let later: usize = sources.iter().skip(index + 1).map(|s| s.reserved()).sum();
        let room = left.saturating_sub(later).min(per_section);
        if room == 0 {
            continue;
        }
        let hits = source.search(&query, room);
        left = left.saturating_sub(hits.len());
        if !hits.is_empty() {
            sections.push(Section {
                title: source.section(),
                hits,
            });
        }
    }
    sections
}
