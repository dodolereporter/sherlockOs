//! The notification area (lot D-45): the icons applications put in the bar with
//! `StatusNotifierItem` (Discord, Steam, a VPN), the KDE protocol that libappindicator, Electron
//! and Qt speak. The bar serves the `StatusNotifierWatcher` where applications register their
//! item (`watcher`), and is the host that reads each item and follows its changes (`host`).
//! This module holds what an item is, read from its D-Bus properties, and how its icon is
//! found: a name in the icon theme, or pixels the application sends.

pub mod host;
pub mod icon;
pub mod watcher;

use std::collections::HashMap;

use zbus::zvariant::{OwnedValue, Value};

pub use host::{Event, watch};
pub use icon::Icon;

/// The interface of an item.
pub const ITEM: &str = "org.kde.StatusNotifierItem";
/// Where an item registered by its bus name alone is.
pub const ITEM_PATH: &str = "/StatusNotifierItem";

/// What an item says of itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing to show: the item is hidden.
    Passive,
    #[default]
    Active,
    /// Something waits for the user: the icon breathes and the chevron carries a dot.
    NeedsAttention,
}

impl Status {
    fn parse(value: &str) -> Self {
        match value {
            "Passive" => Self::Passive,
            "NeedsAttention" => Self::NeedsAttention,
            _ => Self::Active,
        }
    }
}

/// An image the application sends: `width` by `height` pixels in ARGB32, network byte order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Pixmap {
    pub width: i32,
    pub height: i32,
    pub argb: Vec<u8>,
}

impl Pixmap {
    /// Its pixels in RGBA, as iced draws them, or `None` when they do not fill the size it
    /// gives. The alpha stays straight, as the protocol sends it.
    pub fn rgba(&self) -> Option<Vec<u8>> {
        let (width, height) = (
            usize::try_from(self.width).ok()?,
            usize::try_from(self.height).ok()?,
        );
        let count = width.checked_mul(height)?;
        if count == 0 || self.argb.len() < count.checked_mul(4)? {
            return None;
        }
        let pixels = self.argb.as_chunks::<4>().0.iter().take(count);
        Some(pixels.flat_map(|p| [p[1], p[2], p[3], p[0]]).collect())
    }
}

/// The pixmap to draw `side` device pixels wide: the smallest one at least that large, else
/// the largest. `None` without a usable one.
pub fn best(pixmaps: &[Pixmap], side: i32) -> Option<&Pixmap> {
    let usable = pixmaps.iter().filter(|p| p.rgba().is_some());
    let large = usable.clone().filter(|p| p.width.min(p.height) >= side);
    large
        .min_by_key(|p| p.width * p.height)
        .or_else(|| usable.max_by_key(|p| p.width * p.height))
}

/// An icon as the item describes it: a name in the icon theme, and pixels.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    pub name: Option<String>,
    pub pixmaps: Vec<Pixmap>,
}

impl Source {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.pixmaps.is_empty()
    }
}

/// The properties of an item, as `GetAll` gives them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Properties {
    /// Stable from one start of the application to the next: what the bar remembers it by.
    pub id: String,
    pub title: String,
    pub status: Status,
    pub icon: Source,
    pub attention: Source,
    pub overlay: Source,
    /// A folder of its own where the application puts its icons (Electron, libappindicator).
    pub theme_path: Option<String>,
    /// The title and the text of its tooltip, without markup.
    pub tooltip: Option<(String, String)>,
    /// The object path of its menu (`com.canonical.dbusmenu`).
    pub menu: Option<String>,
    /// It only has a menu: a click opens it rather than activating the item.
    pub item_is_menu: bool,
}

impl Properties {
    /// Reads `GetAll`'s answer. Unknown properties and wrong types are left out.
    pub fn parse(values: &HashMap<String, OwnedValue>) -> Self {
        let text = |name: &str| match values.get(name).map(|v| &**v) {
            Some(Value::Str(s)) if !s.is_empty() => Some(s.as_str().to_owned()),
            _ => None,
        };
        let pixmaps = |name: &str| values.get(name).map(|v| pixmaps(v)).unwrap_or_default();
        let source = |name: &str, pixels: &str| Source {
            name: text(name),
            pixmaps: pixmaps(pixels),
        };
        let menu = match values.get("Menu").map(|v| &**v) {
            Some(Value::ObjectPath(path)) if path.as_str() != "/" => Some(path.to_string()),
            _ => None,
        };
        Self {
            id: text("Id").unwrap_or_default(),
            title: text("Title").unwrap_or_default(),
            status: Status::parse(&text("Status").unwrap_or_default()),
            icon: source("IconName", "IconPixmap"),
            attention: source("AttentionIconName", "AttentionIconPixmap"),
            overlay: source("OverlayIconName", "OverlayIconPixmap"),
            theme_path: text("IconThemePath"),
            tooltip: values.get("ToolTip").and_then(|v| tooltip(v)),
            menu,
            item_is_menu: matches!(
                values.get("ItemIsMenu").map(|v| &**v),
                Some(Value::Bool(true))
            ),
        }
    }

