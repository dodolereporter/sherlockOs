//! The settings files Control Room reads and writes, and Sherlock's settings server too (lot
//! A-17): out of Control Room, so that both go through the same functions. They are the files
//! the other programs already read and follow live: the appearance (every program), the dock,
//! the wallpaper, the touchpad, the keyboard and the pointer (the compositor). Nothing here
//! depends on iced, so that each file is written and read back in a temporary folder by the
//! tests.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use appearance_config::AppearanceConfig;
use desktop_config::{clipboard, keyboard, lock, pointer, touchpad, wallpaper};
use dock_config::DockConfig;

/// Where each file is, and where the installed wallpapers are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub appearance: PathBuf,
    pub dock: PathBuf,
    pub wallpaper: PathBuf,
    pub touchpad: PathBuf,
    pub keyboard: PathBuf,
    pub pointer: PathBuf,
    /// Folders of installed wallpapers, most important first.
    pub wallpaper_dirs: Vec<PathBuf>,
    pub lock: PathBuf,
    pub clipboard: PathBuf,
    /// Where the applications and `mimeapps.list` are, the current desktops (lower case,
    /// for `<desktop>-mimeapps.list`) and the locales of the application names.
    pub apps: mime_apps::Dirs,
    pub desktops: Vec<String>,
    pub locales: Vec<String>,
    /// The `pactl` the sound page runs.
    pub pactl: OsString,
}

impl Paths {
    /// The files under `config_home`, as `$XDG_CONFIG_HOME`, the wallpapers in
    /// `wallpaper_dirs`.
    pub fn under(config_home: &Path, wallpaper_dirs: Vec<PathBuf>) -> Self {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| OsString::from(config_home));
        let desktop = desktop_config::config_dir(var).unwrap_or_else(|| config_home.into());
        Self {
            appearance: config_home.join("appearance/appearance.conf"),
            dock: config_home.join("dock/dock.conf"),
            wallpaper: desktop.join(wallpaper::FILE),
            touchpad: desktop.join(touchpad::FILE),
            keyboard: desktop.join(keyboard::FILE),
            pointer: desktop.join(pointer::FILE),
            lock: desktop.join(lock::FILE),
            clipboard: desktop.join(clipboard::FILE),
            wallpaper_dirs,
            apps: mime_apps::Dirs {
                config_home: config_home.into(),
                ..mime_apps::Dirs::default()
            },
            desktops: Vec::new(),
            locales: Vec::new(),
            pactl: OsString::from("pactl"),
        }
    }

    /// The files of the environment, found as each program finds them: `None` without a home
    /// folder.
    pub fn from_env() -> Option<Self> {
        let var = |key: &str| std::env::var_os(key);
        let home = PathBuf::from(var("HOME")?);
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|d| !d.is_empty())
            .map(str::to_lowercase)
            .collect();
        Some(Self {
            lock: lock::config_file(var)?,
            clipboard: clipboard::config_file(var)?,
            apps: mime_apps::Dirs::from_env(&home),
            desktops,
            locales: locales(),
            pactl: OsString::from("pactl"),
            appearance: appearance_config::path()?,
            dock: dock_config::path()?,
            wallpaper: wallpaper::config_file(var)?,
            touchpad: touchpad::config_file(var)?,
            keyboard: keyboard::config_file(var)?,
            pointer: pointer::config_file(var)?,
            wallpaper_dirs: wallpaper::dirs(var),
        })
    }
}

/// One settings file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum File {
    Appearance,
    Dock,
    Wallpaper,
    Touchpad,
    Keyboard,
    Pointer,
    Lock,
    Clipboard,
}

impl Paths {
    /// Where `file` is.
    pub fn of(&self, file: File) -> &Path {
        match file {
            File::Appearance => &self.appearance,
            File::Dock => &self.dock,
            File::Wallpaper => &self.wallpaper,
            File::Touchpad => &self.touchpad,
            File::Keyboard => &self.keyboard,
            File::Pointer => &self.pointer,
            File::Lock => &self.lock,
            File::Clipboard => &self.clipboard,
        }
    }
}

impl File {
    /// The group of the file's keys, for those that have one.
    pub fn group(self) -> Option<&'static str> {
        match self {
            Self::Appearance => Some("Appearance"),
            Self::Dock => Some("Dock"),
            _ => None,
        }
    }

    pub const ALL: [Self; 8] = [
        Self::Appearance,
        Self::Dock,
        Self::Wallpaper,
        Self::Touchpad,
        Self::Keyboard,
        Self::Pointer,
        Self::Lock,
        Self::Clipboard,
    ];
}

