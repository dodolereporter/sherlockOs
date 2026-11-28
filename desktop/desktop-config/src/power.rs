//! The power setting, `<config dir>/power.conf`: after how long without input the compositor
//! turns the screens off (lot D-38), without sleeping. The compositor reads it at start and
//! again when it changes; Control Room will write it.
//!
//! ```text
//! # Minutes without input before the screens go off; 0: never (10 by default).
//! screen-off-minutes = 10
//! ```

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "power.conf";

/// Minutes without input before the screens go off, by default.
pub const DEFAULT_SCREEN_OFF_MINUTES: u32 = 10;

/// The longest delay accepted: a day.
pub const MAX_MINUTES: u32 = 24 * 60;

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// The power setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Minutes without input before the screens go off; 0: never.
    pub screen_off_minutes: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            screen_off_minutes: DEFAULT_SCREEN_OFF_MINUTES,
        }
    }
}

impl Config {
    /// The delay before the screens go off, `None` for never.
    pub fn screen_off(&self) -> Option<Duration> {
        (self.screen_off_minutes > 0)
            .then(|| Duration::from_secs(u64::from(self.screen_off_minutes) * 60))
    }

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
                "screen-off-minutes" => {
                    config.screen_off_minutes = value
                        .parse()
                        .ok()
                        .filter(|m| *m <= MAX_MINUTES)
                        .ok_or_else(|| {
                            error(format!("expected 0 to {MAX_MINUTES}, not {value:?}"))
                        })?;
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

/// `<config dir>/power.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screens_go_off_after_ten_minutes_by_default() {
        assert_eq!(
            Config::default().screen_off(),
            Some(Duration::from_secs(600))
        );
        let missing = std::env::temp_dir().join("desktop-config-no-power.conf");
        assert_eq!(Config::read(&missing).unwrap(), Config::default());
        assert_eq!(
            Config::parse("screen-off-minutes = 0")
                .unwrap()
                .screen_off(),
            None
        );
        let two = Config::parse("screen-off-minutes = 2").unwrap();
        assert_eq!(two.screen_off(), Some(Duration::from_secs(120)));
        for wrong in [
            "screen-off-minutes = -1",
            "screen-off = 2",
            "screen-off-minutes = 99999",
        ] {
            assert!(Config::parse(wrong).is_err(), "{wrong}");
        }
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| "/cfg".into());
        assert_eq!(
            config_file(var),
            Some(PathBuf::from("/cfg/sherlockos/power.conf"))
        );
    }
}
