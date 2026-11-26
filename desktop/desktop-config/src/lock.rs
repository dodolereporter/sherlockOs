//! The lock setting, `<config dir>/lock.conf`: after how long without input the compositor
//! locks the session (lot D-25a). The compositor reads it at start and again when it changes;
//! Control Room's lock page writes it (lot D-26a).
//!
//! ```text
//! # Minutes without input before the session locks; 0: never (5 by default).
//! idle-minutes = 5
//! ```

use std::{
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "lock.conf";

/// Minutes without input before the lock, by default.
pub const DEFAULT_IDLE_MINUTES: u32 = 5;

/// The longest delay accepted: a day.
pub const MAX_IDLE_MINUTES: u32 = 24 * 60;

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// The lock setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Minutes without input before the lock; 0: never.
    pub idle_minutes: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            idle_minutes: DEFAULT_IDLE_MINUTES,
        }
    }
}

impl Config {
    /// The delay before the lock, `None` for never.
    pub fn idle(&self) -> Option<Duration> {
        (self.idle_minutes > 0).then(|| Duration::from_secs(u64::from(self.idle_minutes) * 60))
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
                "idle-minutes" => {
                    config.idle_minutes = value
                        .parse()
                        .ok()
                        .filter(|m| *m <= MAX_IDLE_MINUTES)
                        .ok_or_else(|| {
                            error(format!("expected 0 to {MAX_IDLE_MINUTES}, not {value:?}"))
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

impl fmt::Display for Config {
    /// The file as Control Room writes it: `parse` reads it back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "idle-minutes = {}", self.idle_minutes)
    }
}

/// `<config dir>/lock.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_control_room_writes_reads_back() {
        for idle_minutes in [0, 1, 15, MAX_IDLE_MINUTES] {
            let config = Config { idle_minutes };
            assert_eq!(Config::parse(&config.to_string()).unwrap(), config);
        }
    }

    #[test]
    fn the_session_locks_after_five_minutes_by_default() {
        assert_eq!(Config::default().idle(), Some(Duration::from_secs(300)));
        assert_eq!(Config::parse("# nothing\n").unwrap(), Config::default());
        let missing = std::env::temp_dir().join("desktop-config-no-lock.conf");
        assert_eq!(Config::read(&missing).unwrap(), Config::default());
    }

    #[test]
    fn zero_is_never_and_mistakes_are_refused() {
        assert_eq!(Config::parse("idle-minutes = 0").unwrap().idle(), None);
        let ten = Config::parse("idle-minutes = 10").unwrap();
        assert_eq!(ten.idle(), Some(Duration::from_secs(600)));
        for wrong in [
            "idle-minutes = -1",
            "idle-minutes = 5 min",
            "idle-minutes = 99999",
            "idle = 5",
        ] {
            assert!(Config::parse(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn the_file_is_in_the_config_dir() {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| "/cfg".into());
        assert_eq!(
            config_file(var),
            Some(PathBuf::from("/cfg/sherlockos/lock.conf"))
        );
    }
}