/// Every setting Control Room shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub appearance: AppearanceConfig,
    pub dock: DockConfig,
    pub wallpaper: wallpaper::Config,
    pub touchpad: touchpad::Config,
    pub keyboard: keyboard::Config,
    pub pointer: pointer::Config,
    pub lock: lock::Config,
    pub clipboard: clipboard::Config,
}

impl Settings {
    /// Reads every file, and lists those that cannot be changed: unreadable (permissions) or
    /// not text. Those keep their defaults on screen and are never written. In a file that is
    /// read, a line that cannot be understood is left aside and goes to the log; the others
    /// count.
    pub fn load(paths: &Paths) -> (Self, Vec<File>) {
        let mut settings = Self::default();
        let mut locked = Vec::new();
        for file in File::ALL {
            let path = paths.of(file);
            match read_text(path) {
                Ok(text) => {
                    settings.read(file, path, &text);
                }
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "settings file not changeable");
                    locked.push(file);
                }
            }
        }
        (settings, locked)
    }

    /// Takes the settings of `file` from its text; what is not understood goes to the log, and
    /// is given back.
    pub fn read(&mut self, file: File, path: &Path, text: &str) -> Vec<String> {
        let problems: Vec<String> = match file {
            File::Appearance => {
                let (config, problems) = AppearanceConfig::parse(text);
                self.appearance = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Dock => {
                let (config, problems) = DockConfig::parse(text);
                self.dock = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Wallpaper => {
                let (config, problems) = wallpaper::Config::parse_lenient(text);
                self.wallpaper = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Touchpad => {
                let (config, problems) = touchpad::Config::parse_lenient(text);
                self.touchpad = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Keyboard => {
                let (config, problems) = keyboard::Config::parse_lenient(text);
                self.keyboard = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Pointer => {
                let (config, problems) = pointer::Config::parse_lenient(text);
                self.pointer = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Lock => {
                let (config, problems) = lock::Config::parse_lenient(text);
                self.lock = config;
                problems.iter().map(ToString::to_string).collect()
            }
            File::Clipboard => {
                let (config, problems) = clipboard::Config::parse_lenient(text);
                self.clipboard = config;
                problems.iter().map(ToString::to_string).collect()
            }
        };
        for problem in &problems {
            tracing::warn!(%problem, path = %path.display(), "settings line left aside");
        }
        problems
    }

    /// The settings of `file` taken from `other`.
    pub fn take(&mut self, file: File, other: &Self) {
        match file {
            File::Appearance => self.appearance = other.appearance,
            File::Dock => self.dock = other.dock.clone(),
            File::Wallpaper => self.wallpaper = other.wallpaper.clone(),
            File::Touchpad => self.touchpad = other.touchpad,
            File::Keyboard => self.keyboard = other.keyboard.clone(),
            File::Pointer => self.pointer = other.pointer,
            File::Lock => self.lock = other.lock.clone(),
            File::Clipboard => self.clipboard = other.clipboard,
        }
    }

    /// Sets `key` of `file` to `value`, as a line of the file says it (lot A-17): the line is
    /// put in the file's text in memory, the file's own parser reads it, and only a value it
    /// understands is kept, then written by [`Settings::update`]. `Err(None)` for a value the
    /// parser refuses, `Err(Some(error))` when the file cannot be read or written.
    pub fn set_key(
        &mut self,
        file: File,
        paths: &Paths,
        key: &str,
        value: &str,
    ) -> Result<(), Option<io::Error>> {
        if value.chars().any(char::is_control) || key.chars().any(|c| !c.is_ascii_graphic()) {
            return Err(None);
        }
        let path = paths.of(file);
        let text = read_text(path).map_err(Some)?;
        let candidate = match file.group() {
            Some(group) => set_in_group(&text, group, key, value),
            None => desktop_config::conf::set(&text, key, value),
        };
        let mut probe = self.clone();
        let before = probe.read(file, path, &text).len();
        if probe.read(file, path, &candidate).len() > before {
            return Err(None);
        }
        self.update(file, paths, |fresh| fresh.take(file, &probe))
            .map_err(Some)
    }

    /// `text` with the keys of `file` whose value these settings change set again.
    fn write_into(&self, file: File, text: &str) -> String {
        match file {
            File::Appearance => self.appearance.write_into(text),
            // The dock keeps the text it was read from.
            File::Dock => self.dock.to_text(),
            File::Wallpaper => self.wallpaper.write_into(text),
            File::Touchpad => self.touchpad.write_into(text),
            File::Keyboard => self.keyboard.write_into(text),
            File::Pointer => self.pointer.write_into(text),
            File::Lock => self.lock.write_into(text),
            File::Clipboard => self.clipboard.write_into(text),
        }
    }

    /// Changes `file` on the disk: reads it as it is now, takes what it says (another program
    /// or the user may have changed it since), makes `change` there, and writes only the keys
    /// that change, with `mime_apps::write_atomically`. These settings then show the file as
    /// written. A file that cannot be read is not written.
    pub fn update(
        &mut self,
        file: File,
        paths: &Paths,
        change: impl FnOnce(&mut Self),
    ) -> io::Result<()> {
        let path = paths.of(file);
        let text = read_text(path)?;
        let mut fresh = self.clone();
        let _ = fresh.read(file, path, &text);
        change(&mut fresh);
        let written = fresh.write_into(file, &text);
        if written != text {
            mime_apps::write_atomically(path, &written)?;
        }
        // Read back from what was written: the dock keeps that text for its next change.
        let _ = self.read(file, path, &written);
        Ok(())
    }
}

/// `text` with `key` set to `value` in the group `[group]`: its last line there is replaced, or a
/// line added at the end of the group, the group added at the end of the file without one.
/// Every other line is kept.
pub fn set_in_group(text: &str, group: &str, key: &str, value: &str) -> String {
    let header = format!("[{group}]");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let start = lines.iter().position(|l| l.trim() == header);
    let Some(start) = start else {
        let mut text = text.to_owned();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        return format!("{text}{header}\n{key}={value}\n");
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |n| start + 1 + n);
    let line = format!("{key}={value}");
    let same_key = |l: &String| {
        l.split_once('=')
            .is_some_and(|(k, _)| k.trim() == key && !l.trim_start().starts_with('#'))
    };
    match (start + 1..end).rev().find(|i| same_key(&lines[*i])) {
        Some(at) => lines[at] = line,
        None => {
            // After the group's last line that is not blank.
            let mut at = end;
            while at > start + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, line);
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The text of a settings file, empty when there is none. A file that cannot be read, or that
/// is not text (not UTF-8, a NUL byte), is an error: it is never written over.
pub fn read_text(path: &Path) -> io::Result<String> {
    match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
        Ok(text) if text.contains('\0') => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a text file",
        )),
        Ok(text) => Ok(text),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// Locales to try for application names, most specific first: `fr_FR`, then `fr` (as
/// Evidence).
fn locales() -> Vec<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let base = raw.split(['.', '@']).next().unwrap_or_default().to_owned();
    match base.split_once('_') {
        _ if base.is_empty() || base == "C" || base == "POSIX" => Vec::new(),
        Some((language, _)) => vec![base.clone(), language.to_owned()],
        None => vec![base],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_set_in_its_group_and_a_refused_value_is_never_written() {
        let text = "# mine\n[Other]\nTheme=x\n[Appearance]\nTheme=auto\n\n[Last]\nA=1\n";
        let set = set_in_group(text, "Appearance", "Theme", "dark");
        assert_eq!(
            set,
            "# mine\n[Other]\nTheme=x\n[Appearance]\nTheme=dark\n\n[Last]\nA=1\n"
        );
        let added = set_in_group(
            "[Appearance]\nTheme=auto\n",
            "Appearance",
            "AutoDarkFrom",
            "20:00",
        );
        assert_eq!(added, "[Appearance]\nTheme=auto\nAutoDarkFrom=20:00\n");
        assert_eq!(
            set_in_group("", "Dock", "Position", "left"),
            "[Dock]\nPosition=left\n"
        );

        let dir = std::env::temp_dir().join(format!("settings-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let paths = Paths::under(&dir, Vec::new());
        let (mut settings, _) = Settings::load(&paths);
        settings
            .set_key(File::Appearance, &paths, "Theme", "light")
            .unwrap();
        assert_eq!(Settings::load(&paths).0.appearance.mode.key(), "light");
        assert!(
            settings
                .set_key(File::Appearance, &paths, "Theme", "rose")
                .unwrap_err()
                .is_none()
        );
        assert!(
            settings
                .set_key(File::Touchpad, &paths, "speed", "1\nx")
                .unwrap_err()
                .is_none()
        );
        assert_eq!(Settings::load(&paths).0.appearance.mode.key(), "light");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
