//! Sidebar favourites: home folder and user folders, read from `user-dirs.dirs`
//! (xdg-user-dirs) so they follow the machine's language.

use std::path::{Path, PathBuf};

use sherlock_ui::icon::Icon;
use sherlock_ui::texts::evidence as texts;

/// A sidebar place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub label: &'static str,
    pub icon: Icon,
    pub path: PathBuf,
    /// Hidden when the folder does not exist, instead of shown disabled.
    pub optional: bool,
}

/// The favourites, in the mock-up's order. `user_dirs` is the content of
/// `~/.config/user-dirs.dirs`, if it exists; without it, the default French names.
pub fn favorites(home: &Path, user_dirs: Option<&str>) -> Vec<Place> {
    let lookup = |key: &str, fallback: &str| {
        user_dirs
            .and_then(|content| user_dir(content, key, home))
            .unwrap_or_else(|| home.join(fallback))
    };
    vec![
        Place {
            label: texts::HOME,
            icon: Icon::Home,
            path: home.to_path_buf(),
            optional: false,
        },
        Place {
            label: texts::DESKTOP,
            icon: Icon::Desktop,
            path: lookup("XDG_DESKTOP_DIR", texts::DESKTOP),
            optional: false,
        },
        Place {
            label: texts::DOCUMENTS,
            icon: Icon::Document,
            path: lookup("XDG_DOCUMENTS_DIR", texts::DOCUMENTS),
            optional: false,
        },
        Place {
            label: texts::DOWNLOADS,
            icon: Icon::Download,
            path: lookup("XDG_DOWNLOAD_DIR", texts::DOWNLOADS),
            optional: false,
        },
        // The design board's « Projets »: a plain folder of the home, shown only when it exists.
        Place {
            label: texts::PROJECTS,
            icon: Icon::Folder,
            path: home.join(texts::PROJECTS),
            optional: true,
        },
        Place {
            label: texts::PICTURES,
            icon: Icon::Image,
            path: lookup("XDG_PICTURES_DIR", texts::PICTURES),
            optional: false,
        },
    ]
}

/// Reads `XDG_..._DIR="$HOME/Bureau"` from `user-dirs.dirs`.
fn user_dir(content: &str, key: &str, home: &Path) -> Option<PathBuf> {
    let value = content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))?
        .trim()
        .trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    // A folder set to the home folder itself is disabled, as xdg-user-dirs specifies.
    (path.is_absolute() && path != home).then_some(path)
}

/// Content of `user-dirs.dirs`, in `$XDG_CONFIG_HOME` or `~/.config`.
pub fn read_user_dirs(home: &Path) -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    crate::ops::read_small_text(&config.join("user-dirs.dirs"), crate::ops::SMALL_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_follow_user_dirs() {
        let home = Path::new("/home/utilisateur");
        let content = "# commentaire\nXDG_DESKTOP_DIR=\"$HOME/Bureau\"\n\
                       XDG_DOWNLOAD_DIR=\"/data/dl\"\nXDG_PICTURES_DIR=\"$HOME/\"\n";
        let places = favorites(home, Some(content));
        let paths: Vec<_> = places.iter().map(|p| p.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                home,
                Path::new("/home/utilisateur/Bureau"),
                Path::new("/home/utilisateur/Documents"),
                Path::new("/data/dl"),
                Path::new("/home/utilisateur/Projets"),
                Path::new("/home/utilisateur/Images"),
            ]
        );
        assert_eq!(places[0].label, "Accueil");
    }
}
