//! « Demander à Sherlock »: always the last entry when the query is not empty. sherlockd has no
//! interface for the shell yet, nor an option that takes a request from it (it knows `--demo`,
//! `--claude-code`, `--proxy` and `--test-server`): the entry stays visible, disabled, with
//! « Bientôt ». Once `ASK_OPTION` names such an option, the entry runs
//! `sherlockd <option> <file>` in a terminal, the query written in `<file>`, readable by the user
//! alone: never in the arguments, which every local account reads (`ps`, `/proc/*/cmdline`).
//! sherlockd reads that file and removes it. See `docs/launcher.md` for what the interface
//! should offer.

use std::ffi::OsString;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

    /// One entry, the query as typed handed over in a private file when it is chosen.
    fn search(&self, query: &Query, limit: usize) -> Vec<Hit> {
        if query.terms.is_empty() || limit == 0 {
            return Vec::new();
        }
        let (subtitle, action) = match &self.prefix {
            Some(prefix) => {
                let mut words = prefix.iter().cloned();
                let program = words.next().unwrap_or_default();
                let command = Command {
                    program,
                    args: words.collect(),
                    dir: None,
                };
                let request = query.raw.trim().to_owned();
                (text::ASK_LINE, Action::Ask { command, request })
            }
            None => (text::SOON, Action::Disabled),
        };
        vec![Hit {
            title: text::ASK.to_owned(),
            subtitle: Some(subtitle.to_owned()),
            tile: Tile::Sherlock,
            app: None,
            action,
        }]
    }

    fn reserved(&self) -> usize {
        1
    }
}

/// Requests written by this process, for names of their own.
static REQUESTS: AtomicU64 = AtomicU64::new(0);

/// Writes `request` into a new file of `<runtime>/sherlockd/requests`, made 0600 from the start
/// (the folders 0700), and returns its path. The file is created, never opened if it was there.
pub fn request_file(runtime: &Path, request: &str) -> io::Result<PathBuf> {
    let dir = runtime.join("sherlockd").join("requests");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let count = REQUESTS.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("{}-{count}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?;
    file.write_all(request.as_bytes())?;
    Ok(path)
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
    fn enabled_it_hands_the_query_over_in_a_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let ask = Ask::new(Some(vec!["kitty".into(), "-e".into()]), Some("--ask"));
        let hits = ask.search(&Query::new(" range « mes » photos; rm -rf ~ "), 1);
        let Action::Ask { command, request } = &hits[0].action else {
            panic!("a request");
        };
        assert_eq!(command.program, "kitty");
        // Never the query in the arguments.
        let args: Vec<&OsString> = command.args.iter().collect();
        assert_eq!(args, ["-e", "sherlockd", "--ask"]);
        assert_eq!(request, "range « mes » photos; rm -rf ~");
        let runtime = std::env::temp_dir().join(format!("launcher-ask-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&runtime);
        let path = request_file(&runtime, request).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), *request);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_ne!(request_file(&runtime, request).unwrap(), path);
        let _ = std::fs::remove_dir_all(&runtime);
        // No terminal: disabled.
        assert!(!Ask::new(None, Some("--ask")).is_enabled());
    }
}
