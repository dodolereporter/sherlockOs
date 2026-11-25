//! Sources of results. Each one answers a query with its own results, shown under its own
//! section. D-22a has the installed applications; files, settings, the calculator and « Demander
//! à Sherlock » come as more sources in D-22b.

use std::ffi::OsString;
use std::path::PathBuf;

/// A program to start, without a shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    /// Working directory, when the source gives one.
    pub dir: Option<PathBuf>,
}

/// What choosing a result does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(Command),
}

/// One result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub title: String,
    /// The line under the title.
    pub subtitle: Option<String>,
    pub action: Action,
}

/// A source of results.
pub trait Source {
    /// Title of its section, in French, from sherlock-ui.
    fn section(&self) -> &'static str;
    /// Its results for the folded query `terms`, best first, `limit` at most. No terms, no
    /// results.
    fn search(&self, terms: &[String], limit: usize) -> Vec<Hit>;
}

/// The results of one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: &'static str,
    pub hits: Vec<Hit>,
}

/// Asks the sources in their order for `limit` results in all; a source without results has
/// no section.
pub fn search(sources: &[Box<dyn Source>], query: &str, limit: usize) -> Vec<Section> {
    let terms = crate::search::terms(query);
    let mut left = limit;
    let mut sections = Vec::new();
    for source in sources {
        if left == 0 {
            break;
        }
        let hits = source.search(&terms, left);
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
