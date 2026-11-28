//! Evidence's menus in the bar (lot D-36c): « Fichier », « Édition », « Présentation »,
//! « Aller », « Fenêtre », exported with `com.canonical.dbusmenu` on the session bus at
//! `/MenuBar`, then registered with `com.canonical.AppMenu.Registrar` when the session has one
//! (the bar's). The compositor gives the menu to Evidence's window by its process.
//!
//! The entries run what the shortcuts and the right-click menu run: a click comes back to the
//! application as an `Action`. Their state (a selection, the hidden files shown, the history)
//! follows the application: a change sends `LayoutUpdated`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use iced::futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use sherlock_ui::texts::evidence as texts;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};
use zbus::{Connection, fdo};

use crate::app::{Command, WindowAction};

/// Where the menus are exported.
pub const PATH: &str = "/MenuBar";
/// The registrar of the session, and its object.
const REGISTRAR: &str = "com.canonical.AppMenu.Registrar";
const REGISTRAR_PATH: &str = "/com/canonical/AppMenu/Registrar";

/// What an entry runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Command),
    Back,
    Forward,
    Up,
    /// A folder of the sidebar, or the trash (`None`).
    Go(Option<PathBuf>),
    Window(WindowAction),
}

/// What the entries depend on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct State {
    /// Something is selected.
    pub selection: bool,
    /// The trash is shown.
    pub in_trash: bool,
    pub show_hidden: bool,
    pub show_preview: bool,
    /// Something can be undone, or redone (lot E-10).
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_back: bool,
    pub can_forward: bool,
    /// The places of « Aller »: label and folder.
    pub places: Vec<(&'static str, PathBuf)>,
}

/// One entry of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: i32,
    pub label: &'static str,
    pub action: Option<Action>,
    pub enabled: bool,
    pub separator: bool,
    /// `Some(on)` for an entry with a check mark.
    pub check: Option<bool>,
    /// Key names as D-Bus menus give them, such as `["Control", "c"]`.
    pub keys: &'static [&'static str],
    pub children: Vec<Node>,
}

