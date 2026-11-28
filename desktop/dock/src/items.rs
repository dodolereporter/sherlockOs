//! The icons of the dock: pinned applications and Sherlock, at its place among them or in their
//! middle, then, after a separator, the open applications that are not pinned.

use std::collections::HashMap;
use std::path::Path;

use dock_config::SHERLOCK;
use iced::widget::svg;
use sherlock_ui::app_icon::AppIcon;
use sherlock_ui::dock;

use crate::entry::{Catalog, DesktopEntry};
use crate::icons::IconLookup;
use crate::windows::Windows;

/// What an icon stands for.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// The button that opens the applications view (D-41), first, as on macOS.
    Applications,
    /// Sherlock, at its place among the pinned applications (`dock_config::SHERLOCK`), or in
    /// their middle. It has no window to open yet.
    Sherlock,
    App(Box<DesktopEntry>),
    /// Open windows whose application has no desktop entry: their `app_id`.
    Window(String),
    /// The trash, last (lot D-21j): full or empty.
    Trash(bool),
    /// A stack, before the trash (lot D-21k): its folder.
    Stack(std::path::PathBuf),
    /// The hole of an application dragged over the dock (lot D-41d).
    Hole,
}

/// What an icon stands for, by identity rather than by place: the place of an icon changes
/// when windows open and close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Applications,
    Sherlock,
    /// A desktop file id.
    App(String),
    /// An `app_id` without desktop entry.
    Window(String),
    Trash,
    Stack,
    Hole,
}

/// How an icon is drawn.
#[derive(Debug, Clone)]
pub enum Glyph {
    /// Nothing: the hole an application dragged from the Applications view opens (lot D-41d).
    None,
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
    /// In the pinned list (Sherlock and the « Applications » button count as pinned).
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

    /// The hole an application dragged over the dock opens among the pinned ones (lot D-41d).
    pub fn hole() -> Self {
        Self {
            kind: Kind::Hole,
            name: String::new(),
            glyph: Glyph::None,
            pinned: true,
        }
    }

    /// The « Applications » button: always there, first.
    pub fn applications() -> Self {
        Self {
            kind: Kind::Applications,
            name: String::from(dock::text::APPLICATIONS),
            glyph: Glyph::File(svg::Handle::from_memory(
                dock::applications_icon().into_bytes(),
            )),
            pinned: true,
        }
    }

    /// The trash, full or empty: its tile from sherlock-ui.
    pub fn trash(full: bool) -> Self {
        Self {
            kind: Kind::Trash(full),
            name: String::from(dock::text::TRASH),
            glyph: Glyph::File(svg::Handle::from_memory(
                dock::trash_icon(full).into_bytes(),
            )),
            pinned: false,
        }
    }

    /// A stack named `name`, the downloads folder: the theme's `folder-download`, else the
    /// tile from sherlock-ui.
    pub fn stack(folder: std::path::PathBuf, name: &str, themed: Option<&Path>) -> Self {
        let glyph = match themed {
            Some(path) => Glyph::File(icon_handle(path)),
            None => Glyph::File(svg::Handle::from_memory(dock::stack_icon().into_bytes())),
        };
        Self {
            kind: Kind::Stack(folder),
            name: name.to_owned(),
            glyph,
            pinned: false,
        }
    }

    pub fn target(&self) -> Target {
        match &self.kind {
            Kind::Trash(_) => Target::Trash,
            Kind::Stack(_) => Target::Stack,
            Kind::Applications => Target::Applications,
            Kind::Sherlock => Target::Sherlock,
            Kind::App(entry) => Target::App(entry.id.clone()),
            Kind::Window(app_id) => Target::Window(app_id.clone()),
            Kind::Hole => Target::Hole,
        }
    }

