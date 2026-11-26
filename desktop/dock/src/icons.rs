//! Application icons, found by the shared `icon-theme` crate as the Icon Theme specification
//! describes it: the chosen theme and the themes it inherits, then `hicolor`, then `pixmaps`.
//! Only the `Applications` folders are searched.

use std::path::{Path, PathBuf};

use icon_theme::Lookup;
use sherlock_ui::dock;

/// Where icons are looked for.
#[derive(Debug, Clone)]
pub struct IconLookup(Lookup);

impl IconLookup {
    /// Looks in `data_dirs`, theme `theme` first (with what it inherits), then `hicolor`.
    pub fn new(data_dirs: &[PathBuf], home: Option<&Path>, theme: Option<&str>) -> Self {
        Self(Lookup::new(data_dirs, home, theme).context("Applications"))
    }

    /// The themes searched, in order.
    pub fn themes(&self) -> Vec<&str> {
        self.0.themes()
    }

    /// File of icon `name` for an icon of `size` logical pixels. It is looked up for a screen
    /// at scale `dock::ICON_SCALE`: a scalable icon wins whenever the theme has one, and is
    /// drawn at the size and scale of the screen; a fixed one is sharp on a scale 2 screen and
    /// scaled down on the others. An absolute path is taken as it is.
    pub fn find(&self, name: &str, size: u32) -> Option<PathBuf> {
        self.0.find(name, size, dock::ICON_SCALE)
    }
}

/// The icon theme the user chose: GTK's, which Plasma writes too, else Plasma's own. `None`
/// when none is set.
pub fn user_theme(config_home: &Path) -> Option<String> {
    icon_theme::user_theme(config_home)
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
        // Only in hicolor: the closest fixed size at scale 2, 96 rather than 48.
        assert_eq!(
            file_of(lookup.find("example-browser", 44)),
            "96x96/apps/example-browser.png"
        );
        assert_eq!(
            file_of(lookup.find("example-browser", 24)),
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
    fn our_icons_are_found_in_hicolor() {
        // The theme of sherlock-ui installed beside the fixtures, whose hicolor index describes
        // the folders, as the system's does.
        let ours = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sherlock-ui");
        let lookup = IconLookup::new(&[ours, data()], None, Some("Exemple"));
        for icon in sherlock_ui::app_icon::AppIcon::ALL {
            let found = lookup.find(icon.id(), 88);
            assert_eq!(
                file_of(found),
                format!("scalable/apps/{}.svg", icon.id()),
                "{icon:?}"
            );
        }
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
    fn the_user_theme_is_read() {
        let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/config");
        assert_eq!(user_theme(&config).as_deref(), Some("Exemple"));
        assert_eq!(user_theme(&data()), None);
    }
}