fn entry(id: i32, label: &'static str, action: Action, keys: &'static [&'static str]) -> Node {
    Node {
        id,
        label,
        action: Some(action),
        enabled: true,
        separator: false,
        check: None,
        keys,
        children: Vec::new(),
    }
}

fn separator(id: i32) -> Node {
    Node {
        separator: true,
        action: None,
        ..entry(id, "", Action::Up, &[])
    }
}

fn menu(id: i32, label: &'static str, children: Vec<Node>) -> Node {
    Node {
        action: None,
        children,
        ..entry(id, label, Action::Up, &[])
    }
}

fn usable(node: Node, on: bool) -> Node {
    Node {
        enabled: on,
        ..node
    }
}

fn checked(node: Node, on: bool) -> Node {
    Node {
        check: Some(on),
        ..node
    }
}

/// The menus for `state`. The ids are fixed: an id names the same entry whatever the state.
pub fn tree(state: &State) -> Node {
    let run = Action::Run;
    let some = state.selection;
    let folder = !state.in_trash;
    let file = menu(
        1,
        texts::MENU_FILE,
        vec![
            usable(
                entry(
                    10,
                    texts::NEW_FOLDER,
                    run(Command::NewFolder),
                    &["Control", "Shift", "n"],
                ),
                folder,
            ),
            usable(
                entry(11, texts::OPEN, run(Command::Open), &["Return"]),
                some,
            ),
            usable(
                entry(12, texts::TERMINAL_HERE, run(Command::OpenTerminal), &[]),
                folder,
            ),
            separator(13),
            usable(
                entry(14, texts::RENAME, run(Command::Rename), &["F2"]),
                some && folder,
            ),
            usable(
                entry(
                    15,
                    texts::DUPLICATE,
                    run(Command::Duplicate),
                    &["Control", "d"],
                ),
                some && folder,
            ),
            usable(
                entry(
                    16,
                    texts::COPY_PATH,
                    run(Command::CopyPath),
                    &["Control", "Shift", "c"],
                ),
                some,
            ),
            separator(17),
            usable(
                entry(18, texts::MOVE_TO_TRASH, run(Command::Trash), &["Delete"]),
                some && folder,
            ),
            usable(
                entry(19, texts::RESTORE, run(Command::Restore), &[]),
                some && !folder,
            ),
            usable(
                entry(20, texts::EMPTY_TRASH, run(Command::EmptyTrash), &[]),
                !folder,
            ),
        ],
    );
    let edit = menu(
        2,
        texts::MENU_EDIT,
        vec![
            usable(
                entry(36, texts::UNDO, run(Command::Undo), &["Control", "z"]),
                state.can_undo,
            ),
            usable(
                entry(
                    37,
                    texts::REDO,
                    run(Command::Redo),
                    &["Control", "Shift", "z"],
                ),
                state.can_redo,
            ),
            separator(38),
            usable(
                entry(30, texts::CUT, run(Command::Cut), &["Control", "x"]),
                some && folder,
            ),
            usable(
                entry(31, texts::COPY, run(Command::Copy), &["Control", "c"]),
                some,
            ),
            usable(
                entry(32, texts::PASTE, run(Command::Paste), &["Control", "v"]),
                folder,
            ),
            separator(33),
            entry(
                34,
                texts::SELECT_ALL,
                run(Command::SelectAll),
                &["Control", "a"],
            ),
        ],
    );
    let view = menu(
        3,
        texts::MENU_VIEW,
        vec![
            checked(
                entry(
                    40,
                    texts::SHOW_HIDDEN,
                    run(Command::ToggleHidden),
                    &["Control", "h"],
                ),
                state.show_hidden,
            ),
            checked(
                entry(41, texts::SHOW_PREVIEW, run(Command::TogglePreview), &[]),
                state.show_preview,
            ),
            separator(42),
            entry(43, texts::REFRESH, run(Command::Refresh), &["F5"]),
        ],
    );
    let mut places = vec![
        usable(
            entry(50, texts::BACK, Action::Back, &["Alt", "Left"]),
            state.can_back,
        ),
        usable(
            entry(51, texts::FORWARD, Action::Forward, &["Alt", "Right"]),
            state.can_forward,
        ),
        entry(52, texts::PARENT_FOLDER, Action::Up, &["Alt", "Up"]),
        entry(
            54,
            texts::GO_TO_FOLDER_MENU,
            Action::Run(Command::GoToFolder),
            &["Control", "Shift", "g"],
        ),
        separator(53),
    ];
    for (i, (label, path)) in state.places.iter().enumerate() {
        let id = 60 + i32::try_from(i).unwrap_or(0);
        places.push(entry(id, label, Action::Go(Some(path.clone())), &[]));
    }
    places.push(entry(59, texts::TRASH, Action::Go(None), &[]));
    let go = menu(4, texts::MENU_GO, places);
    let window = menu(
        5,
        texts::MENU_WINDOW,
        vec![
            entry(
                80,
                texts::MINIMIZE,
                Action::Window(WindowAction::Minimize),
                &[],
            ),
            entry(
                81,
                texts::MAXIMIZE,
                Action::Window(WindowAction::Maximize),
                &[],
            ),
            separator(82),
            entry(
                83,
                texts::CLOSE_WINDOW,
                Action::Window(WindowAction::Close),
                &["Control", "w"],
            ),
        ],
    );
    menu(0, "", vec![file, edit, view, go, window])
}

impl Node {
    fn find(&self, id: i32) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    /// The node as `GetLayout` gives it: `(ia{sv}av)`, children to `depth` (-1: all).
    fn layout(
        &self,
        depth: i32,
    ) -> zbus::Result<(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>)> {
        let mut properties: HashMap<String, OwnedValue> = HashMap::new();
        let mut put = |name: &str, value: Value<'_>| -> zbus::Result<()> {
            properties.insert(name.to_owned(), value.try_to_owned()?);
            Ok(())
        };
        if self.separator {
            put("type", Value::from("separator"))?;
        } else if self.id != 0 {
            put("label", Value::from(self.label))?;
        }
        if !self.enabled {
            put("enabled", Value::from(false))?;
        }
        if let Some(on) = self.check {
            put("toggle-type", Value::from("checkmark"))?;
            put("toggle-state", Value::from(i32::from(on)))?;
        }
        if !self.keys.is_empty() {
            put("shortcut", Value::from(vec![self.keys.to_vec()]))?;
        }
        if !self.children.is_empty() {
            put("children-display", Value::from("submenu"))?;
        }
        let mut children = Vec::new();
        if depth != 0 {
            for child in &self.children {
                let node = child.layout(depth - 1)?;
                children.push(OwnedValue::try_from(Value::from(node))?);
            }
        }
        Ok((self.id, properties, children))
    }
}

