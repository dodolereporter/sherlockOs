//! Workspaces and active application.
//!
//! The compositor publishes neither its workspaces (`ext-workspace-v1`) nor its windows
//! (`ext-foreign-toplevel-list-v1`) yet. Until then, `Placeholder` is a fake source, set at
//! startup by two environment variables. What the compositor will have to provide is described
//! in `docs/bar.md`.

use super::{Source, SourceError};

/// Names of the placeholder workspaces, comma separated. An empty name keeps the number only.
pub const NAMES_VAR: &str = "BAR_WORKSPACES";
/// Name of the placeholder active application.
pub const APP_VAR: &str = "BAR_APP";
/// Number of placeholder workspaces without `BAR_WORKSPACES`.
const DEFAULT_COUNT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspaces {
    /// Name of each workspace, in the order of their numbers.
    pub names: Vec<Option<String>>,
    /// Index of the active workspace in `names`.
    pub active: usize,
    /// Name of the focused application.
    pub app: Option<String>,
}

impl Workspaces {
    /// The placeholder workspaces: those of `names` (`BAR_WORKSPACES`), the first one active.
    pub fn placeholder(names: Option<&str>, app: Option<&str>) -> Self {
        let names = match names {
            Some(list) => list
                .split(',')
                .map(|n| Some(n.trim().to_owned()).filter(|n| !n.is_empty()))
                .collect(),
            None => vec![None; DEFAULT_COUNT],
        };
        let app = app
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned);
        Self {
            names,
            active: 0,
            app,
        }
    }
}

/// Fake source: reads the variables once, never changes.
#[derive(Debug, Default)]
pub struct Placeholder;

impl Source for Placeholder {
    type State = Workspaces;

    fn read(&mut self) -> Result<Workspaces, SourceError> {
        let names = std::env::var(NAMES_VAR).ok();
        let app = std::env::var(APP_VAR).ok();
        Ok(Workspaces::placeholder(names.as_deref(), app.as_deref()))
    }

    fn wait(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_variable_three_numbered_workspaces() {
        let w = Workspaces::placeholder(None, None);
        assert_eq!(w.names, [None, None, None]);
        assert_eq!((w.active, w.app), (0, None));
    }

    #[test]
    fn names_come_from_the_variable() {
        let w = Workspaces::placeholder(Some("Code, ,Jeu"), Some(" Éditeur "));
        assert_eq!(
            w.names,
            [Some(String::from("Code")), None, Some(String::from("Jeu"))]
        );
        assert_eq!(w.app.as_deref(), Some("Éditeur"));
    }
}
