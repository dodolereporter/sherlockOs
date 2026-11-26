//! The pointer setting, `<config dir>/pointer.conf`: what the compositor adds to the pointer.
//! The compositor reads it again when it changes; Control Room's touchpad page writes it.
//!
//! ```text
//! # true: a green ring leaves each click, and the pointer sinks (false by default).
//! show-clicks = false
//! ```

use std::{
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "pointer.conf";

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// The pointer setting. By default, nothing added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Config {
    /// Show the clicks: the accessibility option of the cursor boards.
    pub show_clicks: bool,
}

impl Config {
    /// Reads `key = value` lines; `#` starts a comment line. Unknown keys and values are
    /// refused, so that a typo does not go unnoticed.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut config = Self::default();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let error = |reason: String| Error::Line {
                line: i + 1,
                reason,
            };
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| error("expected key = value".into()))?;
            let value = value.trim();
            match key.trim() {
                "show-clicks" => {
                    config.show_clicks = value
                        .parse()
                        .map_err(|_| error(format!("expected true or false, not {value:?}")))?;
                }
                key => return Err(error(format!("unknown key {key:?}"))),
            }
        }
        Ok(config)
    }

    /// Reads the settings file. A missing file is the default setting.
    pub fn read(path: &Path) -> Result<Self, Error> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read {
                path: path.into(),
                source,
            }),
        }
    }
}

impl fmt::Display for Config {
    /// The file as Control Room writes it: `parse` reads it back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "show-clicks = {}", self.show_clicks)
    }
}

/// `<config dir>/pointer.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clicks_are_not_shown_by_default() {
        assert!(!Config::default().show_clicks);
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::parse("# nothing\n\n").unwrap(), Config::default());
        let missing = std::env::temp_dir().join("desktop-config-no-pointer.conf");
        assert_eq!(Config::read(&missing).unwrap(), Config::default());
    }

    #[test]
    fn the_option_is_read_and_mistakes_are_refused() {
        let on = Config::parse("show-clicks = true").unwrap();
        assert!(on.show_clicks);
        let err = Config::parse("\nshow-clicks = yes").unwrap_err();
        assert!(matches!(err, Error::Line { line: 2, .. }), "{err}");
        let err = Config::parse("clicks = true").unwrap_err();
        assert!(matches!(err, Error::Line { line: 1, .. }), "{err}");
    }

    #[test]
    fn what_control_room_writes_reads_back() {
        for show_clicks in [false, true] {
            let config = Config { show_clicks };
            assert_eq!(Config::parse(&config.to_string()).unwrap(), config);
        }
    }

    #[test]
    fn the_file_is_in_the_config_dir() {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| "/cfg".into());
        assert_eq!(
            config_file(var),
            Some(PathBuf::from("/cfg/sherlockos/pointer.conf"))
        );
    }
}
