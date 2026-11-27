//! Client of `com.canonical.dbusmenu` (lot D-36): the menus an application exports on the
//! session bus, which the bar shows after its name. `GetLayout` gives the tree of entries,
//! `AboutToShow` lets the application fill a menu before it opens, `Event` « clicked » runs
//! an entry; `LayoutUpdated` and `ItemsPropertiesUpdated` say the tree changed.
//!
//! Every call has a time limit (`TIMEOUT`, on the connection) and runs in an iced task: an
//! application that hangs leaves its menus empty, never the bar frozen.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use iced::futures::stream::{self, BoxStream};
use iced::futures::{FutureExt, StreamExt};
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, MatchRule, MessageStream};

use crate::source::workspaces::AppMenu;

/// The interface.
pub const INTERFACE: &str = "com.canonical.dbusmenu";
/// Longest wait for an application's answer.
pub const TIMEOUT: Duration = Duration::from_millis(1500);

// What a layout may hold (Q-38): any application on the session bus can send one, and the bar
// lays it out on its interface thread. Beyond, entries are left out and labels cut.

/// Entries kept from one `GetLayout`, all levels together.
pub const MAX_ENTRIES: usize = 200;
/// Levels of submenus kept under the node asked for.
pub const MAX_DEPTH: usize = 8;
/// Characters kept of a label.
pub const MAX_LABEL_CHARS: usize = 256;
/// Key combinations of a shortcut, keys of a combination, and characters of a key name.
pub const MAX_COMBINATIONS: usize = 4;
pub const MAX_KEYS: usize = 4;
pub const MAX_KEY_CHARS: usize = 32;

/// The entries a layout may still bring.
struct Budget(usize);

/// The connection of the menus: the session bus, with `TIMEOUT` on every call.
pub async fn bus() -> zbus::Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    if let Some(bus) = BUS.get() {
        return Ok(bus.clone());
    }
    let bus = zbus::connection::Builder::session()?
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    Ok(BUS.get_or_init(|| bus).clone())
}

/// How an entry is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Toggle {
    #[default]
    None,
    /// A check mark, on or off.
    Check(bool),
    /// One choice among others, on or off.
    Radio(bool),
}

/// An entry and what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub id: i32,
    /// Without its mnemonic marks.
    pub label: String,
    pub separator: bool,
    pub enabled: bool,
    pub visible: bool,
    pub toggle: Toggle,
    /// Its shortcut as the menu writes it, such as « Ctrl+S »: from one or more key
    /// combinations pressed in turn, each such as `["Control", "S"]`.
    pub keys: Option<String>,
    /// It opens a submenu, even before its children are known.
    pub submenu: bool,
    pub children: Vec<Item>,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            id: 0,
            label: String::new(),
            separator: false,
            enabled: true,
            visible: true,
            toggle: Toggle::None,
            keys: None,
            submenu: false,
            children: Vec::new(),
        }
    }
}

impl Item {
    /// The entry of id `id` in the tree, itself included.
    pub fn find(&self, id: i32) -> Option<&Item> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    /// Puts the children of `item` under the entry of the same id in the tree, which keeps its
    /// own properties: Qt answers `GetLayout` of a menu with the menu's properties empty, and
    /// taking them would leave its button in the bar without a label (D-36d).
    pub fn graft(&mut self, item: Item) -> bool {
        if self.id == item.id {
            self.children = item.children;
            return true;
        }
        self.children.iter_mut().any(|c| c.graft(item.clone()))
    }

    /// The visible children: what a menu shows.
    pub fn shown(&self) -> impl Iterator<Item = &Item> {
        self.children.iter().filter(|c| c.visible)
    }

    /// It opens a menu: it says so, or it has entries.
    pub fn opens(&self) -> bool {
        self.submenu || !self.children.is_empty()
    }
}

