//! The clipboard history setting, `<config dir>/clipboard.conf` (lot D-42): whether the
//! compositor keeps what is copied, and whether that history, the pinned items with it, is
//! kept on disk between sessions. The compositor reads it at start and again when it changes;
//! Control Room writes it.
//!
//! ```text
//! # Keep what is copied, for Super + V (on by default).
//! history = on
//! # Keep the history on disk, the pinned items with it (off by default: memory only).
//! keep-on-disk = off
//! ```

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "clipboard.conf";

/// Name of the file that holds the history kept on disk, in the desktop's state folder.
pub const HISTORY_FILE: &str = "clipboard-history";

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// The clipboard history setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// The compositor keeps what is copied.
    pub history: bool,
    /// The history is written to disk, and read back at the next session.
    pub keep_on_disk: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            history: true,
            keep_on_disk: false,
        }
    }
}

fn switch(value: &str) -> Result<bool, String> {
    match value {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("expected on or off, not {value:?}")),
    }
}

fn word(on: bool) -> String {
    String::from(if on { "on" } else { "off" })
}

impl Config {
    /// Reads `key = value` lines; `#` starts a comment line. Unknown keys and values are
    /// refused: the error of the first one is returned.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (config, mut problems) = Self::parse_lenient(text);
        match problems.is_empty() {
            true => Ok(config),
            false => Err(problems.remove(0)),
        }
    }

    /// Reads what can be read: each line that cannot is left at its default and listed.
    pub fn parse_lenient(text: &str) -> (Self, Vec<Error>) {
        let mut config = Self::default();
        let mut problems = Vec::new();
        for (line, pair) in crate::conf::pairs(text) {
            let taken = pair.and_then(|(key, value)| match key {
                "history" => switch(value).map(|on| config.history = on),
                "keep-on-disk" => switch(value).map(|on| config.keep_on_disk = on),
                key => Err(format!("unknown key {key:?}")),
            });
            if let Err(reason) = taken {
                problems.push(Error::Line { line, reason });
            }
        }
        (config, problems)
    }

    /// Reads the settings file. A missing file is the default setting.
    pub fn read(path: &Path) -> Result<Self, Error> {
        match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read {
                path: path.into(),
                source,
            }),
        }
    }

    /// `text`, the file as it is, with what changed set again; everything else stays.
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse_lenient(text);
        let values = |c: &Self| {
            [
                ("history", word(c.history)),
                ("keep-on-disk", word(c.keep_on_disk)),
            ]
        };
        crate::conf::update(text, &values(self), &values(&read))
    }
}

/// `<config dir>/clipboard.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

/// `$XDG_STATE_HOME/<DIR>/clipboard-history`, or `~/.local/state/<DIR>/clipboard-history`:
/// where the history goes when it is kept on disk.
pub fn history_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |v: Option<OsString>| v.map(PathBuf::from).filter(|p| p.is_absolute());
    let state = absolute(var("XDG_STATE_HOME"))
        .or_else(|| absolute(var("HOME")).map(|home| home.join(".local/state")))?;
    Some(state.join(crate::DIR).join(HISTORY_FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_is_on_and_in_memory_by_default() {
        assert_eq!(Config::parse("# nothing\n").unwrap(), Config::default());
        assert!(Config::default().history);
        assert!(!Config::default().keep_on_disk);
        let missing = std::env::temp_dir().join("desktop-config-no-clipboard.conf");
        assert_eq!(Config::read(&missing).unwrap(), Config::default());
    }

    #[test]
    fn what_control_room_writes_reads_back() {
        for history in [true, false] {
            for keep_on_disk in [true, false] {
                let config = Config {
                    history,
                    keep_on_disk,
                };
                assert_eq!(Config::parse(&config.write_into("")).unwrap(), config);
            }
        }
        let off = Config {
            history: false,
            ..Config::default()
        };
        assert_eq!(
            off.write_into("# mine\nhistory = on\n"),
            "# mine\nhistory = off\n"
        );
    }

    #[test]
    fn mistakes_are_refused() {
        for wrong in [
            "history = yes",
            "keep-on-disk = 1",
            "histroy = on",
            "history",
        ] {
            assert!(Config::parse(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_files_follow_xdg() {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| "/cfg".into());
        assert_eq!(
            config_file(var),
            Some(PathBuf::from("/cfg/sherlockos/clipboard.conf"))
        );
        let home = |key: &str| (key == "HOME").then(|| "/home/u".into());
        assert_eq!(
            history_file(home),
            Some(PathBuf::from(
                "/home/u/.local/state/sherlockos/clipboard-history"
            ))
        );
        assert_eq!(history_file(|_| None), None);
    }
}
