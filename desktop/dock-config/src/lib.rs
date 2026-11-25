//! The dock's configuration file, `$XDG_CONFIG_HOME/dock/dock.conf`: reading, checking and
//! writing. A library of its own, so that the dock and the settings application (Control Room,
//! D-26) read and write the same file the same way. Standard library only.
//!
//! ```text
//! [Dock]
//! Pinned=evidence;org.kde.konsole;app.zen_browser.zen;
//! IconTheme=breeze
//! IconSize=52
//! Magnification=true
//! MagnificationFactor=1.7
//! AutoHide=never
//! Position=bottom
//! ```
//!
//! Every key is optional. A value that cannot be read, or is out of its bounds, gives the
//! default and a `Problem` that the caller logs: a bad file never stops the dock. See
//! `docs/dock.md`.

pub mod keyfile;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use sherlock_ui::{dock, metrics};

pub use keyfile::KeyFile;

const GROUP: &str = "Dock";
const PINNED: &str = "Pinned";
const ICON_THEME: &str = "IconTheme";
const ICON_SIZE: &str = "IconSize";
const MAGNIFICATION: &str = "Magnification";
const FACTOR: &str = "MagnificationFactor";
const AUTO_HIDE: &str = "AutoHide";
const POSITION: &str = "Position";

/// Edge of the screen the dock sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Position {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

impl Position {
    pub const ALL: [Self; 4] = [Self::Bottom, Self::Top, Self::Left, Self::Right];

    /// The word written in the file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// On the left or right edge: the icons run top to bottom.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

/// When the dock hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoHide {
    /// Always shown, with its exclusive zone.
    #[default]
    Never,
    /// Hidden until the pointer touches its edge.
    Always,
    /// Hidden only while a window overlaps it.
    Smart,
}

impl AutoHide {
    pub const ALL: [Self; 3] = [Self::Never, Self::Always, Self::Smart];

    /// The word written in the file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Always => "always",
            Self::Smart => "smart",
        }
    }
}

/// A value of the file that was not taken: the default is used instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub key: &'static str,
    pub value: String,
    pub expected: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}={} ignored, expected {}",
            self.key, self.value, self.expected
        )
    }
}

/// The dock settings, each with its default.
#[derive(Debug, Clone, PartialEq)]
pub struct DockConfig {
    /// Desktop file ids of the pinned applications, `None` when the file does not say.
    pub pinned: Option<Vec<String>>,
    /// Icon theme, `None` to follow the GTK setting.
    pub icon_theme: Option<String>,
    /// Side of an icon at rest, from `dock::ICON_SIZE_MIN` to `ICON_SIZE_MAX`.
    pub icon_size: f32,
    pub magnification: bool,
    /// From `dock::MAGNIFICATION_MIN` to `MAGNIFICATION_MAX`.
    pub magnification_factor: f32,
    pub auto_hide: AutoHide,
    pub position: Position,
    /// The file as read, so that writing it back keeps the keys this version does not know.
    file: KeyFile,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            pinned: None,
            icon_theme: None,
            icon_size: metrics::DOCK_ICON,
            magnification: true,
            magnification_factor: metrics::DOCK_MAGNIFICATION,
            auto_hide: AutoHide::Never,
            position: Position::Bottom,
            file: KeyFile::default(),
        }
    }
}

