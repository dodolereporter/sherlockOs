//! A stack of the dock (lot D-21k), as on macOS: the downloads folder, between the applications
//! and the trash; a click opens its newest items in a grid over the dock, each opened by its
//! default application, and « Ouvrir dans Evidence ». The folder is found and read on threads of
//! their own, with bounds: never on the interface thread.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dock_config::stack::Sort;
use sherlock_ui::dock::PileCard;

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use sherlock_ui::icon::Icon;

/// Entries of the folder looked at, at most, to find the newest ones.
pub const MAX_LOOKED_AT: usize = 4096;
/// Items the grid shows: four rows of four, the newest first.
pub const MAX_SHOWN: usize = 48;

/// A press on the stack's content, from the popup's events.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Gesture {
    Pressed,
    Moved(iced::Point),
    Released,
}

/// The press, the moves and the release of the left button on window `id`.
pub fn gesture(
    event: iced::Event,
    _: iced::event::Status,
    id: iced::window::Id,
) -> Option<(iced::window::Id, Gesture)> {
    use iced::mouse::{Button, Event};
    let gesture = match event {
        iced::Event::Mouse(Event::ButtonPressed(Button::Left)) => Gesture::Pressed,
        iced::Event::Mouse(Event::CursorMoved { position }) => Gesture::Moved(position),
        iced::Event::Mouse(Event::ButtonReleased(Button::Left)) => Gesture::Released,
        _ => return None,
    };
    Some((id, gesture))
}

/// A file dragged out of the stack's content (lot D-21k2): pressed on an item, the pointer goes
/// `DRAG_THRESHOLD` away.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DragOut {
    hovered: Option<usize>,
    pointer: Option<iced::Point>,
    pressed: Option<(usize, iced::Point)>,
    dragging: bool,
}

impl DragOut {
    pub fn hover(&mut self, index: Option<usize>) {
        self.hovered = index;
    }

    /// The item to drag, once the pointer pressed on it has gone far enough.
    pub fn gesture(&mut self, gesture: Gesture) -> Option<usize> {
        match gesture {
            Gesture::Pressed => {
                self.pressed = self.hovered.zip(self.pointer);
                None
            }
            Gesture::Moved(point) => {
                self.pointer = Some(point);
                let (index, from) = self.pressed?;
                if self.dragging || from.distance(point) < sherlock_ui::dock::DRAG_THRESHOLD {
                    return None;
                }
                self.pressed = None;
                Some(index)
            }
            Gesture::Released => {
                self.pressed = None;
                None
            }
        }
    }

    pub fn started(&mut self) {
        self.dragging = true;
    }

    /// The drag is over: whether it was one of the stack's.
    pub fn ended(&mut self) -> bool {
        std::mem::take(&mut self.dragging)
    }
}

/// The `file://` address of `path`, its bytes outside the unreserved ones and `/` written
/// `%XX`, as `text/uri-list` asks.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// How the content of the stack opens, from its setting (lot D-21k2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presentation {
    Fan,
    Grid,
    List,
}

/// The presentation of `count` items for `view`: « Automatique » is the fan for a few items,
/// else the grid. The fan rises over a dock at the bottom of the screen only, as on macOS.
pub fn presentation(view: dock_config::stack::View, count: usize, bottom: bool) -> Presentation {
    use dock_config::stack::View;
    let fits = count <= sherlock_ui::dock::FAN_MAX;
    match view {
        View::Fan | View::Automatic if bottom && (fits || view == View::Fan) => Presentation::Fan,
        View::List => Presentation::List,
        _ => Presentation::Grid,
    }
}
/// Largest `user-dirs.dirs` read.
const MAX_USER_DIRS: u64 = 64 * 1024;

/// A stack: its folder, its name in the dock, its newest items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    pub folder: PathBuf,
    pub name: String,
    pub items: Vec<Shown>,
    /// Its three first items as the cards of its icon (lot D-21k2): a PNG's own bytes, else
    /// the glyph of its kind.
    pub pile: Vec<PileCard>,
}

/// Bytes of a PNG drawn on a card of the stack's icon, at most: a larger picture shows its
/// kind's glyph.
pub const MAX_PILE_PNG: u64 = 512 * 1024;