    /// Its name in the pinned list: a desktop file id, or Sherlock's token. The « Applications »
    /// button and the open applications have none.
    pub fn pin_id(&self) -> Option<&str> {
        match &self.kind {
            Kind::App(entry) => Some(entry.id.as_str()),
            Kind::Sherlock => Some(SHERLOCK),
            Kind::Hole => Some(HOLE),
            Kind::Applications | Kind::Window(_) | Kind::Trash(_) | Kind::Stack(_) => None,
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
            Kind::Sherlock | Kind::Applications | Kind::Trash(_) | Kind::Stack(_) | Kind::Hole => {
                false
            }
        }
    }
}

/// The token of the hole in the pinned list while an application is dragged over the dock
/// (lot D-41d): never a desktop id, and never written to the file.
pub const HOLE: &str = "<hole>";

/// Builds the icons, and finds their glyphs once: `glyphs` keeps them by desktop file id.
pub struct Builder<'a> {
    /// The « Applications » button first.
    pub applications: bool,
    /// Sherlock in the dock (`DockConfig::sherlock`).
    pub sherlock: bool,
    /// The trash at the end, full (`Some(true)`) or empty; none without a trash folder.
    pub trash: Option<bool>,
    /// The downloads stack before it: its folder and its name (lot D-21k).
    pub stack: Option<(&'a Path, &'a str)>,
    pub catalog: &'a Catalog,
    pub lookup: &'a IconLookup,
    /// Size the icon files are looked up for: a fully magnified icon.
    pub icon_size: u32,
    pub glyphs: &'a mut HashMap<String, Glyph>,
}