impl DockConfig {
    /// Reads `text`, the content of the file. Values that cannot be taken are listed.
    pub fn parse(text: &str) -> (Self, Vec<Problem>) {
        let file = KeyFile::parse(text);
        let mut problems = Vec::new();
        let defaults = Self::default();
        let pinned = file.get(GROUP, PINNED).map(|_| {
            file.list(GROUP, PINNED)
                .into_iter()
                .map(|id| id.strip_suffix(".desktop").unwrap_or(&id).to_owned())
                .collect()
        });
        let icon_size = number(
            &file,
            ICON_SIZE,
            (dock::ICON_SIZE_MIN, dock::ICON_SIZE_MAX),
            defaults.icon_size,
            &mut problems,
        );
        let magnification_factor = number(
            &file,
            FACTOR,
            (dock::MAGNIFICATION_MIN, dock::MAGNIFICATION_MAX),
            defaults.magnification_factor,
            &mut problems,
        );
        let magnification = word(
            &file,
            MAGNIFICATION,
            &[(true, "true"), (false, "false")],
            defaults.magnification,
            &mut problems,
        );
        let auto_hide = word(
            &file,
            AUTO_HIDE,
            &AutoHide::ALL.map(|a| (a, a.key())),
            defaults.auto_hide,
            &mut problems,
        );
        let position = word(
            &file,
            POSITION,
            &Position::ALL.map(|p| (p, p.key())),
            defaults.position,
            &mut problems,
        );
        let config = Self {
            pinned,
            icon_theme: file.get(GROUP, ICON_THEME).filter(|t| !t.is_empty()),
            icon_size,
            magnification,
            magnification_factor,
            auto_hide,
            position,
            file,
        };
        (config, problems)
    }

