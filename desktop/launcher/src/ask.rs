//! « Demander à Sherlock »: always the last entry when the query is not empty. sherlockd has no
//! interface for the shell yet, nor an option that takes a request from it (it knows `--demo`,
//! `--claude-code`, `--proxy` and `--test-server`): the entry stays visible, disabled, with
//! « Bientôt ». Once `ASK_OPTION` names such an option, the entry runs
//! `sherlockd <option> <query>` in a terminal. See `docs/launcher.md` for what the interface
//! should offer.

use std::ffi::OsString;

use sherlock_ui::launcher::text;

use crate::source::{Action, Command, Hit, Query, Source, Tile};

/// The option of sherlockd that takes a request from the shell: none yet.
pub const ASK_OPTION: Option<&str> = None;
/// The daemon's program.
pub const SHERLOCKD: &str = "sherlockd";

#[derive(Debug, Clone, Default)]
pub struct Ask {
    /// The terminal and its option, `sherlockd`, then its option; `None`: disabled.
    prefix: Option<Vec<OsString>>,
}

impl Ask {
    /// Enabled only with a terminal and an option of sherlockd.
    pub fn new(terminal: Option<Vec<OsString>>, option: Option<&str>) -> Self {
        let prefix = terminal.zip(option).map(|(mut terminal, option)| {
            terminal.push(SHERLOCKD.into());
            terminal.push(option.into());
            terminal
        });
        Self { prefix }
    }

    pub fn is_enabled(&self) -> bool {
        self.prefix.is_some()
    }
}

impl Source for Ask {
    fn section(&self) -> &'static str {
        text::SHERLOCK
    }

    /// One entry, the query as typed handed over in a single argument.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        if query.terms.is_empty() || limit == 0 {
            return Vec::new();
        }
        let (subtitle, action) = match &self.prefix {
            Some(prefix) => {
                let mut words = prefix.iter().cloned();
                let program = words.next().unwrap_or_default();
                let mut args: Vec<OsString> = words.collect();
                args.push(query.raw.trim().into());
                let command = Command {
                    program,
                    args,
                    dir: None,
                };
                (text::ASK_LINE, Action::Start(command))
            }
            None => (text::SOON, Action::Disabled),
        };
        vec![Hit {
            title: text::ASK.to_owned(),
            subtitle: Some(subtitle.to_owned()),
            tile: Tile::Sherlock,
            action,
        }]
    }

    fn reserved(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_while_sherlockd_has_no_option() {
        let ask = Ask::new(Some(vec!["kitty".into(), "-e".into()]), ASK_OPTION);
        assert!(!ask.is_enabled());
        let hits = ask.search(&Query::new("range mes photos"), 1);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, text::ASK);
        assert_eq!(hits[0].subtitle.as_deref(), Some(text::SOON));
        assert_eq!(hits[0].action, Action::Disabled);
        assert!(ask.search(&Query::new("  "), 1).is_empty());
        assert!(ask.search(&Query::new("x"), 0).is_empty());
    }

    #[test]
    fn enabled_it_hands_the_query_over_in_one_argument() {
        let ask = Ask::new(Some(vec!["kitty".into(), "-e".into()]), Some("--ask"));
        let hits = ask.search(&Query::new(" range « mes » photos; rm -rf ~ "), 1);
        let Action::Start(command) = &hits[0].action else {
            panic!("a command");
        };
        assert_eq!(command.program, "kitty");
        let args: Vec<&OsString> = command.args.iter().collect();
        assert_eq!(
            args,
            ["-e", "sherlockd", "--ask", "range « mes » photos; rm -rf ~"]
        );
        // No terminal: disabled.
        assert!(!Ask::new(None, Some("--ask")).is_enabled());
    }
}
