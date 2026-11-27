//! Icon lookup by name, after the freedesktop Icon Theme specification: in the chosen theme and
//! the themes it inherits from, then in `hicolor`, then in the base folders and `pixmaps`. In
//! each theme an exact size (and scale) wins, else the closest one. No `icon-theme.cache`: the
//! files are looked for directly. See `docs/icon-theme.md`.
//!
//! The launcher and the dock use it; Evidence (`desktop/evidence/src/icons.rs`) still carries
//! its own copy and is meant to move onto this crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// File formats looked for, in the order of the specification. XPM is left out: nothing in
/// the desktop draws it.
pub const EXTENSIONS: [&str; 2] = ["png", "svg"];

/// The theme every lookup ends with.
pub const FALLBACK_THEME: &str = "hicolor";

/// The largest key file read. `hicolor`'s `index.theme` lists every size and context and already
/// holds 55 KiB; the biggest themes stay under a few hundred.
pub const MAX_KEY_FILE_SIZE: u64 = 512 * 1024;

/// A key file (`index.theme`, `settings.ini`, `kdeglobals`): `[Group]` headers, `Key=Value`
/// lines, `#` comments. The first value of a key wins.
#[derive(Debug, Clone, Default)]
struct KeyFile(HashMap<String, HashMap<String, String>>);

impl KeyFile {
    fn parse(text: &str) -> Self {
        let mut groups: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current: Option<String> = None;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = Some(name.to_owned());
            } else if let (Some(group), Some((key, value))) = (&current, line.split_once('=')) {
                groups
                    .entry(group.clone())
                    .or_default()
                    .entry(key.trim().to_owned())
                    .or_insert_with(|| value.trim().to_owned());
            }
        }
        Self(groups)
    }

    /// Reads a key file of `MAX_KEY_FILE_SIZE` at most (`index.theme`, `settings.ini`,
    /// `kdeglobals`): a link to a device or a huge file is ignored like an absent one.
    fn read(path: &Path) -> Option<Self> {
        mime_apps::file::read_bounded(path, MAX_KEY_FILE_SIZE)
            .ok()
            .map(|t| Self::parse(&t))
    }

    fn get(&self, group: &str, key: &str) -> Option<&str> {
        self.0.get(group)?.get(key).map(String::as_str)
    }

    /// A comma separated list, without empty items.
    fn list(&self, group: &str, key: &str) -> Vec<String> {
        self.get(group, key)
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fixed,
    Scalable,
    Threshold,
}

/// One folder of a theme, as `index.theme` describes it.
#[derive(Debug, Clone)]
struct Dir {
    path: String,
    scale: u32,
    /// Sizes it covers, in its own pixels.
    low: u32,
    high: u32,
    context: Option<String>,
}

impl Dir {
    fn read(index: &KeyFile, path: &str) -> Option<Self> {
        let number = |key: &str| index.get(path, key).and_then(|v| v.parse::<u32>().ok());
        let size = number("Size")?;
        // `Threshold` is the default type, with a threshold of 2.
        let kind = match index.get(path, "Type") {
            Some("Fixed") => Kind::Fixed,
            Some("Scalable") => Kind::Scalable,
            _ => Kind::Threshold,
        };
        let (low, high) = match kind {
            Kind::Fixed => (size, size),
            Kind::Scalable => (
                number("MinSize").unwrap_or(size),
                number("MaxSize").unwrap_or(size),
            ),
            Kind::Threshold => {
                let threshold = number("Threshold").unwrap_or(2);
                (
                    size.saturating_sub(threshold),
                    size.saturating_add(threshold),
                )
            }
        };
        Some(Self {
            path: path.to_owned(),
            scale: number("Scale").unwrap_or(1).max(1),
            low,
            high,
            context: index.get(path, "Context").map(str::to_owned),
        })
    }

    /// `DirectoryMatchesSize`: the same scale, and the size in its range.
    fn matches(&self, size: u32, scale: u32) -> bool {
        self.scale == scale && (self.low..=self.high).contains(&size)
    }

