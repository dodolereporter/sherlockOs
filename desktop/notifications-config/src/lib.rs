//! The notifications settings file, `$XDG_CONFIG_HOME/notifications/notifications.conf`:
//! reading, writing, and reading again when it changed. A library of its own, so that the
//! notifications daemon, the quick settings panel and the settings application (Control Room)
//! share the same file the same way.
//!
//! ```text
//! [Notifications]
//! DoNotDisturb=true
//! ```
//!
//! Every key is optional; an absent file gives the defaults. See `docs/notifications.md`.

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dock_config::KeyFile;

const GROUP: &str = "Notifications";
const DO_NOT_DISTURB: &str = "DoNotDisturb";

/// The notifications settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NotificationsConfig {
    /// Nothing shows but critical notifications; all of them still go to the history.
    pub do_not_disturb: bool,
}

impl NotificationsConfig {
    /// Reads a text: the settings, and the value that could not be read, if any (the default
    /// is then used).
    pub fn parse(text: &str) -> (Self, Option<String>) {
        let file = KeyFile::parse(text);
        let Some(raw) = file.get(GROUP, DO_NOT_DISTURB) else {
            return (Self::default(), None);
        };
        match raw.trim().to_ascii_lowercase().as_str() {
            "true" => (
                Self {
                    do_not_disturb: true,
                },
                None,
            ),
            "false" => (Self::default(), None),
            _ => (Self::default(), Some(raw)),
        }
    }

    /// Reads the file at `path`; an absent file gives the defaults.
    pub fn load(path: &Path) -> io::Result<(Self, Option<String>)> {
        match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok((Self::default(), None)),
            Err(error) => Err(error),
        }
    }

    /// Writes the settings into the file at `path`, creating its directory and keeping its
    /// other lines, with `mime_apps::write_atomically`: a reader never sees half a file, two
    /// writers never mix, a linked file (dotfiles) is written where it lives and keeps its
    /// permissions. A file that cannot be read is not written over.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut file = match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
            Ok(text) => KeyFile::parse(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => KeyFile::default(),
            Err(error) => return Err(error),
        };
        let value = if self.do_not_disturb { "true" } else { "false" };
        file.set(GROUP, DO_NOT_DISTURB, value);
        mime_apps::write_atomically(path, &file.to_text())
    }
}

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
    fn saving_keeps_the_other_lines() {
        let path = temp("save");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# mine\n[Notifications]\nOther=1\n").unwrap();
        NotificationsConfig {
            do_not_disturb: true,
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
        // A linked file (dotfiles) stays a link, and a private one stays private.
        use std::os::unix::fs::PermissionsExt;
        let dir = path.parent().unwrap();
        let target = dir.join("dotfiles.conf");
        std::fs::rename(&path, &target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink("dotfiles.conf", &path).unwrap();
        NotificationsConfig {
            do_not_disturb: false,
        }
        .save(&path)
        .unwrap();
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(
            std::fs::read_to_string(&target)
                .unwrap()
                .contains("DoNotDisturb=false")
        );
        std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn the_follower_sees_each_change() {
        let path = temp("follow");
        let mut follower = Follower::new(path.clone());
        assert!(!follower.current().0.do_not_disturb);
        NotificationsConfig {
            do_not_disturb: true,
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
