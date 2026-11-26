//! The icons of the dock: pinned applications with Sherlock in their middle, then, after a
//! separator, the open applications that are not pinned.

use std::collections::HashMap;
use std::path::Path;

use iced::widget::svg;
use sherlock_ui::app_icon::AppIcon;
use sherlock_ui::dock;

use crate::entry::{Catalog, DesktopEntry};
use crate::icons::IconLookup;
use crate::windows::Windows;

/// What an icon stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Sherlock, in the middle of the pinned applications. It has no window to open yet.
    Sherlock,
    App(Box<DesktopEntry>),
    /// Open windows whose application has no desktop entry: their `app_id`.
    Window(String),
}

/// What an icon stands for, by identity rather than by place: the place of an icon changes
/// when windows open and close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Sherlock,
    /// A desktop file id.
    App(String),
    /// An `app_id` without desktop entry.
    Window(String),
}

/// How an icon is drawn.
#[derive(Debug, Clone)]
pub enum Glyph {
    /// An icon file, or Sherlock's tile. A PNG goes through an SVG that embeds it: tiny-skia's
    /// SVG renderer draws both, without iced's `image` feature and the decoders it would add.
    File(svg::Handle),
    /// No icon found: the initial of the name on a tile.
    Letter(char),
}

/// An icon of the dock.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub glyph: Glyph,
    /// In the pinned list (Sherlock counts as pinned: it is always there).
    pub pinned: bool,
}

impl Item {
    pub fn sherlock() -> Self {
        Self {
            kind: Kind::Sherlock,
            name: String::from(dock::text::SHERLOCK),
            // The author's tile, from sherlock-ui rather than the icon theme: Sherlock has no
            // desktop entry, and no theme can replace it.
            glyph: Glyph::File(svg::Handle::from_memory(AppIcon::Sherlock.svg().as_bytes())),
            pinned: true,
        }
    }

    pub fn target(&self) -> Target {
        match &self.kind {
            Kind::Sherlock => Target::Sherlock,
            Kind::App(entry) => Target::App(entry.id.clone()),
            Kind::Window(app_id) => Target::Window(app_id.clone()),
        }
    }

    pub fn entry(&self) -> Option<&DesktopEntry> {
        match &self.kind {
            Kind::App(entry) => Some(entry),
            _ => None,
        }
    }

    /// Whether a window with this `app_id` belongs to the icon.
    pub fn owns(&self, app_id: &str) -> bool {
        match &self.kind {
            Kind::App(entry) => entry.owns(app_id),
            Kind::Window(id) => id == app_id,
            Kind::Sherlock => false,
        }
    }
}

/// Builds the icons, and finds their glyphs once: `glyphs` keeps them by desktop file id.
pub struct Builder<'a> {
    pub catalog: &'a Catalog,
    pub lookup: &'a IconLookup,
    /// Size the icon files are looked up for: a fully magnified icon.
    pub icon_size: u32,
    pub glyphs: &'a mut HashMap<String, Glyph>,
}

impl Builder<'_> {
    /// The icons for `pinned`, then the open applications of `windows` that none of them owns,
    /// in the order their windows came. Returns them with the index of the first unpinned one,
    /// where the separator goes.
    pub fn build(
        &mut self,
        pinned: &[String],
        windows: Option<&Windows>,
    ) -> (Vec<Item>, Option<usize>) {
        let mut items: Vec<Item> = pinned
            .iter()
            .filter_map(|id| match self.catalog.get(id) {
                Some(entry) => Some(self.app(entry, true)),
                None => {
                    tracing::warn!(id, "pinned application not installed");
                    None
                }
            })
            .collect();
        items.insert(items.len().div_ceil(2), Item::sherlock());
        let first_open = items.len();
        for window in windows.map(|w| w.list.as_slice()).unwrap_or_default() {
            if items.iter().any(|item| item.owns(&window.app_id)) {
                continue;
            }
            let item = match self.catalog.entries.iter().find(|e| e.owns(&window.app_id)) {
                Some(entry) => self.app(entry, false),
                None => Item {
                    kind: Kind::Window(window.app_id.clone()),
                    name: window.app_id.clone(),
                    glyph: letter(&window.app_id),
                    pinned: false,
                },
            };
            items.push(item);
        }
        let separator = (items.len() > first_open).then_some(first_open);
        (items, separator)
    }

    fn app(&mut self, entry: &DesktopEntry, pinned: bool) -> Item {
        let glyph = match self.glyphs.get(&entry.id) {
            Some(glyph) => glyph.clone(),
            None => {
                let file = entry
                    .icon
                    .as_deref()
                    .and_then(|name| self.lookup.find(name, self.icon_size));
                let glyph = match file {
                    Some(path) => Glyph::File(icon_handle(&path)),
                    None => letter(&entry.name),
                };
                self.glyphs.insert(entry.id.clone(), glyph.clone());
                glyph
            }
        };
        Item {
            name: entry.name.clone(),
            kind: Kind::App(Box::new(entry.clone())),
            glyph,
            pinned,
        }
    }
}

fn letter(name: &str) -> Glyph {
    Glyph::Letter(name.chars().next().map_or('?', |c| c.to_ascii_uppercase()))
}

/// An SVG file as it is; any other image inside an SVG.
fn icon_handle(path: &Path) -> svg::Handle {
    if path.extension().is_some_and(|e| e == "svg") {
        return svg::Handle::from_path(path);
    }
    let href = path
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"{href}\"/></svg>"
    );
    svg::Handle::from_memory(document.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::Placeholder;
    use std::path::PathBuf;

    fn names(items: &[Item]) -> Vec<&str> {
        items.iter().map(|i| i.name.as_str()).collect()
    }

    #[test]
    fn open_applications_come_after_the_separator() {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data");
        let catalog = Catalog::load(std::slice::from_ref(&data));
        let lookup = IconLookup::new(&[data], None, None);
        let mut glyphs = HashMap::new();
        let mut builder = Builder {
            catalog: &catalog,
            lookup: &lookup,
            icon_size: 88,
            glyphs: &mut glyphs,
        };
        let pinned = ["org.example.Files".to_owned(), "example-browser".to_owned()];
        let (items, separator) = builder.build(&pinned, None);
        assert_eq!(names(&items), ["Fichiers", "Sherlock", "Example Browser"]);
        assert_eq!(separator, None);
        // The browser is open but pinned; the terminal is known; `mystery` has no entry.
        let Placeholder(windows) =
            Placeholder::new("ExampleBrowser,terminal,mystery,org.example.terminal", None);
        let (items, separator) = builder.build(&pinned, Some(&windows));
        assert_eq!(
            names(&items),
            [
                "Fichiers",
                "Sherlock",
                "Example Browser",
                "Terminal d'exemple",
                "mystery"
            ]
        );
        assert_eq!(separator, Some(3));
        assert!(items[..3].iter().all(|i| i.pinned));
        assert!(items[3..].iter().all(|i| !i.pinned));
        assert_eq!(items[4].kind, Kind::Window("mystery".to_owned()));
        assert!(matches!(items[4].glyph, Glyph::Letter('M')));
        // Glyphs are looked up once per application.
        assert_eq!(glyphs.len(), 3);
    }
}
