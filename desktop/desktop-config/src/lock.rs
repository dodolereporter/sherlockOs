//! The lock setting, `<config dir>/lock.conf`: after how long without input the compositor
//! locks the session (lot D-25a), and the picture behind the lock screen (lot D-25b). The
//! compositor and the lock screen read it at start and again when it changes; Control Room's
//! lock page writes the delay (lot D-26a).
//!
//! ```text
//! # Minutes without input before the session locks; 0: never (5 by default).
//! idle-minutes = 5
//! # The picture behind the lock screen, an absolute path (the theme's picture by default).
//! background = /home/me/Pictures/lock.jpg
//! ```

use std::{
    ffi::OsString,
    fs, io,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Minutes without input before the lock; 0: never.
    pub idle_minutes: u32,
    /// The picture behind the lock screen, instead of the theme's (lot D-25b).
    pub background: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            idle_minutes: DEFAULT_IDLE_MINUTES,
            background: None,
        }
    }
}

impl Config {
    /// The delay before the lock, `None` for never.
    pub fn idle(&self) -> Option<Duration> {
        (self.idle_minutes > 0).then(|| Duration::from_secs(u64::from(self.idle_minutes) * 60))
    }

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
                "idle-minutes" => {
                    config.idle_minutes = value
                        .parse()
                        .ok()
                        .filter(|m| *m <= MAX_IDLE_MINUTES)
                        .ok_or_else(|| {
                            format!("expected 0 to {MAX_IDLE_MINUTES}, not {value:?}")
                        })?;
                    Ok(())
                }
                "background" => {
                    let path = PathBuf::from(value);
                    if !path.is_absolute() {
                        return Err(format!("expected an absolute path, not {value:?}"));
                    }
                    config.background = Some(path);
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
        match fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse_lenient(&text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(source) => Err(Error::Read {
                path: path.into(),
                source,
            }),
        }
    }

    /// `text`, the file as it is, with the delay set again if it changed; everything else
    /// stays (see `touchpad::Config::write_into`). Control Room writes the file so.
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse_lenient(text);
        let values = |c: &Self| [("idle-minutes", c.idle_minutes.to_string())];
        crate::conf::update(text, &values(self), &values(&read))
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
        // The comment stays, the delay changes in place.
        let mut config = Config {
            idle_minutes: 10,
            ..Config::default()
        };
        assert_eq!(
            config.write_into("# mine\nidle-minutes = 5\n"),
            "# mine\nidle-minutes = 10\n"
        );
        config.idle_minutes = 5;
        assert_eq!(
            config.write_into("idle-minutes = 5\n"),
            "idle-minutes = 5\n"
        );
        for idle_minutes in [0, 1, 15, MAX_IDLE_MINUTES] {
            let config = Config {
                idle_minutes,
                ..Config::default()
            };
            assert_eq!(Config::parse(&config.write_into("")).unwrap(), config);
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
    fn never_survives_a_wrong_line() {
        // What the compositor reads: "Jamais" chosen in Control Room, then a typo by hand
        // and a key of a later version. The delay stays at never, the picture is taken.
        let dir = std::env::temp_dir().join(format!("desktop-config-lock-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        fs::write(
            &path,
            "idle-minutes = 0\nidel-minutes = 3\nbackground = /pictures/lock.jpg\nfuture = 1\n",
        )
        .unwrap();
        let (config, problems) = Config::read_lenient(&path).unwrap();
        assert_eq!(config.idle(), None);
        assert_eq!(config.background, Some(PathBuf::from("/pictures/lock.jpg")));
        assert_eq!(problems.len(), 2);
        assert!(matches!(problems[0], Error::Line { line: 2, .. }));
        assert_eq!(Config::read(&path).unwrap().idle(), None);
        // A wrong value of the delay itself leaves it at its default, the rest still counts.
        fs::write(
            &path,
            "idle-minutes = soon\nbackground = /pictures/lock.jpg\n",
        )
        .unwrap();
        let (config, problems) = Config::read_lenient(&path).unwrap();
        assert_eq!(config.idle_minutes, DEFAULT_IDLE_MINUTES);
        assert!(config.background.is_some() && problems.len() == 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_file_is_in_the_config_dir() {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| "/cfg".into());
        assert_eq!(
            config_file(var),
            Some(PathBuf::from("/cfg/sherlockos/lock.conf"))
        );
    }

    #[test]
    fn the_background_is_an_absolute_path() {
        assert_eq!(Config::default().background, None);
        let set = Config::parse("background = /srv/lock.jpg").unwrap();
        assert_eq!(set.background, Some(PathBuf::from("/srv/lock.jpg")));
        assert!(Config::parse("background = lock.jpg").is_err());
    }
}
