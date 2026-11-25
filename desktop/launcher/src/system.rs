//! System commands: « Verrouiller », « Se déconnecter », « Redémarrer », « Éteindre ». The
//! launcher never runs them itself: it hands them to a `Session`, which `Systemd` implements
//! with `loginctl` and `systemctl`, run directly, without a shell. Tests use a fake one.

use std::ffi::OsString;

use sherlock_ui::launcher::{Glyph, text};

use crate::launch::{self, LaunchError};
use crate::search::Haystack;
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

/// The four commands, a source.
#[derive(Debug, Clone)]
pub struct System {
    commands: Vec<(Power, Haystack)>,
}

impl Default for System {
    fn default() -> Self {
        let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
        let command =
            |power, title, list: &[&str]| (power, Haystack::new(title, None, &words(list)));
        Self {
            commands: vec![
                command(Power::Lock, text::LOCK, &text::LOCK_WORDS),
                command(Power::LogOut, text::LOG_OUT, &text::LOG_OUT_WORDS),
                command(Power::Reboot, text::REBOOT, &text::REBOOT_WORDS),
                command(Power::PowerOff, text::POWER_OFF, &text::POWER_OFF_WORDS),
            ],
        }
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

    /// Best score first, then in the order written. Scattered letters do not count: « et »
    /// must not offer to power off.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        let mut found: Vec<(u32, usize, Power)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(order, (power, words))| {
                let score = words.score_from(&query.terms, crate::search::WORD)?;
                Some((score, order, *power))
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        found
            .into_iter()
            .take(limit)
            .map(|(_, _, power)| {
                let (title, line, glyph) = describe(power);
                Hit {
                    title: title.to_owned(),
                    subtitle: Some(line.to_owned()),
                    tile: Tile::Glyph(glyph),
                    action: Action::Session(power),
                }
            })
            .collect()
    }
}
