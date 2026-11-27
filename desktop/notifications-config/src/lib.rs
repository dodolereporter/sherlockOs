//! The notifications settings file, `$XDG_CONFIG_HOME/notifications/notifications.conf`:
//! reading, writing, and reading again when it changed. A library of its own, so that the
//! notifications daemon, the quick settings panel and the settings application (Control Room)
//! share the same file the same way.
//!
//! ```text
//! [Notifications]
//! DoNotDisturb=true
//! DoNotDisturbUntil=1795892400
//! ```
//!
//! `DoNotDisturbUntil` (D-23e) ends Do not disturb at that instant, in seconds since 1970:
//! « Pendant 1 heure », « Jusqu'à ce soir ». Without it, it lasts until switched off.
//!
//! Every key is optional; an absent file gives the defaults. See `docs/notifications.md`.

use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use dock_config::KeyFile;

const GROUP: &str = "Notifications";
const DO_NOT_DISTURB: &str = "DoNotDisturb";
const UNTIL: &str = "DoNotDisturbUntil";

/// The notifications settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotificationsConfig {
    /// Nothing shows but critical notifications; all of them still go to the history.
    pub do_not_disturb: bool,
    /// When Do not disturb ends by itself, in seconds since 1970.
    pub until: Option<u64>,
}

impl NotificationsConfig {
    /// Do not disturb holds at `now` (seconds since 1970): on, and its end not reached.
    pub fn active_at(&self, now: u64) -> bool {
        self.do_not_disturb && self.until.is_none_or(|until| now < until)
    }

    /// `active_at` the system clock's now.
    pub fn active_now(&self) -> bool {
        self.active_at(epoch_now())
    }

    /// Reads a text: the settings, and the value that could not be read, if any (the default
    /// is then used).
    pub fn parse(text: &str) -> (Self, Option<String>) {
        let file = KeyFile::parse(text);
        let until = file.get(GROUP, UNTIL);
        let Some(raw) = file.get(GROUP, DO_NOT_DISTURB) else {
            return (Self::default(), None);
        };
        let on = match raw.trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => return (Self::default(), Some(raw)),
        };
        match until.as_deref().map(str::trim) {
            None | Some("") => (
                Self {
                    do_not_disturb: on,
                    until: None,
                },
                None,
            ),
            Some(text) => match text.parse::<u64>() {
                Ok(at) => (
                    Self {
                        do_not_disturb: on,
                        until: Some(at),
                    },
                    None,
                ),
                // An end that cannot be read: on until switched off, as without one.
                Err(_) => (
                    Self {
                        do_not_disturb: on,
                        until: None,
                    },
                    Some(text.to_owned()),
                ),
            },
        }
    }

    /// Reads the file at `path`; an absent file gives the defaults.
    pub fn load(path: &Path) -> io::Result<(Self, Option<String>)> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((Self::default(), None)),
            Err(error) => Err(error),
        }
    }

    /// Writes the settings into the file at `path`, creating its directory and keeping its
    /// other lines. The text goes to a temporary file of its own, renamed over the old one: a
    /// reader never sees half a file, and two writers never mix.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut file = match std::fs::read_to_string(path) {
            Ok(text) => KeyFile::parse(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => KeyFile::default(),
            Err(error) => return Err(error),
        };
        let value = if self.do_not_disturb { "true" } else { "false" };
        file.set(GROUP, DO_NOT_DISTURB, value);
        let until = self.until.map(|at| at.to_string()).unwrap_or_default();
        file.set(GROUP, UNTIL, &until);
        let dir = path.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let count = SAVES.fetch_add(1, Ordering::Relaxed);
        let temporary = dir.join(format!(".{name}.{}.{count}.tmp", std::process::id()));
        let written = (|| {
            let mut out = std::fs::File::create(&temporary)?;
            out.write_all(file.to_text().as_bytes())?;
            out.sync_all()?;
            std::fs::rename(&temporary, path)
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        written
    }
}

/// The system clock, in seconds since 1970.
pub fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Saves made by this process, for unique temporary names.
static SAVES: AtomicU64 = AtomicU64::new(0);

