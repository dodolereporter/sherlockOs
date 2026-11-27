//! The pointer setting, `<config dir>/pointer.conf`: what the compositor adds to the pointer.
//! The compositor reads it again when it changes; Control Room's touchpad page writes it.
//!
//! ```text
//! # true: a green ring leaves each click, and the pointer sinks (false by default).
//! show-clicks = false
//! ```

use std::{
    ffi::OsString,
    fs, io,
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
    /// refused, so that a typo does not go unnoticed: the error of the first one is returned.
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
                "show-clicks" => {
                    config.show_clicks = value
                        .parse()
                        .map_err(|_| format!("expected true or false, not {value:?}"))?;
                    Ok(())
                }
                key => Err(format!("unknown key {key:?}")),
            });
            if let Err(reason) = taken {
                problems.push(Error::Line { line, reason });
            }
        }
        (config, problems)
    }

    /// Reads the settings file as `read_lenient` does, without the list of its wrong lines.
    pub fn read(path: &Path) -> Result<Self, Error> {
        Self::read_lenient(path).map(|(config, _)| config)
    }

    /// Reads the settings file. A missing file is the default setting. A line that cannot be
    /// taken is left out and listed, and the other lines still count: one typo never turns
    /// the whole file back to the defaults (a lock set to never would lock after 5 minutes).
    /// Only a file that cannot be read at all is an error.
    pub fn read_lenient(path: &Path) -> Result<(Self, Vec<Error>), Error> {
        match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
            Ok(text) => Ok(Self::parse_lenient(&text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(source) => Err(Error::Read {
                path: path.into(),
                source,
            }),
        }
    }

    /// `text`, the file as it is, with the keys whose value changed set again; everything
    /// else stays (see `touchpad::Config::write_into`).
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse_lenient(text);
        let values = |c: &Self| [("show-clicks", c.show_clicks.to_string())];
        crate::conf::update(text, &values(self), &values(&read))
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
            assert_eq!(Config::parse(&config.write_into("")).unwrap(), config);
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