    /// Reads the file at `path`; an absent file is the default configuration.
    pub fn load(path: &Path) -> io::Result<(Self, Vec<Problem>)> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok((Self::default(), Vec::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// The file's text with the current settings. Keys left at their default and never written
    /// stay absent; the pinned list is written once it is known.
    pub fn to_text(&self) -> String {
        let mut file = self.file.clone();
        if let Some(pinned) = &self.pinned {
            file.set(
                GROUP,
                PINNED,
                pinned.iter().map(|id| format!("{id};")).collect(),
            );
        }
        let defaults = Self::default();
        let mut put = |key: &str, value: String, default: String| {
            if file.get(GROUP, key).is_some() || value != default {
                file.set(GROUP, key, value);
            }
        };
        put(
            ICON_SIZE,
            format_number(self.icon_size),
            format_number(defaults.icon_size),
        );
        put(
            MAGNIFICATION,
            self.magnification.to_string(),
            defaults.magnification.to_string(),
        );
        put(
            FACTOR,
            format_number(self.magnification_factor),
            format_number(defaults.magnification_factor),
        );
        put(
            AUTO_HIDE,
            self.auto_hide.key().to_owned(),
            defaults.auto_hide.key().to_owned(),
        );
        put(
            POSITION,
            self.position.key().to_owned(),
            defaults.position.key().to_owned(),
        );
        if let Some(theme) = &self.icon_theme {
            file.set(GROUP, ICON_THEME, theme.clone());
        }
        file.to_text()
    }

    /// Writes the file at `path`, creating its directory. The text goes to a temporary file
    /// first, then replaces the old one: a reader never sees half a file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension("conf.tmp");
        std::fs::write(&temporary, self.to_text())?;
        std::fs::rename(&temporary, path)
    }
}

/// A number of `key` within `bounds`, or `default` and a problem.
fn number(
    file: &KeyFile,
    key: &'static str,
    (low, high): (f32, f32),
    default: f32,
    problems: &mut Vec<Problem>,
) -> f32 {
    let Some(raw) = file.get(GROUP, key) else {
        return default;
    };
    match raw.trim().parse::<f32>() {
        Ok(value) if (low..=high).contains(&value) => value,
        _ => {
            problems.push(Problem {
                key,
                value: raw,
                expected: format!("a number from {low} to {high}"),
            });
            default
        }
    }
}

/// One of the `words` of `key`, ignoring case, or `default` and a problem.
fn word<T: Copy>(
    file: &KeyFile,
    key: &'static str,
    words: &[(T, &str)],
    default: T,
    problems: &mut Vec<Problem>,
) -> T {
    let Some(raw) = file.get(GROUP, key) else {
        return default;
    };
    match words
        .iter()
        .find(|(_, w)| w.eq_ignore_ascii_case(raw.trim()))
    {
        Some((value, _)) => *value,
        None => {
            let expected: Vec<&str> = words.iter().map(|(_, w)| *w).collect();
            problems.push(Problem {
                key,
                value: raw,
                expected: format!("one of {}", expected.join(", ")),
            });
            default
        }
    }
}

/// A number without useless decimals: `52`, `1.7`.
fn format_number(value: f32) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// `$XDG_CONFIG_HOME`, `~/.config` by default.
pub fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// Path of the file: `$XDG_CONFIG_HOME/dock/dock.conf`.
pub fn path() -> Option<PathBuf> {
    Some(config_home()?.join("dock/dock.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_is_read() {
        let (config, problems) = DockConfig::parse(
            "[Dock]\nPinned=a.desktop;b;\nIconTheme=breeze\nIconSize=64\nMagnification=false\n\
             MagnificationFactor=1.5\nAutoHide=Smart\nPosition=left\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.pinned, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(config.icon_theme.as_deref(), Some("breeze"));
        assert_eq!(config.icon_size, 64.0);
        assert!(!config.magnification);
        assert_eq!(config.magnification_factor, 1.5);
        assert_eq!(config.auto_hide, AutoHide::Smart);
        assert_eq!(config.position, Position::Left);
    }

    #[test]
    fn without_a_file_everything_is_default() {
        let (config, problems) = DockConfig::parse("");
        assert!(problems.is_empty());
        assert_eq!(config, DockConfig::default());
        assert_eq!(config.icon_size, 52.0);
        assert_eq!(config.magnification_factor, 1.7);
        assert_eq!(config.position, Position::Bottom);
        let path = std::env::temp_dir().join(format!("dock-absent-{}.conf", std::process::id()));
        assert_eq!(
            DockConfig::load(&path).map(|(c, _)| c).ok(),
            Some(DockConfig::default())
        );
        // An empty list is a choice: Sherlock alone, not the defaults.
        assert_eq!(
            DockConfig::parse("[Dock]\nPinned=\n").0.pinned,
            Some(Vec::new())
        );
    }

    #[test]
    fn invalid_values_give_the_default_and_a_problem() {
        let (config, problems) = DockConfig::parse(
            "[Dock]\nIconSize=500\nMagnification=oui\nMagnificationFactor=abc\n\
             AutoHide=sometimes\nPosition=middle\n",
        );
        assert_eq!(config.icon_size, 52.0);
        assert!(config.magnification);
        assert_eq!(config.magnification_factor, 1.7);
        assert_eq!(config.auto_hide, AutoHide::Never);
        assert_eq!(config.position, Position::Bottom);
        let keys: Vec<&str> = problems.iter().map(|p| p.key).collect();
        assert_eq!(
            keys,
            [
                "IconSize",
                "MagnificationFactor",
                "Magnification",
                "AutoHide",
                "Position"
            ]
        );
        assert_eq!(
            problems[0].to_string(),
            "IconSize=500 ignored, expected a number from 32 to 96"
        );
        // The bounds themselves are taken.
        let (config, problems) = DockConfig::parse("[Dock]\nIconSize=32\nMagnificationFactor=2\n");
        assert!(problems.is_empty());
        assert_eq!((config.icon_size, config.magnification_factor), (32.0, 2.0));
    }

    #[test]
    fn writing_keeps_unknown_keys_and_reads_back() {
        let (mut config, _) = DockConfig::parse("[Dock]\nPinned=a;\nFuture=1\n[Other]\nX=y\n");
        config.pinned = Some(vec!["b".to_owned(), "a".to_owned()]);
        config.position = Position::Right;
        config.magnification_factor = 1.25;
        let text = config.to_text();
        assert_eq!(
            text,
            "[Dock]\nPinned=b;a;\nFuture=1\nMagnificationFactor=1.25\nPosition=right\n\
             [Other]\nX=y\n"
        );
        let (read, problems) = DockConfig::parse(&text);
        assert!(problems.is_empty());
        assert_eq!(read.pinned, config.pinned);
        assert_eq!(read.position, Position::Right);
        assert_eq!(read.magnification_factor, 1.25);

        let dir = std::env::temp_dir().join(format!("dock-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock/dock.conf");
        config.save(&path).expect("written");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), text);
        assert!(!path.with_extension("conf.tmp").exists());
    }
}
