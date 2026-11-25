//! Application icons: lookup in the icon themes, as the Icon Theme specification describes it,
//! then in `pixmaps`. Only what the dock needs: the `apps` icons of one size, no cache file.

use std::path::{Path, PathBuf};

use crate::entry::KeyFile;

/// Formats the dock can draw: both go through the SVG renderer (see `crate::app::Glyph`).
const EXTENSIONS: [&str; 2] = ["svg", "png"];

/// Where icons are looked for.
#[derive(Debug, Clone)]
pub struct IconLookup {
    /// Base directories: `$HOME/.icons`, then `<data dir>/icons` for each data directory.
    bases: Vec<PathBuf>,
    /// `pixmaps` directories, the last resort.
    pixmaps: Vec<PathBuf>,
    /// Themes to search, the chosen one and those it inherits from, `hicolor` last.
    themes: Vec<String>,
}

impl IconLookup {
    /// Looks in `data_dirs`, theme `theme` first (with what it inherits), then `hicolor`.
    pub fn new(data_dirs: &[PathBuf], home: Option<&Path>, theme: Option<&str>) -> Self {
        let bases: Vec<PathBuf> = home
            .map(|h| h.join(".icons"))
            .into_iter()
            .chain(data_dirs.iter().map(|d| d.join("icons")))
            .collect();
        let pixmaps = data_dirs.iter().map(|d| d.join("pixmaps")).collect();
        let mut lookup = Self {
            bases,
            pixmaps,
            themes: Vec::new(),
        };
        let mut pending: Vec<String> = theme.map(str::to_owned).into_iter().collect();
        while let Some(name) = pending.pop() {
            if lookup.themes.contains(&name) || name == "hicolor" {
                continue;
            }
            let inherits = lookup
                .index(&name)
                .map(|index| index.split("Icon Theme", "Inherits", ','))
                .unwrap_or_default();
            lookup.themes.push(name);
            // Depth first, in the order written.
            pending.extend(inherits.into_iter().rev());
        }
        lookup.themes.push(String::from("hicolor"));
        lookup
    }

    /// The themes searched, in order.
    pub fn themes(&self) -> &[String] {
        &self.themes
    }

    fn index(&self, theme: &str) -> Option<KeyFile> {
        self.bases.iter().find_map(|base| {
            let text = std::fs::read_to_string(base.join(theme).join("index.theme")).ok()?;
            Some(KeyFile::parse(&text))
        })
    }

    /// File of icon `name` best fitting `size` pixels. An absolute path is taken as it is.
    pub fn find(&self, name: &str, size: u32) -> Option<PathBuf> {
        let path = Path::new(name);
        if path.is_absolute() {
            return path.is_file().then(|| path.to_path_buf());
        }
        self.themes
            .iter()
            .find_map(|theme| self.find_in_theme(theme, name, size))
            .or_else(|| {
                self.pixmaps
                    .iter()
                    .flat_map(|dir| EXTENSIONS.map(|ext| dir.join(format!("{name}.{ext}"))))
                    .find(|p| p.is_file())
            })
    }

    /// The closest size wins (`DirectorySizeDistance`); between equals, the first directory.
    fn find_in_theme(&self, theme: &str, name: &str, size: u32) -> Option<PathBuf> {
        let index = self.index(theme)?;
        let mut best: Option<(u32, PathBuf)> = None;
        for dir in index.split("Icon Theme", "Directories", ',') {
            let Some(distance) = distance(&index, &dir, size) else {
                continue;
            };
            if best.as_ref().is_some_and(|(d, _)| *d <= distance) {
                continue;
            }
            let found = self
                .bases
                .iter()
                .flat_map(|base| {
                    EXTENSIONS.map(|ext| base.join(theme).join(&dir).join(format!("{name}.{ext}")))
                })
                .find(|p| p.is_file());
            if let Some(found) = found {
                best = Some((distance, found));
            }
        }
        best.map(|(_, path)| path)
    }
}

/// How far the icons of directory `dir` are from `size`, 0 when they fit. `None` for a
/// directory that is not described or does not hold application icons.
fn distance(index: &KeyFile, dir: &str, size: u32) -> Option<u32> {
    let number = |key: &str| {
        index
            .get(dir, key)
            .and_then(|v| v.trim().parse::<u32>().ok())
    };
    if index
        .get(dir, "Context")
        .is_some_and(|c| c != "Applications")
    {
        return None;
    }
    let nominal = number("Size")?;
    let scale = number("Scale").unwrap_or(1).max(1);
    let (low, high) = match index.get(dir, "Type").as_deref() {
        Some("Fixed") => (nominal, nominal),
        Some("Scalable") => (
            number("MinSize").unwrap_or(nominal),
            number("MaxSize").unwrap_or(nominal),
        ),
        // `Threshold` is the default type.
        _ => {
            let threshold = number("Threshold").unwrap_or(2);
            (nominal.saturating_sub(threshold), nominal + threshold)
        }
    };
    let (low, high) = (low * scale, high * scale);
    Some(if size < low {
        low - size
    } else {
        size.saturating_sub(high)
    })
}

/// Icon theme chosen by the user for GTK applications (`gtk-icon-theme-name` in
/// `$XDG_CONFIG_HOME/gtk-3.0/settings.ini`), which Plasma writes too. `None` when unset.
pub fn gtk_theme(config_home: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config_home.join("gtk-3.0/settings.ini")).ok()?;
    KeyFile::parse(&text)
        .get("Settings", "gtk-icon-theme-name")
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data")
    }

    fn file_of(found: Option<PathBuf>) -> String {
        let found = found.expect("icon found");
        let parts: Vec<_> = found.iter().rev().take(3).collect();
        parts
            .into_iter()
            .rev()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/")
    }

    #[test]
    fn the_theme_comes_before_hicolor_and_pixmaps() {
        let lookup = IconLookup::new(&[data()], None, Some("Exemple"));
        assert_eq!(lookup.themes(), ["Exemple", "hicolor"]);
        // In the theme, the scalable icon covers 88.
        assert_eq!(
            file_of(lookup.find("example-terminal", 88)),
            "scalable/apps/example-terminal.svg"
        );
        // Only in hicolor: the closest fixed size, 96 rather than 48.
        assert_eq!(
            file_of(lookup.find("example-browser", 88)),
            "96x96/apps/example-browser.png"
        );
        assert_eq!(
            file_of(lookup.find("example-browser", 40)),
            "48x48/apps/example-browser.png"
        );
        // Only in pixmaps.
        assert_eq!(
            file_of(lookup.find("example-editor", 88)),
            "data/pixmaps/example-editor.png"
        );
        assert_eq!(lookup.find("missing", 88), None);
    }

    #[test]
    fn an_absolute_icon_is_taken_as_it_is() {
        let lookup = IconLookup::new(&[data()], None, None);
        let path = data().join("pixmaps/example-editor.png");
        assert_eq!(lookup.find(&path.to_string_lossy(), 88), Some(path));
        assert_eq!(lookup.find("/nonexistent/icon.png", 88), None);
        assert_eq!(lookup.themes(), ["hicolor"]);
    }

    #[test]
    fn the_gtk_theme_is_read() {
        let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/config");
        assert_eq!(gtk_theme(&config).as_deref(), Some("Exemple"));
        assert_eq!(gtk_theme(&data()), None);
    }
}