/// Path of the file: `$XDG_CONFIG_HOME/notifications/notifications.conf`.
pub fn path() -> Option<PathBuf> {
    Some(dock_config::config_home()?.join("notifications/notifications.conf"))
}

/// The file read again whenever it changed (modification time, size or inode: a file replaced
/// by a rename too), at the moment its value is needed. No thread and no watch: the daemon only
/// needs the value when a notification comes in.
#[derive(Debug)]
pub struct Follower {
    path: PathBuf,
    stamp: Option<(SystemTime, u64, u64)>,
    config: NotificationsConfig,
}

impl Follower {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            stamp: None,
            config: NotificationsConfig::default(),
        }
    }

    /// The settings as the file holds them now. A file that cannot be read gives the defaults;
    /// the error, or the value that was ignored, is returned once per change of the file.
    pub fn current(&mut self) -> (NotificationsConfig, Option<String>) {
        let stamp = std::fs::metadata(&self.path)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len(), m.ino())));
        if stamp.is_some() && stamp == self.stamp {
            return (self.config, None);
        }
        self.stamp = stamp;
        let (config, problem) = match NotificationsConfig::load(&self.path) {
            Ok(read) => read,
            Err(error) => (NotificationsConfig::default(), Some(error.to_string())),
        };
        self.config = config;
        (config, problem)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notifications-config-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("notifications/notifications.conf")
    }

    #[test]
    fn reads_the_setting_and_its_defaults() {
        assert_eq!(
            NotificationsConfig::parse(""),
            (NotificationsConfig::default(), None)
        );
        let on = NotificationsConfig {
            do_not_disturb: true,
            until: None,
        };
        assert_eq!(
            NotificationsConfig::parse("[Notifications]\nDoNotDisturb=True\n"),
            (on, None)
        );
        let (config, problem) = NotificationsConfig::parse("[Notifications]\nDoNotDisturb=oui\n");
        assert_eq!(
            (config, problem.as_deref()),
            (NotificationsConfig::default(), Some("oui"))
        );
    }

    #[test]
    fn do_not_disturb_ends_at_its_time() {
        let (config, problem) = NotificationsConfig::parse(
            "[Notifications]\nDoNotDisturb=true\nDoNotDisturbUntil=1000\n",
        );
        assert_eq!((config.until, problem), (Some(1000), None));
        assert!(config.active_at(999) && !config.active_at(1000));
        let (config, problem) =
            NotificationsConfig::parse("[Notifications]\nDoNotDisturb=true\nDoNotDisturbUntil=x\n");
        assert!(config.active_at(u64::MAX) && problem.is_some());
        assert!(!NotificationsConfig::default().active_at(0));
        let path = temp("until");
        let timed = NotificationsConfig {
            do_not_disturb: true,
            until: Some(42),
        };
        timed.save(&path).unwrap();
        assert_eq!(NotificationsConfig::load(&path).unwrap().0, timed);
        let endless = NotificationsConfig {
            until: None,
            ..timed
        };
        endless.save(&path).unwrap();
        assert_eq!(NotificationsConfig::load(&path).unwrap().0, endless);
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn saving_keeps_the_other_lines() {
        let path = temp("save");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# mine\n[Notifications]\nOther=1\n").unwrap();
        NotificationsConfig {
            do_not_disturb: true,
            until: None,
        }
        .save(&path)
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# mine\n[Notifications]\nOther=1\nDoNotDisturb=true\n"),
            "{text}"
        );
        let leftovers = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1);
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn the_follower_sees_each_change() {
        let path = temp("follow");
        let mut follower = Follower::new(path.clone());
        assert!(!follower.current().0.do_not_disturb);
        NotificationsConfig {
            do_not_disturb: true,
            until: None,
        }
        .save(&path)
        .unwrap();
        assert!(follower.current().0.do_not_disturb);
        // Replaced by a rename, as editors do: a new inode.
        let other = path.with_extension("new");
        std::fs::write(&other, "[Notifications]\nDoNotDisturb=false\n").unwrap();
        std::fs::rename(&other, &path).unwrap();
        assert!(!follower.current().0.do_not_disturb);
        std::fs::remove_file(&path).unwrap();
        assert!(!follower.current().0.do_not_disturb);
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }
}
