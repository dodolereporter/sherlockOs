//! The lock setting, `<config dir>/lock.conf`, as the compositor of lot D-25a reads it:
//! `idle-minutes = N`, 0 for never, 5 by default. Kept here until D-25a brings
//! `desktop_config::lock` to the main branch; this module then gives way to it.

use std::path::Path;

/// Name of the file, in the desktop's configuration folder.
pub const FILE: &str = "lock.conf";
/// Minutes without input before the lock, by default (D-25a).
pub const DEFAULT_IDLE_MINUTES: u32 = 5;
/// The longest delay the compositor accepts: a day.
pub const MAX_IDLE_MINUTES: u32 = 24 * 60;

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
    /// Reads the file; a missing file, a line it cannot read, gives the default and goes to
    /// the log, as the compositor does.
    pub fn read(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let minutes = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key.trim() == "idle-minutes").then(|| value.trim().parse::<u32>().ok())
            });
        match minutes {
            None => Self::default(),
            Some(Some(m)) if m <= MAX_IDLE_MINUTES => Self { idle_minutes: m },
            Some(_) => {
                tracing::warn!(path = %path.display(), "lock delay not read");
                Self::default()
            }
        }
    }

    /// The file as the compositor reads it.
    pub fn to_text(self) -> String {
        format!(
            "idle-minutes = {}\n",
            self.idle_minutes.min(MAX_IDLE_MINUTES)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let dir = std::env::temp_dir().join(format!("control-room-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        assert_eq!(Config::read(&path), Config::default());
        for minutes in [0, 1, 15, MAX_IDLE_MINUTES] {
            let config = Config {
                idle_minutes: minutes,
            };
            std::fs::write(&path, config.to_text()).unwrap();
            assert_eq!(Config::read(&path), config);
        }
        std::fs::write(&path, "# mine\nidle-minutes = soon\n").unwrap();
        assert_eq!(Config::read(&path), Config::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