    /// What its tooltip says: the tooltip's title and text, else the item's title, else its id.
    pub fn label(&self) -> (String, String) {
        match &self.tooltip {
            Some((title, text)) if !title.is_empty() => (title.clone(), text.clone()),
            Some((_, text)) if !text.is_empty() => (self.title.clone(), text.clone()),
            _ if !self.title.is_empty() => (self.title.clone(), String::new()),
            _ => (self.id.clone(), String::new()),
        }
    }
}

/// `a(iiay)`.
fn pixmaps(value: &Value<'_>) -> Vec<Pixmap> {
    let Ok(value) = value.try_clone() else {
        return Vec::new();
    };
    let list = Vec::<(i32, i32, Vec<u8>)>::try_from(value).unwrap_or_default();
    list.into_iter()
        .map(|(width, height, argb)| Pixmap {
            width,
            height,
            argb,
        })
        .collect()
}

/// `(sa(iiay)ss)`: icon name, pixmaps, title, text. The text may hold markup: it goes.
fn tooltip(value: &Value<'_>) -> Option<(String, String)> {
    let value = value.try_clone().ok()?;
    type Tip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);
    let (_, _, title, text) = Tip::try_from(value).ok()?;
    let (title, text) = (strip_markup(&title), strip_markup(&text));
    (!title.is_empty() || !text.is_empty()).then_some((title, text))
}

/// `text` without its tags, its entities read, on one line.
pub fn strip_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// An item of the notification area, its icons found.
#[derive(Debug, Clone)]
pub struct Item {
    /// As the watcher lists it: its bus name, then its object path.
    pub key: String,
    pub service: String,
    pub path: String,
    pub properties: Properties,
    pub icon: Option<Icon>,
    pub attention: Option<Icon>,
    pub overlay: Option<Icon>,
}

impl Item {
    /// Shown in the notification area: not passive.
    pub fn shown(&self) -> bool {
        self.properties.status != Status::Passive
    }

    pub fn needs_attention(&self) -> bool {
        self.properties.status == Status::NeedsAttention
    }

    /// The icon to draw now: the attention icon while it needs attention, when it has one.
    pub fn drawn(&self) -> Option<&Icon> {
        let attention = self.needs_attention().then_some(self.attention.as_ref());
        attention.flatten().or(self.icon.as_ref())
    }
}

/// What the bar asks of an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// A click: its main action, at a point of the screen.
    Activate(i32, i32),
    /// A click of the middle button.
    SecondaryActivate(i32, i32),
    /// The wheel: so much, 120 a notch as Qt counts, vertical or not.
    Scroll(i32, bool),
}

impl Request {
    /// The wheel's `delta`, `None` when it is nothing.
    pub fn scroll(delta: iced::mouse::ScrollDelta) -> Option<Self> {
        let (x, y) = match delta {
            iced::mouse::ScrollDelta::Lines { x, y } => (x * 120.0, y * 120.0),
            iced::mouse::ScrollDelta::Pixels { x, y } => (x, y),
        };
        let vertical = y.abs() >= x.abs();
        let amount = if vertical { y } else { x }.round() as i32;
        (amount != 0).then_some(Self::Scroll(amount, vertical))
    }

    /// Sends it to the item at `path` of `service`. Within the connection's time limit.
    pub async fn send(
        &self,
        bus: &zbus::Connection,
        service: &str,
        path: &str,
    ) -> zbus::Result<()> {
        let (service, interface) = (Some(service), Some(ITEM));
        let called = match *self {
            Self::Activate(x, y) => {
                bus.call_method(service, path, interface, "Activate", &(x, y))
                    .await
            }
            Self::SecondaryActivate(x, y) => {
                bus.call_method(service, path, interface, "SecondaryActivate", &(x, y))
                    .await
            }
            Self::Scroll(amount, vertical) => {
                let orientation = if vertical { "vertical" } else { "horizontal" };
                bus.call_method(service, path, interface, "Scroll", &(amount, orientation))
                    .await
            }
        };
        called.map(|_| ())
    }
}