/// The cards of the stack's icon for `items`: read here, on the watcher's thread.
pub fn pile(items: &[Shown]) -> Vec<PileCard> {
    items
        .iter()
        .take(3)
        .map(|item| {
            let png = item
                .path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"));
            png.then(|| read_png(&item.path))
                .flatten()
                .map_or(PileCard::Glyph(item.icon), PileCard::Png)
        })
        .collect()
}

/// The bytes of the PNG at `path`, a regular file of `MAX_PILE_PNG` at most.
fn read_png(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_PILE_PNG + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    (png && bytes.len() as u64 <= MAX_PILE_PNG).then_some(bytes)
}

/// An item of a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub path: PathBuf,
    pub name: String,
    pub folder: bool,
    /// Its picture in the grid, as Evidence draws it in its lists.
    pub icon: Icon,
}

/// The downloads stack, each time its folder changes; `None` without a downloads folder. The
/// thread stops when the stream is dropped.
pub fn watch(home: PathBuf, config_home: PathBuf, sort: Sort) -> impl Stream<Item = Option<Stack>> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(String::from("watch-stack"))
        .spawn(move || {
            let Some(folder) = downloads(&home, &config_home) else {
                let _ = sender.unbounded_send(None);
                return;
            };
            let name = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let read = || {
                let items = newest(&folder, sort);
                Some(Stack {
                    folder: folder.clone(),
                    name: name.clone(),
                    pile: pile(&items),
                    items,
                })
            };
            if let Err(error) = crate::trash::follow(&folder, read, &sender) {
                tracing::warn!(%error, "cannot follow the downloads folder");
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "cannot start the stack watcher");
    }
    receiver
}

/// The downloads folder of the user: `XDG_DOWNLOAD_DIR` of `$XDG_CONFIG_HOME/user-dirs.dirs`
/// (`$HOME/` replaced), else `~/Downloads`; only when it exists. Reads a file: off the
/// interface thread.
pub fn downloads(home: &Path, config_home: &Path) -> Option<PathBuf> {
    let text = mime_apps::read_bounded(&config_home.join("user-dirs.dirs"), MAX_USER_DIRS).ok();
    let named = text.as_deref().and_then(|text| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=")?;
            let value = value.trim().trim_matches('"');
            let path = match value.strip_prefix("$HOME") {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None => PathBuf::from(value),
            };
            // `$HOME` itself is no downloads folder; a relative path is refused.
            (path.is_absolute() && path != home).then_some(path)
        })
    });
    named
        .or_else(|| Some(home.join("Downloads")))
        .filter(|path| path.is_dir())
}

/// The first items of `folder` in the order `sort` (lot D-21k2: the dates newest first, the
/// name and the kind in alphabetical order), hidden ones left out, `MAX_SHOWN` at most among
/// the first `MAX_LOOKED_AT` entries.
pub fn newest(folder: &Path, sort: Sort) -> Vec<Shown> {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, Shown)> = dir
        .flatten()
        .take(MAX_LOOKED_AT)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let when = match sort {
                // Its status change: when it came into the folder (moved or written there).
                Sort::Added => {
                    let secs = u64::try_from(meta.ctime()).unwrap_or(0);
                    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)
                }
                Sort::Created => meta.created().unwrap_or(modified),
                _ => modified,
            };
            let icon = icon_of(&name, meta.is_dir());
            Some((
                when,
                Shown {
                    path: entry.path(),
                    name,
                    folder: meta.is_dir(),
                    icon,
                },
            ))
        })
        .collect();
    let kind = |name: &str| {
        let extension = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        extension.unwrap_or_default()
    };
    match sort {
        Sort::Name => found.sort_by_key(|a| a.1.name.to_lowercase()),
        Sort::Kind => found.sort_by(|a, b| {
            (!a.1.folder, kind(&a.1.name), a.1.name.to_lowercase()).cmp(&(
                !b.1.folder,
                kind(&b.1.name),
                b.1.name.to_lowercase(),
            ))
        }),
        _ => found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name))),
    }
    found
        .into_iter()
        .take(MAX_SHOWN)
        .map(|(_, shown)| shown)
        .collect()
}

/// The picture of an item, as in Evidence's lists (`listing::Entry::icon`).
fn icon_of(name: &str, folder: bool) -> Icon {
    let extension = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    match extension.as_deref() {
        _ if folder => Icon::Folder,
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") => Icon::Image,
        _ => Icon::Document,
    }
}

