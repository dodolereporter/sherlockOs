//! Workspaces, active application, and the screens a maximized window covers.
//!
//! The compositor publishes them on its shell socket (`source::shell`, ADR 0021). Elsewhere
//! (Plasma, another compositor), or when `BAR_WORKSPACES` or `BAR_APP` is set, `Placeholder`
//! is a fake source read once from those two variables.

use std::collections::BTreeSet;

use super::{Source, SourceError};

/// Names of the placeholder workspaces, comma separated. An empty name keeps the number only.
pub const NAMES_VAR: &str = "BAR_WORKSPACES";
/// Name of the placeholder active application.
pub const APP_VAR: &str = "BAR_APP";
/// Number of placeholder workspaces without `BAR_WORKSPACES`.
const DEFAULT_COUNT: usize = 3;

/// What the bar shows of the desktop.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Desktop {
    /// The workspace groups, each with the screens it holds.
    pub groups: Vec<Group>,
    /// Title of the active window, or its `app_id` when it has none.
    pub app: Option<String>,
    /// Names of the screens a maximized or fullscreen window is on.
    pub covered: BTreeSet<String>,
    /// The compositor can switch workspaces: not the placeholder.
    pub live: bool,
}

/// Workspaces shown together, on the screens of `outputs`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Group {
    pub outputs: Vec<String>,
    /// In the order of their numbers.
    pub workspaces: Vec<Workspace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// The `ext-workspace-v1` id, which a click names it by.
    pub id: String,
    /// Shown after the number, unless it is the number itself.
    pub name: Option<String>,
    pub active: bool,
    /// Its windows, minimized ones included. `None` when unknown.
    pub windows: Option<usize>,
}

impl Workspace {
    /// Holds windows, or may: the placeholder does not know.
    pub fn occupied(&self) -> bool {
        self.windows != Some(0)
    }
}

impl Desktop {
    /// The placeholder: the workspaces of `names` (`BAR_WORKSPACES`), the first one active.
    pub fn placeholder(names: Option<&str>, app: Option<&str>) -> Self {
        let names: Vec<_> = match names {
            Some(list) => list.split(',').map(|n| n.trim().to_owned()).collect(),
            None => vec![String::new(); DEFAULT_COUNT],
        };
        let workspaces = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| Workspace {
                id: (i + 1).to_string(),
                name: Some(name).filter(|n| !n.is_empty()),
                active: i == 0,
                windows: None,
            })
            .collect();
        let app = app.map(str::trim).filter(|a| !a.is_empty());
        Self {
            groups: vec![Group {
                outputs: Vec::new(),
                workspaces,
            }],
            app: app.map(str::to_owned),
            covered: BTreeSet::new(),
            live: false,
        }
    }

    /// The group of the screen `output`: the one holding it. A bar that does not know its
    /// screen yet takes the first group; a screen no group holds has none.
    pub fn group(&self, output: Option<&str>) -> Option<&Group> {
        match output {
            Some(output) => self
                .groups
                .iter()
                .find(|g| g.outputs.iter().any(|n| n == output)),
            None => self.groups.first(),
        }
    }

    /// Whether a maximized or fullscreen window is on the screen `output`. A bar that does not
    /// know its screen yet takes the only one there is.
    pub fn covers(&self, output: Option<&str>) -> bool {
        match output {
            Some(output) => self.covered.contains(output),
            None => {
                let screens: BTreeSet<_> = self.groups.iter().flat_map(|g| &g.outputs).collect();
                self.covered.len() == 1 && screens.len() <= 1
            }
        }
    }
}

/// Fake source: reads the variables once, never changes.
#[derive(Debug, Default)]
pub struct Placeholder;

impl Placeholder {
    /// Whether one of the variables is set: the bar then shows them, even on our compositor.
    pub fn asked() -> bool {
        std::env::var_os(NAMES_VAR).is_some() || std::env::var_os(APP_VAR).is_some()
    }
}

impl Source for Placeholder {
    type State = Desktop;

    fn read(&mut self) -> Result<Desktop, SourceError> {
        let names = std::env::var(NAMES_VAR).ok();
        let app = std::env::var(APP_VAR).ok();
        Ok(Desktop::placeholder(names.as_deref(), app.as_deref()))
    }

    fn wait(&mut self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(desktop: &Desktop) -> Vec<Option<&str>> {
        let group = desktop.group(None).map_or(&[][..], |g| &g.workspaces);
        group.iter().map(|w| w.name.as_deref()).collect()
    }

    #[test]
    fn without_variable_three_numbered_workspaces() {
        let d = Desktop::placeholder(None, None);
        assert_eq!(names(&d), [None, None, None]);
        assert_eq!(d.app, None);
        assert!(!d.live && !d.covers(None));
        let first = &d.groups[0].workspaces[0];
        assert!(
            first.active && first.occupied(),
            "the board's look: all veiled"
        );
    }

    #[test]
    fn names_come_from_the_variable() {
        let d = Desktop::placeholder(Some("Code, ,Jeu"), Some(" Éditeur "));
        assert_eq!(names(&d), [Some("Code"), None, Some("Jeu")]);
        assert_eq!(d.app.as_deref(), Some("Éditeur"));
        assert_eq!(d.groups[0].workspaces[2].id, "3");
    }

    #[test]
    fn each_screen_has_its_group_and_its_cover() {
        let group = |output: &str, id: &str| Group {
            outputs: vec![output.to_owned()],
            workspaces: vec![Workspace {
                id: id.to_owned(),
                name: None,
                active: true,
                windows: Some(1),
            }],
        };
        let d = Desktop {
            groups: vec![group("DP-1", "a"), group("eDP-1", "b")],
            covered: BTreeSet::from([String::from("eDP-1")]),
            ..Desktop::default()
        };
        let id = |output| d.group(output).map(|g| g.workspaces[0].id.as_str());
        assert_eq!(id(Some("eDP-1")), Some("b"));
        assert_eq!(id(Some("HDMI-A-1")), None, "no group holds that screen");
        assert_eq!(id(None), Some("a"), "screen not known yet: the first group");
        assert!(d.covers(Some("eDP-1")) && !d.covers(Some("DP-1")));
        assert!(
            !d.covers(None),
            "two screens: an unknown one is not guessed"
        );
    }
}
