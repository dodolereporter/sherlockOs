//! The settings files Control Room reads and writes. They are the files the other programs
//! already read and follow live: the appearance (every program), the dock, the wallpaper, the
//! touchpad and the pointer (the compositor). Nothing here depends on iced, so that each file
//! is written and read back in a temporary folder by the tests.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use appearance_config::AppearanceConfig;
use desktop_config::{clipboard, lock, pointer, touchpad, wallpaper};
use dock_config::DockConfig;

/// Where each file is, and where the installed wallpapers are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub appearance: PathBuf,
    pub dock: PathBuf,
    pub wallpaper: PathBuf,
    pub touchpad: PathBuf,
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
            File::Pointer => &self.pointer,
            File::Lock => &self.lock,
            File::Clipboard => &self.clipboard,
        }
    }
}

impl File {
    pub const ALL: [Self; 7] = [
        Self::Appearance,
        Self::Dock,
        Self::Wallpaper,
        Self::Touchpad,
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
                Ok(text) => settings.read(file, path, &text),
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "settings file not changeable");
                    locked.push(file);
                }
            }
        }
        (settings, locked)
    }

    /// Takes the settings of `file` from its text; what is not understood goes to the log.
    fn read(&mut self, file: File, path: &Path, text: &str) {
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
        for problem in problems {
            tracing::warn!(%problem, path = %path.display(), "settings line left aside");
        }
    }

    /// `text` with the keys of `file` whose value these settings change set again.
    fn write_into(&self, file: File, text: &str) -> String {
        match file {
            File::Appearance => self.appearance.write_into(text),
            // The dock keeps the text it was read from.
            File::Dock => self.dock.to_text(),
            File::Wallpaper => self.wallpaper.write_into(text),
            File::Touchpad => self.touchpad.write_into(text),
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
        fresh.read(file, path, &text);
        change(&mut fresh);
        let written = fresh.write_into(file, &text);
        if written != text {
            mime_apps::write_atomically(path, &written)?;
        }
        // Read back from what was written: the dock keeps that text for its next change.
        self.read(file, path, &written);
        Ok(())
    }
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