/// `label` without its mnemonic marks: a single `_` (dbusmenu) or `&` (Qt, some GTK) goes, a
/// doubled one stays once.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' || c == '&' {
            if chars.peek() == Some(&c) {
                out.push(c);
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Reads one node of a layout: `(ia{sv}av)`, or a variant that holds one, within `MAX_ENTRIES`
/// and `MAX_DEPTH`.
pub fn decode(value: &Value<'_>) -> Option<Item> {
    decode_within(value, &mut Budget(MAX_ENTRIES), 0)
}

fn decode_within(value: &Value<'_>, budget: &mut Budget, depth: usize) -> Option<Item> {
    if let Value::Value(inner) = value {
        return decode_within(inner, budget, depth);
    }
    if budget.0 == 0 || depth > MAX_DEPTH {
        return None;
    }
    let Value::Structure(structure) = value else {
        return None;
    };
    let [id, properties, children] = structure.fields() else {
        return None;
    };
    let Value::I32(id) = id else { return None };
    let properties: HashMap<String, OwnedValue> = properties.try_clone().ok()?.try_into().ok()?;
    let children: Vec<OwnedValue> = children.try_clone().ok()?.try_into().ok()?;
    Some(item(*id, &properties, &children, budget, depth))
}

/// A node of a layout: `(ia{sv}av)`, as `GetLayout` returns it.
pub type Node = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// The entry of a node, its children read with `decode`.
fn item(
    id: i32,
    properties: &HashMap<String, OwnedValue>,
    children: &[OwnedValue],
    budget: &mut Budget,
    depth: usize,
) -> Item {
    budget.0 = budget.0.saturating_sub(1);
    let mut item = Item {
        id,
        ..Item::default()
    };
    for (name, value) in properties {
        apply(&mut item, name, value);
    }
    item.children = children
        .iter()
        .map_while(|c| (budget.0 > 0).then(|| decode_within(c, budget, depth + 1)))
        .flatten()
        .collect();
    item
}

/// Sets the property `name` of `item`. Unknown properties and wrong types are left out.
fn apply(item: &mut Item, name: &str, value: &Value<'_>) {
    let text = || match value {
        Value::Str(s) => Some(s.as_str().to_owned()),
        _ => None,
    };
    match (name, value) {
        ("label", _) => {
            let label: String = text()
                .unwrap_or_default()
                .chars()
                .take(MAX_LABEL_CHARS)
                .collect();
            item.label = strip_mnemonic(&label);
        }
        ("type", _) => item.separator = text().as_deref() == Some("separator"),
        ("enabled", Value::Bool(on)) => item.enabled = *on,
        ("visible", Value::Bool(on)) => item.visible = *on,
        ("children-display", _) => item.submenu = text().as_deref() == Some("submenu"),
        // `toggle-type` and `toggle-state` come in any order: each keeps what the other said.
        ("toggle-type", _) => {
            let on = matches!(item.toggle, Toggle::Check(true) | Toggle::Radio(true));
            item.toggle = match text().as_deref() {
                Some("checkmark") => Toggle::Check(on),
                Some("radio") => Toggle::Radio(on),
                _ => Toggle::None,
            };
        }
        ("toggle-state", Value::I32(state)) => {
            let on = *state == 1;
            item.toggle = match item.toggle {
                Toggle::Radio(_) => Toggle::Radio(on),
                // Until its type comes, a state means a check mark.
                _ => Toggle::Check(on),
            };
        }
        ("shortcut", _) => {
            let combinations = shortcut(value);
            let keys = sherlock_ui::strings::top_bar::shortcut(&combinations);
            item.keys = (!keys.is_empty()).then_some(keys);
        }
        _ => {}
    }
}

/// The key combinations of a `shortcut` property (`aas`).
fn shortcut(value: &Value<'_>) -> Vec<Vec<String>> {
    let Ok(value) = value.try_clone() else {
        return Vec::new();
    };
    let mut combinations = Vec::<Vec<String>>::try_from(value).unwrap_or_default();
    combinations.truncate(MAX_COMBINATIONS);
    for keys in &mut combinations {
        keys.truncate(MAX_KEYS);
        for key in keys.iter_mut() {
            *key = key.chars().take(MAX_KEY_CHARS).collect();
        }
    }
    combinations
}

/// The tree under `parent`, all levels: the revision of the layout, and the node.
pub async fn layout(bus: &Connection, menu: &AppMenu, parent: i32) -> zbus::Result<Item> {
    let names: Vec<&str> = Vec::new();
    let reply = bus
        .call_method(
            Some(menu.service.as_str()),
            menu.path.as_str(),
            Some(INTERFACE),
            "GetLayout",
            &(parent, -1i32, names),
        )
        .await?;
    let (_revision, (id, properties, children)): (u32, Node) = reply.body().deserialize()?;
    Ok(item(
        id,
        &properties,
        &children,
        &mut Budget(MAX_ENTRIES),
        0,
    ))
}

/// Tells the application the menu `id` is about to open. `true` when it changed it.
pub async fn about_to_show(bus: &Connection, menu: &AppMenu, id: i32) -> zbus::Result<bool> {
    let reply = bus
        .call_method(
            Some(menu.service.as_str()),
            menu.path.as_str(),
            Some(INTERFACE),
            "AboutToShow",
            &id,
        )
        .await?;
    reply.body().deserialize()
}

/// Runs the entry `id`: `Event` « clicked ».
pub async fn clicked(bus: &Connection, menu: &AppMenu, id: i32) -> zbus::Result<()> {
    let data = Value::from(0i32);
    let timestamp = 0u32;
    bus.call_method(
        Some(menu.service.as_str()),
        menu.path.as_str(),
        Some(INTERFACE),
        "Event",
        &(id, "clicked", data, timestamp),
    )
    .await?;
    Ok(())
}

/// The menu at `menu` changed, on the connection of the menus.
pub fn watch(menu: AppMenu) -> BoxStream<'static, ()> {
    let connected = async move {
        match bus().await {
            Ok(bus) => changes(bus, menu),
            Err(error) => {
                tracing::warn!(%error, "no session bus: the menu's changes are not followed");
                stream::empty().boxed()
            }
        }
    };
    connected.flatten_stream().boxed()
}