/// The exported menus: the state they show, and where a click goes.
struct Menus {
    state: Arc<Mutex<(u32, State)>>,
    clicks: UnboundedSender<Action>,
}

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menus {
    fn get_layout(
        &self,
        parent: i32,
        depth: i32,
        _names: Vec<String>,
    ) -> fdo::Result<(u32, Layout)> {
        let (revision, state) = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let root = tree(&state);
        let node = root
            .find(parent)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("no entry {parent}")))?;
        let layout = node
            .layout(depth)
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        Ok((revision, layout))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _names: Vec<String>,
    ) -> fdo::Result<Vec<(i32, HashMap<String, OwnedValue>)>> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1
            .clone();
        let root = tree(&state);
        let nodes = ids.iter().filter_map(|id| root.find(*id));
        let properties = nodes.map(|n| n.layout(0).map(|(id, properties, _)| (id, properties)));
        properties
            .collect::<zbus::Result<_>>()
            .map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let state = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1
            .clone();
        let root = tree(&state);
        let action = root
            .find(id)
            .filter(|n| n.enabled)
            .and_then(|n| n.action.clone());
        if let Some(action) = action {
            tracing::debug!(id, ?action, "menu entry clicked in the bar");
            let _ = self.clicks.unbounded_send(action);
        }
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn status(&self) -> String {
        String::from("normal")
    }

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;
}

/// The menus served on a bus: a new state changes what they show.
#[derive(Clone)]
pub struct MenuBar {
    bus: Connection,
    state: Arc<Mutex<(u32, State)>>,
}

impl MenuBar {
    /// Exports the menus on `bus` and registers them when a registrar answers there. The
    /// clicks come on the receiver.
    pub async fn serve(
        bus: Connection,
        state: State,
    ) -> zbus::Result<(Self, UnboundedReceiver<Action>)> {
        let (clicks, receiver) = unbounded();
        let shared = Arc::new(Mutex::new((1, state)));
        let menus = Menus {
            state: Arc::clone(&shared),
            clicks,
        };
        bus.object_server().at(PATH, menus).await?;
        let registrar = fdo::DBusProxy::new(&bus).await?;
        let name = zbus::names::BusName::try_from(REGISTRAR)?;
        if registrar.name_has_owner(name).await? {
            // Under Wayland the window id names nothing: the compositor finds the window by
            // the process of this connection.
            bus.call_method(
                Some(REGISTRAR),
                REGISTRAR_PATH,
                Some(REGISTRAR),
                "RegisterWindow",
                &(0u32, zbus::zvariant::ObjectPath::try_from(PATH)?),
            )
            .await?;
            tracing::info!("menus registered with the bar");
        } else {
            tracing::debug!("no menu registrar on the session bus: menus exported only");
        }
        Ok((Self { bus, state: shared }, receiver))
    }

