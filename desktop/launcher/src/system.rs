//! System commands: « Verrouiller », « Se déconnecter », « Redémarrer », « Éteindre ». The
//! launcher never runs them itself: it hands them to a `Session`, which `Systemd` implements
//! with `loginctl` and `systemctl`, run directly, without a shell. Tests use a fake one.

use std::ffi::OsString;

use sherlock_ui::launcher::{Glyph, text};

use crate::launch::{self, LaunchError};
use crate::search::fold;
use crate::source::{Action, Command, Hit, Power, Query, Source, Tile};

/// What runs a session command.
pub trait Session: Send + Sync {
    fn run(&self, power: Power) -> Result<(), LaunchError>;
}

/// The session through systemd: `loginctl` for the session, `systemctl` for the machine. The
/// session is `$XDG_SESSION_ID`, else the caller's (an empty id, as `loginctl` documents it).
#[derive(Debug, Clone, Default)]
pub struct Systemd {
    pub session_id: Option<String>,
}

impl Systemd {
    pub fn from_env() -> Self {
        Self {
            session_id: std::env::var("XDG_SESSION_ID")
                .ok()
                .filter(|id| !id.is_empty()),
        }
    }

    /// The command for `power`, built and not run.
    pub fn command(&self, power: Power) -> Command {
        let session = || OsString::from(self.session_id.clone().unwrap_or_default());
        let (program, args) = match power {
            Power::Lock => ("loginctl", vec!["lock-session".into(), session()]),
            Power::LogOut => ("loginctl", vec!["terminate-session".into(), session()]),
            Power::Reboot => ("systemctl", vec!["reboot".into()]),
            Power::PowerOff => ("systemctl", vec!["poweroff".into()]),
        };
        Command {
            program: program.into(),
            args,
            dir: None,
        }
    }
}

impl Session for Systemd {
    fn run(&self, power: Power) -> Result<(), LaunchError> {
        launch::spawn(&self.command(power)).map(|_| ())
    }
}

/// No session: every command is refused. The launcher's default until one is given.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoSession;

impl Session for NoSession {
    fn run(&self, power: Power) -> Result<(), LaunchError> {
        Err(LaunchError::Spawn {
            program: format!("{power:?}"),
            source: std::io::Error::other("no session to run it"),
        })
    }
}

/// Where the system section goes: before the files when a command is found by its name,
/// after them when it is only found by a keyword (`crate::arrange` holds one of each).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    ByName,
    ByKeyword,
}

/// The four commands, a source. A command is offered only when the query starts its name as
/// shown (« se d », « étein ») or is one of its keywords in full (`logout`): a query that only
/// looks like one (« logo », « déco ») offers nothing.
#[derive(Debug, Clone)]
pub struct System {
    placement: Placement,
    /// Each command, its folded name and its keywords.
    commands: Vec<(Power, String, &'static [&'static str])>,
}

impl System {
    pub fn new(placement: Placement) -> Self {
        let command = |power| {
            let (title, _, _) = describe(power);
            let words: &'static [&'static str] = match power {
                Power::Lock => &text::LOCK_WORDS,
                Power::LogOut => &text::LOG_OUT_WORDS,
                Power::Reboot => &text::REBOOT_WORDS,
                Power::PowerOff => &text::POWER_OFF_WORDS,
            };
            (power, fold(title), words)
        };
        Self {
            placement,
            commands: [Power::Lock, Power::LogOut, Power::Reboot, Power::PowerOff]
                .into_iter()
                .map(command)
                .collect(),
        }
    }
}

impl Default for System {
    fn default() -> Self {
        Self::new(Placement::ByName)
    }
}

/// Title, line and glyph of a command.
pub fn describe(power: Power) -> (&'static str, &'static str, Glyph) {
    match power {
        Power::Lock => (text::LOCK, text::LOCK_LINE, Glyph::Lock),
        Power::LogOut => (text::LOG_OUT, text::LOG_OUT_LINE, Glyph::LogOut),
        Power::Reboot => (text::REBOOT, text::REBOOT_LINE, Glyph::Reboot),
        Power::PowerOff => (text::POWER_OFF, text::POWER_OFF_LINE, Glyph::PowerOff),
    }
}

impl Source for System {
    fn section(&self) -> &'static str {
        text::SYSTEM
    }

    /// The commands found by their name first, then those found by a keyword, each in the
    /// order written. The whole list goes to the placement it calls for, the other one has
    /// none.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        let wanted = query.terms.join(" ");
        if wanted.is_empty() {
            return Vec::new();
        }
        let mut found: Vec<(Placement, Power)> = self
            .commands
            .iter()
            .filter_map(|(power, name, words)| {
                if name.starts_with(&wanted) {
                    Some((Placement::ByName, *power))
                } else {
                    words
                        .contains(&wanted.as_str())
                        .then_some((Placement::ByKeyword, *power))
                }
            })
            .collect();
        found.sort_by_key(|(placement, _)| *placement != Placement::ByName);
        let placement = found.first().map(|(p, _)| *p);
        if placement != Some(self.placement) {
            return Vec::new();
        }
        found
            .into_iter()
            .take(limit)
            .map(|(_, power)| {
                let (title, line, glyph) = describe(power);
                Hit {
                    title: title.to_owned(),
                    subtitle: Some(line.to_owned()),
                    tile: Tile::Glyph(glyph),
                    app: None,
                    action: Action::Session(power),
                }
            })
            .collect()
    }
}