/// The menu at `menu` changed: a `LayoutUpdated` or `ItemsPropertiesUpdated` from it.
pub fn changes(bus: Connection, menu: AppMenu) -> BoxStream<'static, ()> {
    let watched = async move {
        let rule = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(menu.service.as_str())?
            .path(menu.path.as_str())?
            .interface(INTERFACE)?
            .build();
        let messages = MessageStream::for_match_rule(rule, &bus, Some(64)).await?;
        let changed = messages.filter_map(|message| async move {
            let message = message.ok()?;
            let member = message.header().member().map(|m| m.to_string());
            matches!(
                member.as_deref(),
                Some("LayoutUpdated" | "ItemsPropertiesUpdated")
            )
            .then_some(())
        });
        zbus::Result::Ok(changed.boxed())
    };
    watched
        .map(|watched| match watched {
            Ok(changes) => changes,
            Err(error) => {
                tracing::warn!(%error, "the menu's changes are not followed");
                stream::empty().boxed()
            }
        })
        .flatten_stream()
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{StructureBuilder, Value};

    fn node(
        id: i32,
        properties: Vec<(&str, Value<'static>)>,
        children: Vec<Value<'static>>,
    ) -> Value<'static> {
        let properties: HashMap<String, Value<'static>> = properties
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect();
        let children: Vec<Value<'static>> = children
            .into_iter()
            .map(|c| Value::Value(Box::new(c)))
            .collect();
        StructureBuilder::new()
            .add_field(id)
            .add_field(properties)
            .add_field(children)
            .build()
            .unwrap()
            .into()
    }

    #[test]
    fn a_mnemonic_goes_and_a_doubled_mark_stays_once() {
        assert_eq!(strip_mnemonic("_Fichier"), "Fichier");
        assert_eq!(strip_mnemonic("&Édition"), "Édition");
        assert_eq!(strip_mnemonic("Copier && coller"), "Copier & coller");
        assert_eq!(strip_mnemonic("snake__case"), "snake_case");
    }

    #[test]
    fn a_layout_is_read_with_its_entries() {
        let save = node(
            3,
            vec![
                ("label", Value::from("_Enregistrer")),
                (
                    "shortcut",
                    Value::from(vec![vec![String::from("Control"), String::from("S")]]),
                ),
            ],
            vec![],
        );
        let separator = node(4, vec![("type", Value::from("separator"))], vec![]);
        let wrap = node(
            5,
            vec![
                ("label", Value::from("Retour à la ligne")),
                ("toggle-state", Value::from(1i32)),
                ("toggle-type", Value::from("radio")),
                ("enabled", Value::from(false)),
            ],
            vec![],
        );
        let hidden = node(6, vec![("visible", Value::from(false))], vec![]);
        let file = node(
            1,
            vec![
                ("label", Value::from("_Fichier")),
                ("children-display", Value::from("submenu")),
            ],
            vec![save, separator, wrap, hidden],
        );
        let root = decode(&node(0, vec![], vec![file])).unwrap();
        let file = &root.children[0];
        assert_eq!(
            (file.id, file.label.as_str(), file.opens()),
            (1, "Fichier", true)
        );
        let shown: Vec<_> = file.shown().map(|i| i.id).collect();
        assert_eq!(shown, [3, 4, 5]);
        let save = root.find(3).unwrap();
        assert_eq!(save.label, "Enregistrer");
        assert_eq!(save.keys.as_deref(), Some("Ctrl+S"));
        assert!(root.find(4).unwrap().separator);
        let wrap = root.find(5).unwrap();
        assert_eq!(wrap.toggle, Toggle::Radio(true));
        assert!(!wrap.enabled);
        assert_eq!(decode(&Value::from(3i32)), None);
    }

    #[test]
    fn a_layout_too_large_is_cut_to_its_bounds() {
        // Ten thousand entries in one menu: the first ones only.
        let entries: Vec<Value<'static>> = (1..=10_000)
            .map(|i| node(i, vec![("label", Value::from("x"))], vec![]))
            .collect();
        let root = decode(&node(0, vec![], entries)).unwrap();
        assert_eq!(root.children.len(), MAX_ENTRIES - 1);
        // A chain of submenus deeper than the bound.
        let mut chain = node(100, vec![], vec![]);
        for id in (0..100).rev() {
            chain = node(id, vec![], vec![chain]);
        }
        let mut depth = 0;
        let mut at = &decode(&chain).unwrap();
        while let Some(next) = at.children.first() {
            depth += 1;
            at = next;
        }
        assert_eq!(depth, MAX_DEPTH);
        // A label of a megabyte, a shortcut of a hundred combinations.
        let long = "é".repeat(1 << 19);
        let keys: Vec<Vec<String>> = vec![vec!["Control".repeat(20); 50]; 100];
        let big = node(
            1,
            vec![
                ("label", Value::from(long)),
                ("shortcut", Value::from(keys)),
            ],
            vec![],
        );
        let item = decode(&big).unwrap();
        assert_eq!(item.label.chars().count(), MAX_LABEL_CHARS);
        let combinations = shortcut(&Value::from(vec![vec!["Control".repeat(20); 50]; 100]));
        assert_eq!(combinations.len(), MAX_COMBINATIONS);
        assert!(combinations.iter().all(|k| k.len() == MAX_KEYS));
        assert!(combinations[0][0].chars().count() == MAX_KEY_CHARS);
        assert!(item.keys.is_some_and(|k| k.chars().count() < 1000));
    }
}