    /// Shows `state`; when it changed, the bar is told the menus changed.
    pub fn update(&self, state: State) {
        let revision = {
            let mut shared = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if shared.1 == state {
                return;
            }
            shared.0 = shared.0.wrapping_add(1);
            shared.1 = state;
            shared.0
        };
        let bus = self.bus.clone();
        let told = async move {
            let Ok(emitter) = SignalEmitter::new(&bus, PATH) else {
                return;
            };
            if let Err(error) = Menus::layout_updated(&emitter, revision, 0).await {
                tracing::debug!(%error, "LayoutUpdated not sent");
            }
        };
        self.bus.executor().spawn(told, "menubar-updated").detach();
    }
}

/// What the menus tell the application.
#[derive(Clone)]
pub enum Event {
    /// The menus are served: their handle, for the state.
    Ready(MenuBar),
    Click(Action),
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ready(_) => f.write_str("Ready"),
            Self::Click(action) => f.debug_tuple("Click").field(action).finish(),
        }
    }
}

static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The application exports its menus: `main` says so. Tests never take the session bus.
pub fn enable() {
    ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Serves the menus on the session bus with `state`, then gives the clicks. Without a session
/// bus, nothing: Evidence keeps its own shortcuts and right-click menu.
pub fn events(state: State) -> iced::futures::stream::BoxStream<'static, Event> {
    use iced::futures::{FutureExt, StreamExt, stream};
    let served = async move {
        let bus = zbus::connection::Builder::session()?
            .method_timeout(std::time::Duration::from_secs(5))
            .build()
            .await?;
        MenuBar::serve(bus, state).await
    };
    served
        .map(|served| match served {
            Ok((menubar, clicks)) => stream::once(async move { Event::Ready(menubar) })
                .chain(clicks.map(Event::Click))
                .boxed(),
            Err(error) => {
                tracing::warn!(%error, "the menus are not exported to the bar");
                stream::empty().boxed()
            }
        })
        .flatten_stream()
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entries_follow_the_state() {
        let mut state = State::default();
        let root = tree(&state);
        let labels: Vec<_> = root.children.iter().map(|m| m.label).collect();
        assert_eq!(
            labels,
            ["Fichier", "Édition", "Présentation", "Aller", "Fenêtre"]
        );
        // Nothing selected: « Copier » is disabled, « Coller » and « Nouveau dossier » are not.
        assert!(!root.find(31).unwrap().enabled);
        assert!(root.find(32).unwrap().enabled && root.find(10).unwrap().enabled);
        assert!(!root.find(50).unwrap().enabled, "no history");
        state.selection = true;
        state.show_hidden = true;
        state.can_back = true;
        let root = tree(&state);
        assert!(root.find(31).unwrap().enabled && root.find(50).unwrap().enabled);
        assert_eq!(root.find(40).unwrap().check, Some(true));
        // In the trash: restore and empty, no new folder.
        state.in_trash = true;
        let root = tree(&state);
        assert!(root.find(19).unwrap().enabled && root.find(20).unwrap().enabled);
        assert!(!root.find(10).unwrap().enabled && !root.find(18).unwrap().enabled);
    }

    #[test]
    fn a_layout_carries_labels_checks_and_shortcuts() {
        let state = State {
            show_hidden: true,
            ..State::default()
        };
        let (id, properties, children) = tree(&state).find(3).unwrap().layout(-1).unwrap();
        assert_eq!(id, 3);
        assert_eq!(
            properties["label"],
            Value::from("Présentation").try_to_owned().unwrap()
        );
        assert_eq!(children.len(), 4);
        let (_, hidden, _): Layout = children[0].try_clone().unwrap().try_into().unwrap();
        assert_eq!(
            hidden["toggle-state"],
            Value::from(1i32).try_to_owned().unwrap()
        );
        let shortcut: Vec<Vec<String>> =
            hidden["shortcut"].try_clone().unwrap().try_into().unwrap();
        assert_eq!(shortcut, [["Control", "h"]]);
    }
}
