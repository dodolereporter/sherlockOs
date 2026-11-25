//! Emplacements favoris de la barre latérale : dossier personnel et dossiers de
//! l'utilisateur, lus dans `user-dirs.dirs` (xdg-user-dirs) pour suivre la langue du poste.

use std::path::{Path, PathBuf};

use sherlock_ui::icon::Icon;
use sherlock_ui::texts::evidence as texts;

/// Un emplacement de la barre latérale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub label: &'static str,
    pub icon: Icon,
    pub path: PathBuf,
}

/// Les favoris, dans l'ordre de la planche. `user_dirs` est le contenu de
/// `~/.config/user-dirs.dirs`, s'il existe ; sans lui, les noms français par défaut.
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
        },
        Place {
            label: texts::DESKTOP,
            icon: Icon::Desktop,
            path: lookup("XDG_DESKTOP_DIR", texts::DESKTOP),
        },
        Place {
            label: texts::DOCUMENTS,
            icon: Icon::Document,
            path: lookup("XDG_DOCUMENTS_DIR", texts::DOCUMENTS),
        },
        Place {
            label: texts::DOWNLOADS,
            icon: Icon::Download,
            path: lookup("XDG_DOWNLOAD_DIR", texts::DOWNLOADS),
        },
        Place {
            label: texts::PICTURES,
            icon: Icon::Image,
            path: lookup("XDG_PICTURES_DIR", texts::PICTURES),
        },
    ]
}

/// Lit `XDG_..._DIR="$HOME/Bureau"` dans `user-dirs.dirs`.
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
    // Un dossier réglé sur le dossier personnel lui-même est désactivé, selon xdg-user-dirs.
    (path.is_absolute() && path != home).then_some(path)
}

/// Contenu de `user-dirs.dirs`, dans `$XDG_CONFIG_HOME` ou `~/.config`.
pub fn read_user_dirs(home: &Path) -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(config.join("user-dirs.dirs")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_dossiers_suivent_user_dirs() {
        let home = Path::new("/home/dodo");
        let content = "# commentaire\nXDG_DESKTOP_DIR=\"$HOME/Bureau\"\n\
                       XDG_DOWNLOAD_DIR=\"/data/dl\"\nXDG_PICTURES_DIR=\"$HOME/\"\n";
        let places = favorites(home, Some(content));
        let paths: Vec<_> = places.iter().map(|p| p.path.as_path()).collect();
        assert_eq!(
            paths,
            [
                home,
                Path::new("/home/dodo/Bureau"),
                Path::new("/home/dodo/Documents"),
                Path::new("/data/dl"),
                Path::new("/home/dodo/Images"),
            ]
        );
        assert_eq!(places[0].label, "Accueil");
    }
}