    /// `DirectorySizeDistance`, in device pixels: 0 when the range covers the size.
    fn distance(&self, size: u32, scale: u32) -> u32 {
        let wanted = size.saturating_mul(scale);
        let low = self.low.saturating_mul(self.scale);
        let high = self.high.saturating_mul(self.scale);
        if wanted < low {
            low - wanted
        } else {
            wanted.saturating_sub(high)
        }
    }
}

#[derive(Debug, Clone)]
struct Theme {
    name: String,
    dirs: Vec<Dir>,
}

/// Where icons are looked for, read once: the themes' indexes are parsed here, the icon files
/// are looked for at each `find`.
#[derive(Debug, Clone, Default)]
pub struct Lookup {
    /// `$HOME/.icons`, then `<data dir>/icons` for each data directory.
    bases: Vec<PathBuf>,
    /// The last resort: the base folders themselves, then `<data dir>/pixmaps`.
    fallback: Vec<PathBuf>,
    /// The chosen theme and those it inherits from, depth first, `hicolor` last.
    themes: Vec<Theme>,
    /// Only the folders of this `Context`, when set.
    context: Option<String>,
}

impl Lookup {
    /// Looks in `data_dirs` (most important first, `$XDG_DATA_HOME` included) and in
    /// `home/.icons`, theme `theme` first. A theme that is not installed is skipped.
    pub fn new(data_dirs: &[PathBuf], home: Option<&Path>, theme: Option<&str>) -> Self {
        let bases: Vec<PathBuf> = home
            .map(|h| h.join(".icons"))
            .into_iter()
            .chain(data_dirs.iter().map(|d| d.join("icons")))
            .collect();
        let fallback = bases
            .iter()
            .cloned()
            .chain(data_dirs.iter().map(|d| d.join("pixmaps")))
            .collect();
        let mut lookup = Self {
            bases,
            fallback,
            themes: Vec::new(),
            context: None,
        };
        let mut seen: Vec<String> = Vec::new();
        let mut pending: Vec<String> = theme.map(str::to_owned).into_iter().collect();
        while let Some(name) = pending.pop() {
            // `hicolor` comes last, whoever inherits from it; a loop of themes stops here.
            if name == FALLBACK_THEME || seen.contains(&name) || !is_inside(&name) {
                continue;
            }
            seen.push(name.clone());
            let Some(index) = lookup.index(&name) else {
                continue;
            };
            // Depth first, in the order written.
            pending.extend(index.list("Icon Theme", "Inherits").into_iter().rev());
            lookup.themes.push(Theme::read(name, &index));
        }
        if let Some(index) = lookup.index(FALLBACK_THEME) {
            lookup
                .themes
                .push(Theme::read(FALLBACK_THEME.to_owned(), &index));
        }
        lookup
    }

    /// Only looks in the folders of this `Context` (`Applications`, `MimeTypes`…): far fewer
    /// files to try for an application icon.
    pub fn context(mut self, context: &str) -> Self {
        self.context = Some(context.to_owned());
        self
    }

    /// The themes searched, in order.
    pub fn themes(&self) -> Vec<&str> {
        self.themes.iter().map(|t| t.name.as_str()).collect()
    }

    fn index(&self, theme: &str) -> Option<KeyFile> {
        self.bases
            .iter()
            .find_map(|base| KeyFile::read(&base.join(theme).join("index.theme")))
    }

    /// The file of icon `name` for `size` pixels at `scale`. An absolute path is taken as it
    /// is; a name written with its extension (`kitty.png`) is looked up without it.
    pub fn find(&self, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        let path = Path::new(name);
        if path.is_absolute() {
            return path.is_file().then(|| path.to_path_buf());
        }
        let name = EXTENSIONS
            .iter()
            .chain(&["xpm"])
            .find_map(|ext| name.strip_suffix(&format!(".{ext}")))
            .unwrap_or(name);
        if name.is_empty() || name.contains('/') {
            return None;
        }
        let scale = scale.max(1);
        self.themes
            .iter()
            .find_map(|theme| self.find_in_theme(theme, name, size, scale))
            .or_else(|| self.fallback.iter().find_map(|dir| file_in(dir, name)))
    }