/// The bus name and the object path of an item, from what it registered: a bus name (the path
/// is then `/StatusNotifierItem`), or an object path on the connection that called (`sender`),
/// as libappindicator does.
pub fn address(registered: &str, sender: &str) -> (String, String) {
    if registered.starts_with('/') {
        return (sender.to_owned(), registered.to_owned());
    }
    match registered.split_once('/') {
        Some((service, path)) => (service.to_owned(), format!("/{path}")),
        None => (registered.to_owned(), ITEM_PATH.to_owned()),
    }
}

/// The key of an item in the watcher's list: its bus name, then its object path.
pub fn key(service: &str, path: &str) -> String {
    format!("{service}{path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixmap_goes_from_argb_in_network_order_to_rgba() {
        // Two pixels: opaque red, then half transparent blue.
        let pixmap = Pixmap {
            width: 2,
            height: 1,
            argb: vec![0xFF, 0xFF, 0x00, 0x00, 0x80, 0x00, 0x00, 0xFF],
        };
        assert_eq!(
            pixmap.rgba(),
            Some(vec![0xFF, 0x00, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x80])
        );
        // Short of pixels, or empty: refused.
        let short = Pixmap {
            argb: vec![0; 4],
            ..pixmap.clone()
        };
        assert_eq!(short.rgba(), None);
        let empty = Pixmap { width: 0, ..pixmap };
        assert_eq!(empty.rgba(), None);
    }

    #[test]
    fn the_smallest_pixmap_large_enough_is_drawn() {
        let square = |side: i32| Pixmap {
            width: side,
            height: side,
            argb: vec![0; (side * side * 4) as usize],
        };
        let pixmaps = [square(16), square(64), square(32), square(128)];
        assert_eq!(best(&pixmaps, 36).map(|p| p.width), Some(64));
        assert_eq!(best(&pixmaps, 200).map(|p| p.width), Some(128));
        assert_eq!(best(&pixmaps, 8).map(|p| p.width), Some(16));
        assert_eq!(best(&[], 8), None);
    }

    #[test]
    fn an_item_is_found_by_name_or_by_path() {
        assert_eq!(
            address("org.kde.StatusNotifierItem-12-1", ":1.5"),
            (
                "org.kde.StatusNotifierItem-12-1".into(),
                "/StatusNotifierItem".into()
            )
        );
        assert_eq!(
            address("/org/ayatana/NotificationItem/discord", ":1.7"),
            (
                ":1.7".into(),
                "/org/ayatana/NotificationItem/discord".into()
            )
        );
        assert_eq!(
            address(":1.9/org/blueman/sni", ":1.2"),
            (":1.9".into(), "/org/blueman/sni".into())
        );
        assert_eq!(key(":1.7", "/a/b"), ":1.7/a/b");
    }

    #[test]
    fn properties_are_read_with_their_tooltip_without_markup() {
        let mut values: HashMap<String, OwnedValue> = HashMap::new();
        let mut put = |name: &str, value: Value<'_>| {
            values.insert(name.to_owned(), value.try_to_owned().unwrap());
        };
        put("Id", Value::from("discord"));
        put("Title", Value::from("Discord"));
        put("Status", Value::from("NeedsAttention"));
        put("IconName", Value::from("discord-tray"));
        put(
            "IconPixmap",
            Value::from(vec![(1i32, 1i32, vec![0xFFu8, 1, 2, 3])]),
        );
        put(
            "Menu",
            Value::from(zbus::zvariant::ObjectPath::try_from("/MenuBar").unwrap()),
        );
        put("ItemIsMenu", Value::from(false));
        let tip = (
            String::new(),
            Vec::<(i32, i32, Vec<u8>)>::new(),
            String::from("Discord"),
            String::from("<b>3</b> messages &amp; un appel"),
        );
        put("ToolTip", Value::from(tip));
        let read = Properties::parse(&values);
        assert_eq!(read.id, "discord");
        assert_eq!(read.status, Status::NeedsAttention);
        assert_eq!(read.icon.name.as_deref(), Some("discord-tray"));
        assert_eq!(read.icon.pixmaps[0].rgba(), Some(vec![1, 2, 3, 0xFF]));
        assert_eq!(read.menu.as_deref(), Some("/MenuBar"));
        assert_eq!(
            read.label(),
            ("Discord".into(), "3 messages & un appel".into())
        );
        assert!(read.attention.is_empty());
        // Without a tooltip, the title.
        let bare = Properties {
            tooltip: None,
            ..read
        };
        assert_eq!(bare.label(), ("Discord".into(), String::new()));
    }
}
