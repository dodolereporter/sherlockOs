//! Default pinned applications, when the configuration does not list them. The configuration
//! file itself is read and written by the `dock-config` crate.

use std::path::Path;

use crate::entry::{Catalog, DesktopEntry, KeyFile};

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
    let usable = |id: &str| catalog.get(id).filter(|e| !e.no_display);
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
                    .find(|e| !e.no_display && e.categories.iter().any(|c| c == category))
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
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
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
