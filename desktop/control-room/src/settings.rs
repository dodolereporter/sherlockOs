//! The settings files Control Room reads and writes. They are the files the other programs
//! already read and follow live: the appearance (every program), the dock, the wallpaper, the
//! touchpad and the pointer (the compositor). Nothing here depends on iced, so that each file
//! is written and read back in a temporary folder by the tests.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use appearance_config::AppearanceConfig;
use desktop_config::{pointer, touchpad, wallpaper};
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
            lock: desktop.join(crate::lock::FILE),
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
            lock: desktop_config::config_dir(var)?.join(crate::lock::FILE),
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
}

/// Every setting Control Room shows.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub appearance: AppearanceConfig,
    pub dock: DockConfig,
    pub wallpaper: wallpaper::Config,
    pub touchpad: touchpad::Config,
    pub pointer: pointer::Config,
    pub lock: crate::lock::Config,
}

impl Settings {
    /// Reads every file. A missing file is its default; a file that cannot be read is its
    /// default too, and goes to the log: changing a setting then writes it whole.
    pub fn load(paths: &Paths) -> Self {
        fn logged<T: Default, E: std::fmt::Display>(path: &Path, read: Result<T, E>) -> T {
            read.unwrap_or_else(|error| {
                tracing::warn!(%error, path = %path.display(), "settings file not read");
                T::default()
            })
        }
        fn problems<T, P: std::fmt::Display>(
            path: &Path,
            read: io::Result<(T, Vec<P>)>,
        ) -> io::Result<T> {
            read.map(|(config, problems)| {
                for problem in problems {
                    tracing::warn!(%problem, path = %path.display(), "settings file");
                }
                config
            })
        }
        Self {
            appearance: logged(
                &paths.appearance,
                problems(&paths.appearance, AppearanceConfig::load(&paths.appearance)),
            ),
            dock: logged(
                &paths.dock,
                problems(&paths.dock, DockConfig::load(&paths.dock)),
            ),
            wallpaper: logged(&paths.wallpaper, wallpaper::Config::read(&paths.wallpaper)),
            touchpad: logged(&paths.touchpad, touchpad::Config::read(&paths.touchpad)),
            pointer: logged(&paths.pointer, pointer::Config::read(&paths.pointer)),
            lock: crate::lock::Config::read(&paths.lock),
        }
    }

    /// Writes `file` with the settings it holds.
    pub fn save(&self, file: File, paths: &Paths) -> io::Result<()> {
        match file {
            File::Appearance => write(&paths.appearance, &self.appearance.to_text()),
            File::Dock => self.dock.save(&paths.dock),
            File::Wallpaper => write(&paths.wallpaper, &self.wallpaper.to_string()),
            File::Touchpad => write(&paths.touchpad, &self.touchpad.to_string()),
            File::Pointer => write(&paths.pointer, &self.pointer.to_string()),
            File::Lock => write(&paths.lock, &self.lock.to_text()),
        }
    }
}

/// Writes `text` to `path` whole or not at all: to a temporary file beside it, flushed to the
/// disk, then renamed over it. A program that reads the file never sees half of it, and a
/// symbolic link (dotfiles kept in a repository) keeps pointing at the written file.
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
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