    /// `LookupIcon`: the first folder of an exact size, else the closest one; between equals,
    /// the first folder written.
    fn find_in_theme(&self, theme: &Theme, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        let mut closest: Option<(u32, PathBuf)> = None;
        let dirs = theme.dirs.iter().filter(|dir| {
            self.context
                .as_deref()
                .is_none_or(|c| dir.context.as_deref() == Some(c))
        });
        for dir in dirs {
            let exact = dir.matches(size, scale);
            let distance = dir.distance(size, scale);
            if !exact && closest.as_ref().is_some_and(|(d, _)| *d <= distance) {
                continue;
            }
            let found = self
                .bases
                .iter()
                .find_map(|base| file_in(&base.join(&theme.name).join(&dir.path), name));
            let Some(found) = found else {
                continue;
            };
            if exact {
                return Some(found);
            }
            closest = Some((distance, found));
        }
        closest.map(|(_, path)| path)
    }
}

impl Theme {
    fn read(name: String, index: &KeyFile) -> Self {
        let mut paths = index.list("Icon Theme", "Directories");
        paths.extend(index.list("Icon Theme", "ScaledDirectories"));
        let dirs = paths
            .iter()
            .filter(|p| is_inside(p))
            .filter_map(|p| Dir::read(index, p))
            .collect();
        Self { name, dirs }
    }
}

/// Whether a theme name or a theme folder stays under its base folder: no `..`, not absolute.
fn is_inside(relative: &str) -> bool {
    !relative.is_empty() && !relative.contains("..") && !relative.starts_with('/')
}

/// `dir/name.<ext>` for the first extension that exists.
fn file_in(dir: &Path, name: &str) -> Option<PathBuf> {
    EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|p| p.is_file())
}