/// Opens `path` with its default application, a folder in Evidence, on a thread of its own.
pub fn open(path: PathBuf, folder: bool) {
    let spawned = std::thread::Builder::new()
        .name(String::from("dock-open"))
        .spawn(move || {
            let result = if folder {
                crate::launch::open_in_evidence(&path.to_string_lossy())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                open_file(&path)
            };
            if let Err(error) = result {
                tracing::warn!(%error, "cannot open the item of the stack");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start a thread to open the item");
    }
}

fn open_file(path: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let dirs = mime_apps::Dirs::from_env(&home);
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .map(str::to_lowercase)
        .collect();
    let catalog = mime_apps::Catalog::load(dirs, &desktops, &[]);
    let mime = catalog.mime.type_of(path, false);
    let app = catalog
        .default_for(&mime)
        .ok_or_else(|| format!("no application opens {mime}"))?;
    let (program, args) = catalog
        .command(app, &[path.to_path_buf()])
        .ok_or_else(|| format!("{} has no command", app.id))?;
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| {
            let _ = std::thread::Builder::new()
                .name(String::from("dock-child"))
                .spawn(move || child.wait());
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_goes_out_as_its_address_once_pulled_far_enough() {
        assert_eq!(
            super::uri(Path::new("/home/me/Downloads/rapport final \u{e9}.pdf")),
            "file:///home/me/Downloads/rapport%20final%20%C3%A9.pdf"
        );
        let mut drag = super::DragOut::default();
        let at = |x: f32| super::Gesture::Moved(iced::Point::new(x, 10.0));
        assert_eq!(drag.gesture(at(10.0)), None);
        drag.hover(Some(2));
        assert_eq!(drag.gesture(super::Gesture::Pressed), None);
        assert_eq!(drag.gesture(at(12.0)), None, "a click, not a drag");
        assert_eq!(
            drag.gesture(at(10.0 + sherlock_ui::dock::DRAG_THRESHOLD)),
            Some(2)
        );
        assert_eq!(drag.gesture(at(80.0)), None, "once");
        drag.started();
        assert!(drag.ended() && !drag.ended());
        drag.hover(None);
        let _ = drag.gesture(super::Gesture::Pressed);
        assert_eq!(drag.gesture(at(200.0)), None, "pressed beside the items");
    }

    use super::*;

    #[test]
    fn the_downloads_folder_is_found_and_its_newest_items_listed() {
        let root = std::env::temp_dir().join(format!("dock-stack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, config) = (root.join("home"), root.join("home/.config"));
        std::fs::create_dir_all(home.join("Downloads-fr")).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        assert_eq!(downloads(&home, &config), None, "no file, no ~/Downloads");
        std::fs::write(
            config.join("user-dirs.dirs"),
            "# written by xdg-user-dirs-update\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads-fr\"\n",
        )
        .unwrap();
        let folder = home.join("Downloads-fr");
        assert_eq!(downloads(&home, &config), Some(folder.clone()));
        // `$HOME` itself is refused.
        std::fs::write(
            config.join("user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"$HOME\"\n",
        )
        .unwrap();
        assert_eq!(downloads(&home, &config), None);

        for (i, name) in ["old.pdf", "photo.png", ".cache", "new.zip"]
            .iter()
            .enumerate()
        {
            let path = folder.join(name);
            std::fs::write(&path, "x").unwrap();
            let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        std::fs::create_dir(folder.join("folder")).unwrap();
        let names: Vec<String> = newest(&folder, Sort::Modified)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names[..4], ["folder", "new.zip", "photo.png", "old.pdf"]);
        let icons: Vec<Icon> = newest(&folder, Sort::Modified)
            .into_iter()
            .map(|s| s.icon)
            .collect();
        assert_eq!(
            icons,
            [Icon::Folder, Icon::Document, Icon::Image, Icon::Document]
        );
        // By name, by kind (the folders first, then by extension).
        let by =
            |sort| -> Vec<String> { newest(&folder, sort).into_iter().map(|s| s.name).collect() };
        assert_eq!(
            by(Sort::Name),
            ["folder", "new.zip", "old.pdf", "photo.png"]
        );
        assert_eq!(
            by(Sort::Kind),
            ["folder", "old.pdf", "photo.png", "new.zip"]
        );
        assert_eq!(names.len(), 4, "the hidden one is left out");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