impl Builder<'_> {
    /// The « Applications » button, the icons for `pinned` with Sherlock at its token, or in
    /// their middle without one, then the open applications of `windows` that none of them owns,
    /// in the order their windows came. Returns them with the index of the first unpinned one,
    /// where the separator goes.
    pub fn build(
        &mut self,
        pinned: &[String],
        windows: Option<&Windows>,
    ) -> (Vec<Item>, Option<usize>) {
        let mut sherlock = self.sherlock.then(Item::sherlock);
        let mut items: Vec<Item> = Vec::with_capacity(pinned.len() + 1);
        for id in pinned {
            if id == SHERLOCK {
                items.extend(sherlock.take());
                continue;
            }
            if id == HOLE {
                items.push(Item::hole());
                continue;
            }
            match self.catalog.get(id) {
                Some(entry) => items.push(self.app(entry, true)),
                None => tracing::warn!(id, "pinned application not installed"),
            }
        }
        // A list written before Sherlock could move: in the middle, as it was then.
        if let Some(sherlock) = sherlock {
            items.insert(items.len().div_ceil(2), sherlock);
        }
        if self.applications {
            items.insert(0, Item::applications());
        }
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
        // The stack, then the trash last, after the separator, as on macOS (lots D-21j, k).
        if let Some((folder, name)) = self.stack {
            let themed = self.lookup.find("folder-download", self.icon_size);
            items.push(Item::stack(folder.to_path_buf(), name, themed.as_deref()));
        }
        if let Some(full) = self.trash {
            items.push(Item::trash(full));
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

/// Puts Sherlock's token in `pinned` where `shown` (the pinned icons in their order on screen,
/// Sherlock's token among them) has it: before the application shown after it, or last. The
/// applications that are not installed keep their places, and the order on screen does not
/// change.
pub fn place_sherlock(pinned: &mut Vec<String>, shown: &[&str]) {
    pinned.retain(|p| p != SHERLOCK);
    let next = shown
        .iter()
        .position(|p| *p == SHERLOCK)
        .and_then(|at| shown.get(at + 1));
    let at = next
        .and_then(|next| pinned.iter().position(|p| p == next))
        .unwrap_or(pinned.len());
    pinned.insert(at, SHERLOCK.to_owned());
}

fn letter(name: &str) -> Glyph {
    Glyph::Letter(name.chars().next().map_or('?', |c| c.to_ascii_uppercase()))
}

/// An icon file, SVG or PNG: `sherlock_ui::raster` reads it, with its bounds, and draws it.
fn icon_handle(path: &Path) -> svg::Handle {
    svg::Handle::from_path(path)
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
            applications: true,
            sherlock: true,
            trash: None,
            stack: None,
            catalog: &catalog,
            lookup: &lookup,
            icon_size: 88,
            glyphs: &mut glyphs,
        };
        let pinned = ["org.example.Files".to_owned(), "example-browser".to_owned()];
        let (items, separator) = builder.build(&pinned, None);
        assert_eq!(
            names(&items),
            ["Applications", "Fichiers", "Sherlock", "Example Browser"]
        );
        assert_eq!(separator, None);
        // The browser is open but pinned; the terminal is known; `mystery` has no entry.
        let Placeholder(windows) =
            Placeholder::new("ExampleBrowser,terminal,mystery,org.example.terminal", None);
        let (items, separator) = builder.build(&pinned, Some(&windows));
        assert_eq!(
            names(&items),
            [
                "Applications",
                "Fichiers",
                "Sherlock",
                "Example Browser",
                "Terminal d'exemple",
                "mystery"
            ]
        );
        assert_eq!(separator, Some(4));
        assert!(items[..4].iter().all(|i| i.pinned));
        assert!(items[4..].iter().all(|i| !i.pinned));
        assert_eq!(items[5].kind, Kind::Window("mystery".to_owned()));
        assert!(matches!(items[5].glyph, Glyph::Letter('M')));
        // At its token; out of the dock, the token is ignored.
        let moved = ["sherlock".to_owned(), "org.example.Files".to_owned()];
        assert_eq!(
            names(&builder.build(&moved, None).0),
            ["Applications", "Sherlock", "Fichiers"]
        );
        builder.sherlock = false;
        assert_eq!(
            names(&builder.build(&moved, None).0),
            ["Applications", "Fichiers"]
        );
        assert_eq!(
            names(&builder.build(&pinned, None).0),
            ["Applications", "Fichiers", "Example Browser"]
        );
        // Glyphs are looked up once per application.
        assert_eq!(glyphs.len(), 3);
    }

    #[test]
    fn the_trash_comes_last_after_the_separator() {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data");
        let catalog = Catalog::load(std::slice::from_ref(&data));
        let lookup = IconLookup::new(&[data], None, None);
        let mut glyphs = HashMap::new();
        let mut builder = Builder {
            applications: false,
            sherlock: false,
            trash: Some(true),
            stack: Some((Path::new("/home/me/Downloads"), "Downloads")),
            catalog: &catalog,
            lookup: &lookup,
            icon_size: 88,
            glyphs: &mut glyphs,
        };
        let pinned = ["org.example.Files".to_owned()];
        let (items, separator) = builder.build(&pinned, None);
        assert_eq!(names(&items), ["Fichiers", "Downloads", "Corbeille"]);
        assert_eq!(separator, Some(1));
        assert_eq!(items[2].kind, Kind::Trash(true));
        assert_eq!(items[2].target(), Target::Trash);
        assert_eq!(items[1].target(), Target::Stack);
        assert!(items[1].pin_id().is_none() && !items[2].pinned);
        let Placeholder(windows) = Placeholder::new("terminal", None);
        let (items, separator) = builder.build(&pinned, Some(&windows));
        assert_eq!(
            names(&items),
            ["Fichiers", "Terminal d'exemple", "Downloads", "Corbeille"]
        );
        assert_eq!(separator, Some(1));
    }

    #[test]
    fn sherlock_is_written_where_it_is_shown() {
        let list = |ids: &[&str]| -> Vec<String> { ids.iter().map(|i| (*i).to_owned()).collect() };
        // `gone` is not installed: it is not shown, and stays where it is.
        let mut pinned = list(&["a", "gone", "b", "c"]);
        place_sherlock(&mut pinned, &["a", "sherlock", "b", "c"]);
        assert_eq!(pinned, list(&["a", "gone", "sherlock", "b", "c"]));
        place_sherlock(&mut pinned, &["a", "b", "c", "sherlock"]);
        assert_eq!(pinned, list(&["a", "gone", "b", "c", "sherlock"]));
        place_sherlock(&mut pinned, &["sherlock", "a", "b", "c"]);
        assert_eq!(pinned, list(&["sherlock", "a", "gone", "b", "c"]));
    }
}
