//! Dock configuration: `$XDG_CONFIG_HOME/dock/dock.conf`, a key file.
//!
//! ```text
//! [Dock]
//! Pinned=evidence;org.kde.konsole;app.zen_browser.zen;
//! IconTheme=breeze
//! ```
//!
//! `Pinned` lists desktop file ids, left to right; Sherlock is not in it, the dock puts it in
//! the middle. Without the file, the dock picks installed applications (`defaults`) and writes
//! nothing until the list changes.

use std::io;
use std::path::{Path, PathBuf};

use crate::entry::{Catalog, DesktopEntry, KeyFile};

const GROUP: &str = "Dock";
const PINNED: &str = "Pinned";
const ICON_THEME: &str = "IconTheme";

/// Configuration of the dock.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    /// Desktop file ids of the pinned applications, `None` when the file does not say.
    pub pinned: Option<Vec<String>>,
    /// Icon theme, `None` to follow the GTK setting.
    pub icon_theme: Option<String>,
    /// The file as read, so that writing it back keeps what the dock does not know.
    file: KeyFile,
}

impl Config {
    /// Reads `text`, the content of the configuration file.
    pub fn parse(text: &str) -> Self {
        let file = KeyFile::parse(text);
        let pinned = file.get(GROUP, PINNED).map(|_| {
            file.list(GROUP, PINNED)
                .into_iter()
                .map(|id| id.strip_suffix(".desktop").unwrap_or(&id).to_owned())
                .collect()
        });
        Self {
            pinned,
            icon_theme: file.get(GROUP, ICON_THEME).filter(|t| !t.is_empty()),
            file,
        }
    }

    /// Reads the file at `path`; an absent file is an empty configuration.
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    /// Writes the configuration with `pinned` as the pinned list, creating the directory.
    pub fn save(&mut self, path: &Path, pinned: &[String]) -> io::Result<()> {
        let list: String = pinned.iter().map(|id| format!("{id};")).collect();
        self.file.set(GROUP, PINNED, list);
        self.pinned = Some(pinned.to_vec());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.file.to_text())
    }
}

/// Path of the configuration file: `$XDG_CONFIG_HOME/dock/dock.conf`, `~/.config` by default.
pub fn path() -> Option<PathBuf> {
    Some(config_home()?.join("dock/dock.conf"))
}

/// `$XDG_CONFIG_HOME`, `~/.config` by default.
pub fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// A role filled by default, when the application is installed.
struct Role {
    /// Default application of this type in `mimeapps.list`, if any.
    mime: Option<&'static str>,
    /// Applications preferred for the role, in order: ours first.
    preferred: &'static [&'static str],
    /// Category that any application of the role declares.
    category: Option<&'static str>,
}

/// Files (Evidence, ours), terminal, editor, browser, games: the left part of the Dock board,
/// with what is installed.
const ROLES: [Role; 5] = [
    Role {
        mime: Some("inode/directory"),
        preferred: &["evidence"],
        category: Some("FileManager"),
    },
    Role {
        mime: None,
        preferred: &["org.kde.konsole", "org.gnome.Console", "org.gnome.Terminal"],
        category: Some("TerminalEmulator"),
    },
    Role {
        mime: Some("text/plain"),
        preferred: &[],
        category: Some("TextEditor"),
    },
    Role {
        mime: Some("x-scheme-handler/https"),
        preferred: &[],
        category: Some("WebBrowser"),
    },
    Role {
        mime: None,
        preferred: &["steam", "com.valvesoftware.Steam"],
        category: None,
    },
];

/// Pinned applications without configuration: for each role, the default application of its
/// type (`mimeapps`), else a preferred one, else the first one of its category. Only what is
/// installed and shown in menus; an application fills one role at most.
pub fn defaults(catalog: &Catalog, mimeapps: &KeyFile) -> Vec<String> {
    let usable = |id: &str| catalog.get(id).filter(|e| !e.hidden);
    let mut pinned: Vec<String> = Vec::new();
    for role in &ROLES {
        let from_mime = role
            .mime
            .into_iter()
            .flat_map(|mime| mimeapps.list("Default Applications", mime))
            .find_map(|id| usable(&id));
        let preferred = || role.preferred.iter().find_map(|id| usable(id));
        let by_category = || {
            role.category.and_then(|category| {
                catalog
                    .entries
                    .iter()
                    .find(|e| !e.hidden && e.categories.iter().any(|c| c == category))
            })
        };
        let chosen: Option<&DesktopEntry> = from_mime.or_else(preferred).or_else(by_category);
        if let Some(entry) = chosen
            && !pinned.contains(&entry.id)
        {
            pinned.push(entry.id.clone());
        }
    }
    pinned
}

/// The user's `mimeapps.list` (`$XDG_CONFIG_HOME/mimeapps.list`), empty if absent.
pub fn mimeapps(config_home: &Path) -> KeyFile {
    std::fs::read_to_string(config_home.join("mimeapps.list"))
        .map(|text| KeyFile::parse(&text))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    #[test]
    fn the_configuration_lists_the_pinned_applications() {
        let config = Config::load(&fixtures().join("config/dock/dock.conf")).expect("readable");
        assert_eq!(
            config.pinned.as_deref(),
            Some(&["org.example.Files".to_owned(), "example-browser".to_owned()][..])
        );
        assert_eq!(config.icon_theme.as_deref(), Some("Exemple"));
        let absent = Config::load(&fixtures().join("config/dock/absent.conf")).expect("absent");
        assert_eq!(absent, Config::default());
        // An empty list is a choice: no application, not the defaults.
        assert_eq!(Config::parse("[Dock]\nPinned=\n").pinned, Some(Vec::new()));
    }

    #[test]
    fn saving_keeps_the_other_keys() {
        let dir = std::env::temp_dir().join(format!("dock-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock/dock.conf");
        let mut config = Config::parse("[Dock]\nPinned=a;b;\nIconTheme=Exemple\n");
        config.save(&path, &["b".to_owned()]).expect("written");
        let text = std::fs::read_to_string(&path).expect("read back");
        assert_eq!(text, "[Dock]\nPinned=b;\nIconTheme=Exemple\n");
        assert_eq!(Config::parse(&text).pinned, Some(vec!["b".to_owned()]));
    }

    #[test]
    fn the_defaults_take_what_is_installed() {
        let catalog = Catalog::load(&[fixtures().join("data")]);
        // Without mimeapps: the first of each category; no editor in the fixtures but the
        // pixmaps one, which declares TextEditor; no games.
        let pinned = defaults(&catalog, &KeyFile::default());
        assert_eq!(
            pinned,
            [
                "org.example.Files",
                "org.example.Terminal",
                "example-editor",
                "example-browser"
            ]
        );
        // The default browser of mimeapps comes first, when it is installed.
        let mime = mimeapps(&fixtures().join("config"));
        let pinned = defaults(&catalog, &mime);
        assert_eq!(pinned[3], "org.example.OtherBrowser");
        // A hidden application is never picked.
        assert!(!pinned.iter().any(|id| id == "example-hidden"));
    }
}
