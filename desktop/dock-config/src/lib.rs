//! The dock's configuration file, `$XDG_CONFIG_HOME/dock/dock.conf`: reading, checking and
//! writing. A library of its own, so that the dock and the settings application (Control Room,
//! D-26) read and write the same file the same way.
//!
//! ```text
//! [Dock]
//! Pinned=evidence;sherlock;org.kde.konsole;app.zen_browser.zen;
//! ShowSherlock=true
//! IconTheme=breeze
//! IconSize=52
//! Magnification=true
//! MagnificationFactor=1.7
//! AutoHide=smart
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

pub mod stack;

const GROUP: &str = "Dock";
const PINNED: &str = "Pinned";
const ICON_THEME: &str = "IconTheme";
const ICON_SIZE: &str = "IconSize";
const MAGNIFICATION: &str = "Magnification";
const FACTOR: &str = "MagnificationFactor";
const AUTO_HIDE: &str = "AutoHide";
const POSITION: &str = "Position";
const SHOW_SHERLOCK: &str = "ShowSherlock";
const STACK: &str = "Stack";
const STACK_SORT: &str = "StackSort";
const STACK_DISPLAY: &str = "StackDisplay";
const STACK_VIEW: &str = "StackView";

/// The place of Sherlock in the pinned list. Without it, Sherlock is in the middle of the
/// pinned applications, as before it could be moved.
pub const SHERLOCK: &str = "sherlock";

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
    Never,
    /// Hidden until the pointer touches its edge.
    Always,
    /// Hidden only while a maximized or fullscreen window is on its screen.
    #[default]
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
    /// Sherlock is in the dock: at its place in `pinned`, or in their middle without it.
    pub sherlock: bool,
    /// The downloads stack: in the dock or not, its order, its icon, how it opens (D-21k2).
    pub stack: stack::Stack,
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
            auto_hide: AutoHide::Smart,
            position: Position::Bottom,
            sherlock: true,
            stack: stack::Stack::default(),
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
        let sherlock = word(
            &file,
            SHOW_SHERLOCK,
            &[(true, "true"), (false, "false")],
            defaults.sherlock,
            &mut problems,
        );
        let stack = stack::Stack {
            shown: word(
                &file,
                STACK,
                &[(true, "true"), (false, "false")],
                defaults.stack.shown,
                &mut problems,
            ),
            sort: word(
                &file,
                STACK_SORT,
                &stack::Sort::ALL.map(|s| (s, s.key())),
                defaults.stack.sort,
                &mut problems,
            ),
            display: word(
                &file,
                STACK_DISPLAY,
                &stack::Display::ALL.map(|d| (d, d.key())),
                defaults.stack.display,
                &mut problems,
            ),
            view: word(
                &file,
                STACK_VIEW,
                &stack::View::ALL.map(|v| (v, v.key())),
                defaults.stack.view,
                &mut problems,
            ),
        };
        let config = Self {
            stack,
            pinned,
            icon_theme: file.get(GROUP, ICON_THEME).filter(|t| !t.is_empty()),
            icon_size,
            magnification,
            magnification_factor,
            auto_hide,
            position,
            sherlock,
            file,
        };
        (config, problems)
    }

    /// Reads the file at `path`; an absent file is the default configuration. A file that is not
    /// a regular file, or larger than `MAX_SETTINGS_SIZE`, is an `InvalidData` error.
    pub fn load(path: &Path) -> io::Result<(Self, Vec<Problem>)> {
        match mime_apps::file::read_bounded(path, mime_apps::file::MAX_SETTINGS_SIZE) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok((Self::default(), Vec::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// The file's text with the current settings: the text read, where only the keys whose
    /// value changed since are set again. Comments, other groups, unknown keys and their order
    /// stay, and a key the file got wrong stays as written until it is changed: nothing that
    /// was not touched moves.
    pub fn to_text(&self) -> String {
        let mut file = self.file.clone();
        let (read, _) = Self::parse(&file.to_text());
        if self.pinned != read.pinned
            && let Some(pinned) = &self.pinned
        {
            let list: String = pinned.iter().map(|id| format!("{id};")).collect();
            file.set(GROUP, PINNED, &list);
        }
        if self.icon_theme != read.icon_theme
            && let Some(theme) = &self.icon_theme
        {
            file.set(GROUP, ICON_THEME, theme);
        }
        let mut put = |key: &str, value: String, before: String| {
            if value != before {
                file.set(GROUP, key, &value);
            }
        };
        put(
            ICON_SIZE,
            format_number(self.icon_size),
            format_number(read.icon_size),
        );
        put(
            MAGNIFICATION,
            self.magnification.to_string(),
            read.magnification.to_string(),
        );
        put(
            FACTOR,
            format_number(self.magnification_factor),
            format_number(read.magnification_factor),
        );
        put(
            AUTO_HIDE,
            self.auto_hide.key().to_owned(),
            read.auto_hide.key().to_owned(),
        );
        put(
            POSITION,
            self.position.key().to_owned(),
            read.position.key().to_owned(),
        );
        put(
            SHOW_SHERLOCK,
            self.sherlock.to_string(),
            read.sherlock.to_string(),
        );
        put(
            STACK,
            self.stack.shown.to_string(),
            read.stack.shown.to_string(),
        );
        let (now, then) = (self.stack, read.stack);
        put(STACK_SORT, now.sort.key().into(), then.sort.key().into());
        put(
            STACK_DISPLAY,
            now.display.key().into(),
            then.display.key().into(),
        );
        put(STACK_VIEW, now.view.key().into(), then.view.key().into());
        file.to_text()
    }

    /// Reads the file at `path` again, changes it with `change` and writes it back: a change
    /// made since by another program (Control Room) is kept, and a file that cannot be read
    /// (not text, no permission) is never written over. Returns the configuration written.
    pub fn update(path: &Path, change: impl FnOnce(&mut Self)) -> io::Result<Self> {
        let (mut config, _) = Self::load(path)?;
        change(&mut config);
        config.save(path)?;
        Ok(config)
    }

    /// Writes the file at `path` with `mime_apps::write_atomically`: whole or not at all, its
    /// permissions and its links kept (a dangling link too), the folder flushed after.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        mime_apps::write_atomically(path, &self.to_text())
    }
}

/// Moves `id` in the pinned list so that it comes just before `before`, or last when `before`
/// is `None` or not in the list: the drag and drop of the dock, and the same order in the
/// settings. Returns whether the order changed; an `id` that is not pinned changes nothing.
pub fn move_pinned(pinned: &mut Vec<String>, id: &str, before: Option<&str>) -> bool {
    let Some(from) = pinned.iter().position(|p| p == id) else {
        return false;
    };
    let previous = pinned.clone();
    let moved = pinned.remove(from);
    let to = before
        .and_then(|b| pinned.iter().position(|p| p == b))
        .unwrap_or(pinned.len());
    pinned.insert(to, moved);
    *pinned != previous
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
    fn the_stack_settings_are_read_and_written_back() {
        let (config, problems) = DockConfig::parse(
            "[Dock]\nStack=false\nStackSort=kind\nStackDisplay=folder\nStackView=nope\n",
        );
        assert!(!config.stack.shown);
        assert_eq!(config.stack.sort, stack::Sort::Kind);
        assert_eq!(config.stack.display, stack::Display::Folder);
        assert_eq!(
            config.stack.view,
            stack::View::Automatic,
            "a wrong word: the default"
        );
        assert_eq!(problems.len(), 1);
        let mut changed = config.clone();
        changed.stack.view = stack::View::Fan;
        changed.stack.shown = true;
        let (back, _) = DockConfig::parse(&changed.to_text());
        assert_eq!(back.stack.view, stack::View::Fan);
        assert!(back.stack.shown);
        assert_eq!(back.stack.sort, stack::Sort::Kind);
        // Untouched, the defaults are not written.
        assert!(!DockConfig::default().to_text().contains("Stack"));
    }

    #[test]
    fn a_pinned_application_moves_before_another() {
        let list = |s: &str| -> Vec<String> { s.split(' ').map(String::from).collect() };
        let mut pinned = list("a b c d");
        assert!(move_pinned(&mut pinned, "a", Some("d")));
        assert_eq!(pinned, list("b c a d"));
        assert!(move_pinned(&mut pinned, "d", Some("b")));
        assert_eq!(pinned, list("d b c a"));
        assert!(move_pinned(&mut pinned, "d", None), "last");
        assert_eq!(pinned, list("b c a d"));
        // In place already, or not pinned: nothing changes.
        assert!(!move_pinned(&mut pinned, "c", Some("a")));
        assert!(!move_pinned(&mut pinned, "z", Some("a")));
        assert_eq!(pinned, list("b c a d"));
    }

    #[test]
    fn sherlock_keeps_its_place_in_the_pinned_list() {
        // An old file, without the token: nothing about Sherlock, which stays in the middle.
        let old = "[Dock]\nPinned=a;b;\n";
        let (config, problems) = DockConfig::parse(old);
        assert!(problems.is_empty() && config.sherlock);
        assert_eq!(config.pinned, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(config.to_text(), old, "read and written back unchanged");
        // Moved, then taken out of the dock.
        let (mut config, _) = DockConfig::parse("[Dock]\nPinned=sherlock;a;b;\n");
        assert_eq!(
            config.pinned.as_ref().map(|p| p[0].as_str()),
            Some(SHERLOCK)
        );
        config.pinned = Some(vec!["a".to_owned(), "b".to_owned()]);
        config.sherlock = false;
        let text = config.to_text();
        assert_eq!(text, "[Dock]\nPinned=a;b;\nShowSherlock=false\n");
        assert!(!DockConfig::parse(&text).0.sherlock);
        assert_eq!(DockConfig::parse("[Dock]\nShowSherlock=no\n").1.len(), 1);
    }

    #[test]
    fn every_setting_is_read() {
        let (config, problems) = DockConfig::parse(
            "[Dock]\nPinned=a.desktop;b;\nIconTheme=breeze\nIconSize=64\nMagnification=false\n\
             MagnificationFactor=1.5\nAutoHide=Always\nPosition=left\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.pinned, Some(vec!["a".to_owned(), "b".to_owned()]));
        assert_eq!(config.icon_theme.as_deref(), Some("breeze"));
        assert_eq!(config.icon_size, 64.0);
        assert!(!config.magnification);
        assert_eq!(config.magnification_factor, 1.5);
        assert_eq!(config.auto_hide, AutoHide::Always);
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
        assert_eq!(config.auto_hide, AutoHide::Smart);
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
        let left: Vec<_> = std::fs::read_dir(dir.join("dock"))
            .expect("listed")
            .filter_map(|e| e.ok().map(|e| e.file_name()))
            .collect();
        assert_eq!(
            left,
            [std::ffi::OsString::from("dock.conf")],
            "no temporary file left"
        );
    }

    #[test]
    fn a_link_is_kept_and_its_target_written() {
        let dir = std::env::temp_dir().join(format!("dock-config-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dotfiles")).expect("directory");
        std::fs::create_dir_all(dir.join("dock")).expect("directory");
        let path = dir.join("dock/dock.conf");
        std::os::unix::fs::symlink("../dotfiles/dock.conf", &path).expect("link");
        let config = DockConfig {
            position: Position::Top,
            ..DockConfig::default()
        };
        config.save(&path).expect("written");
        assert!(std::fs::symlink_metadata(&path).expect("link").is_symlink());
        let target = std::fs::read_to_string(dir.join("dotfiles/dock.conf")).expect("target");
        assert_eq!(target, "[Dock]\nPosition=top\n");
    }

    #[test]
    fn an_unreadable_file_is_never_written_over() {
        let dir = std::env::temp_dir().join(format!("dock-config-update-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("dock.conf");
        let pin = |config: &mut DockConfig| config.pinned = Some(vec![String::from("kitty")]);
        // Not UTF-8: the change is refused and the bytes stay as they were.
        let bytes = b"[Dock]\nIconTheme=caf\xe9\nIconSize=64\n";
        std::fs::write(&path, bytes).expect("write");
        assert!(DockConfig::update(&path, pin).is_err());
        assert_eq!(std::fs::read(&path).expect("read"), bytes);
        // Readable: only the list changes, what another program wrote since stays.
        std::fs::write(&path, "# mine\n[Dock]\nIconSize=64\nPosition=left\n").expect("write");
        let written = DockConfig::update(&path, pin).expect("update");
        assert_eq!(written.position, Position::Left);
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "# mine\n[Dock]\nIconSize=64\nPosition=left\nPinned=kitty;\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saves_at_the_same_time_do_not_mix() {
        let dir = std::env::temp_dir().join(format!("dock-config-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock/dock.conf");
        let writers: Vec<_> = Position::ALL
            .into_iter()
            .map(|position| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let config = DockConfig {
                        position,
                        ..DockConfig::default()
                    };
                    for _ in 0..20 {
                        config.save(&path).expect("written");
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().expect("writer");
        }
        // One whole file from one of them, never a mix, and no temporary file left.
        let (config, problems) = DockConfig::load(&path).expect("read");
        assert!(problems.is_empty());
        assert!(Position::ALL.contains(&config.position));
        assert_eq!(
            std::fs::read_dir(dir.join("dock")).expect("listed").count(),
            1
        );
    }
}