/// The icon theme the user chose: GTK's (`gtk-icon-theme-name` in
/// `$XDG_CONFIG_HOME/gtk-3.0/settings.ini`, then `gtk-4.0`), which Plasma writes too, else
/// Plasma's own (`Theme` of `[Icons]` in `kdeglobals`). `None` when none is set.
pub fn user_theme(config_home: &Path) -> Option<String> {
    let gtk = ["gtk-3.0", "gtk-4.0"].iter().find_map(|version| {
        KeyFile::read(&config_home.join(version).join("settings.ini"))?
            .get("Settings", "gtk-icon-theme-name")
            .map(str::to_owned)
    });
    gtk.or_else(|| {
        KeyFile::read(&config_home.join("kdeglobals"))?
            .get("Icons", "Theme")
            .map(str::to_owned)
    })
    .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary folder, removed at the end of the test.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("icon-theme-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temporary folder");
            Self(path)
        }

        fn write(&self, relative: &str, content: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
            std::fs::write(&path, content).expect("written");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A data folder with a fake theme « Exemple » inheriting « Parent », and hicolor.
    fn fake_data(tmp: &TempDir) -> PathBuf {
        tmp.write(
            "share/icons/Exemple/index.theme",
            "[Icon Theme]\nName=Exemple\nInherits=Parent,hicolor\n\
             Directories=32x32/apps,scalable/apps,48x48/mimetypes\nScaledDirectories=32x32@2/apps\n\
             [32x32/apps]\nSize=32\nType=Fixed\nContext=Applications\n\
             [32x32@2/apps]\nSize=32\nScale=2\nType=Fixed\nContext=Applications\n\
             [scalable/apps]\nSize=64\nMinSize=40\nMaxSize=256\nType=Scalable\nContext=Applications\n\
             [48x48/mimetypes]\nSize=48\nContext=MimeTypes\n",
        );
        tmp.write("share/icons/Exemple/32x32/apps/term.png", "png");
        tmp.write("share/icons/Exemple/32x32@2/apps/term.png", "png");
        tmp.write("share/icons/Exemple/scalable/apps/term.svg", "svg");
        tmp.write("share/icons/Exemple/48x48/mimetypes/text-plain.png", "png");
        tmp.write(
            "share/icons/Parent/index.theme",
            "[Icon Theme]\nInherits=Exemple\nDirectories=apps\n[apps]\nSize=24\nContext=Applications\n",
        );
        tmp.write("share/icons/Parent/apps/editor.png", "png");
        tmp.write(
            "share/icons/hicolor/index.theme",
            "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps,96x96/apps\n\
             [48x48/apps]\nSize=48\nType=Threshold\nContext=Applications\n\
             [96x96/apps]\nSize=96\nType=Threshold\nContext=Applications\n",
        );
        tmp.write("share/icons/hicolor/48x48/apps/browser.png", "png");
        tmp.write("share/icons/hicolor/96x96/apps/browser.png", "png");
        tmp.write("share/icons/hicolor/48x48/apps/term.png", "png");
        tmp.write("share/pixmaps/old-app.svg", "svg");
        tmp.0.join("share")
    }

    fn tail(found: Option<PathBuf>) -> String {
        let found = found.expect("icon found");
        let parts: Vec<String> = found
            .iter()
            .rev()
            .take(4)
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        parts.into_iter().rev().collect::<Vec<_>>().join("/")
    }

    #[test]
    fn the_theme_then_its_parents_then_hicolor_then_pixmaps() {
        let tmp = TempDir::new("order");
        let data = fake_data(&tmp);
        let lookup = Lookup::new(std::slice::from_ref(&data), None, Some("Exemple"));
        // The inheritance loop (Parent inherits Exemple) ends; hicolor comes last.
        assert_eq!(lookup.themes(), ["Exemple", "Parent", "hicolor"]);
        // In the theme: an exact size wins over hicolor's.
        assert_eq!(
            tail(lookup.find("term", 32, 1)),
            "Exemple/32x32/apps/term.png"
        );
        // Only in the parent, only in hicolor, only in pixmaps.
        assert_eq!(
            tail(lookup.find("editor", 32, 1)),
            "icons/Parent/apps/editor.png"
        );
        assert_eq!(
            tail(lookup.find("browser", 48, 1)),
            "hicolor/48x48/apps/browser.png"
        );
        assert_eq!(
            lookup.find("old-app", 32, 1),
            Some(data.join("pixmaps/old-app.svg"))
        );
        assert_eq!(lookup.find("missing", 32, 1), None);
        assert_eq!(lookup.find("", 32, 1), None);
        assert_eq!(lookup.find("../hicolor/48x48/apps/browser", 32, 1), None);
    }

    #[test]
    fn the_size_and_the_scale_choose_the_folder() {
        let tmp = TempDir::new("size");
        let lookup = Lookup::new(&[fake_data(&tmp)], None, Some("Exemple"));
        // Scale 2: the @2 folder matches exactly.
        assert_eq!(
            tail(lookup.find("term", 32, 2)),
            "Exemple/32x32@2/apps/term.png"
        );
        // 100 px: the scalable folder covers it.
        assert_eq!(
            tail(lookup.find("term", 100, 1)),
            "Exemple/scalable/apps/term.svg"
        );
        // In hicolor, the closest: 96 (94 to 98) for 88, 48 (46 to 50) for 40.
        assert_eq!(
            tail(lookup.find("browser", 88, 1)),
            "hicolor/96x96/apps/browser.png"
        );
        assert_eq!(
            tail(lookup.find("browser", 40, 1)),
            "hicolor/48x48/apps/browser.png"
        );
        // Scale 0 is read as 1.
        assert_eq!(
            tail(lookup.find("browser", 48, 0)),
            "hicolor/48x48/apps/browser.png"
        );
    }

    #[test]
    fn a_context_narrows_the_folders() {
        let tmp = TempDir::new("context");
        let data = fake_data(&tmp);
        let all = Lookup::new(std::slice::from_ref(&data), None, Some("Exemple"));
        assert!(all.find("text-plain", 48, 1).is_some());
        let apps = Lookup::new(&[data], None, Some("Exemple")).context("Applications");
        assert_eq!(apps.find("text-plain", 48, 1), None);
        assert!(apps.find("term", 32, 1).is_some());
    }

    #[test]
    fn paths_names_with_extensions_and_missing_themes() {
        let tmp = TempDir::new("paths");
        let data = fake_data(&tmp);
        let lookup = Lookup::new(std::slice::from_ref(&data), None, Some("Absent"));
        assert_eq!(lookup.themes(), ["hicolor"]);
        let absolute = data.join("pixmaps/old-app.svg");
        assert_eq!(
            lookup.find(&absolute.to_string_lossy(), 32, 1),
            Some(absolute)
        );
        assert_eq!(lookup.find("/nonexistent/icon.png", 32, 1), None);
        assert_eq!(
            tail(lookup.find("browser.png", 48, 1)),
            "hicolor/48x48/apps/browser.png"
        );
        // `$HOME/.icons` comes first, and holds themes too.
        let home = tmp.0.join("home");
        let mine = tmp.write("home/.icons/hicolor/48x48/apps/browser.png", "png");
        let lookup = Lookup::new(&[data], Some(&home), None);
        assert_eq!(lookup.find("browser", 48, 1), Some(mine));
        // Nothing installed at all: no theme, nothing found.
        assert!(Lookup::new(&[], None, Some("Exemple")).themes().is_empty());
    }

    #[test]
    fn themes_and_folders_stay_under_the_base_folders() {
        let tmp = TempDir::new("escape");
        let data = fake_data(&tmp);
        // A theme outside `icons/`, reachable by `..`, and one by an absolute path.
        tmp.write(
            "share/outside/index.theme",
            "[Icon Theme]\nDirectories=apps\n[apps]\nSize=32\n",
        );
        tmp.write("share/outside/apps/secret.png", "png");
        let absolute = tmp.0.join("share/outside");
        for name in ["../outside", absolute.to_str().expect("utf-8")] {
            let lookup = Lookup::new(std::slice::from_ref(&data), None, Some(name));
            assert_eq!(lookup.themes(), ["hicolor"], "{name}");
            assert_eq!(lookup.find("secret", 32, 1), None, "{name}");
        }
        // Folders of a theme that climb out of it are left out too.
        tmp.write(
            "share/icons/Climber/index.theme",
            "[Icon Theme]\nDirectories=../../outside/apps,/etc\n\
             [../../outside/apps]\nSize=32\n[/etc]\nSize=32\n",
        );
        let lookup = Lookup::new(&[data], None, Some("Climber"));
        assert_eq!(lookup.themes(), ["Climber", "hicolor"]);
        assert_eq!(lookup.find("secret", 32, 1), None);
    }

    #[test]
    fn an_index_that_never_ends_is_ignored() {
        let tmp = TempDir::new("zero");
        let data = fake_data(&tmp);
        let theme = tmp.0.join("share/icons/Endless");
        std::fs::create_dir_all(&theme).expect("folder");
        std::os::unix::fs::symlink("/dev/zero", theme.join("index.theme")).expect("link");
        let lookup = Lookup::new(&[data], None, Some("Endless"));
        assert_eq!(lookup.themes(), ["hicolor"]);
    }

    #[test]
    fn the_user_theme_is_read_from_gtk_then_plasma() {
        let tmp = TempDir::new("user");
        let config = tmp.0.join("config");
        assert_eq!(user_theme(&config), None);
        tmp.write(
            "config/kdeglobals",
            "[General]\nTheme=no\n[Icons]\nTheme=breeze-dark\n",
        );
        assert_eq!(user_theme(&config).as_deref(), Some("breeze-dark"));
        tmp.write(
            "config/gtk-3.0/settings.ini",
            "# comment\n[Settings]\ngtk-icon-theme-name = Exemple\n",
        );
        assert_eq!(user_theme(&config).as_deref(), Some("Exemple"));
    }
}
