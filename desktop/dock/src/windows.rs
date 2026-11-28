//! Open windows, for the dot under open applications, the active one, a click on an open
//! application (brought forward, or minimized when it already is), and « Quitter ».
//!
//! A source implements `Source`, like the bar's sources. Three exist, tried in this order:
//!
//! - `Placeholder`, a fake list set by two environment variables, for tests and screenshots.
//! - `shell::Shell`, our compositor's `sherlock_shell_v1` on its shell socket (ADR 0021).
//! - `Wlr`, the `zwlr_foreign_toplevel_manager_v1` protocol of wlroots compositors.
//!
//! Without any, the dock does not know the windows: no dot, « Quitter » disabled.
//!
//! Acting on windows goes through `Control`: `Auto` sends `activate`, `minimize` or `close`
//! through the shell protocol when its source runs, through the wlroots one otherwise;
//! `Recorder` is the fake one.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{
    ConnectError, Connection, Dispatch, DispatchError, EventQueue, QueueHandle, event_created_child,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_handle_v1::{
    self as handle, ZwlrForeignToplevelHandleV1,
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::zwlr_foreign_toplevel_manager_v1::{
    self as manager, ZwlrForeignToplevelManagerV1,
};

/// Placeholder open applications, comma separated `app_id`s.
pub const OPEN_VAR: &str = "DOCK_OPEN";
/// Placeholder active application, an `app_id`.
pub const ACTIVE_VAR: &str = "DOCK_ACTIVE";

/// A window, as the compositor describes it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Window {
    /// Names the window while it exists: the compositor's id on the shell protocol, a number
    /// of the source's own otherwise.
    pub id: u64,
    /// Wayland `app_id`, or X11 class through XWayland.
    pub app_id: String,
    pub title: String,
    /// Has the keyboard focus.
    pub activated: bool,
    pub minimized: bool,
    /// When it last got the keyboard, counted by the source: higher is more recent, 0 for a
    /// window not seen active since the source started.
    pub recency: u64,
    /// Asked to come forward without the right to take the keyboard (lot Q-52): its icon
    /// bounces a few times.
    pub attention: bool,
}

impl Window {
    /// A window of `app_id`, active or not, without title.
    pub fn new(id: u64, app_id: &str, activated: bool) -> Self {
        Self {
            id,
            app_id: app_id.to_owned(),
            activated,
            ..Self::default()
        }
    }
}

/// The open windows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Windows {
    pub list: Vec<Window>,
    /// Whether the windows can be acted on (brought forward, minimized, closed). Not those of
    /// the placeholder: a click starts the application, « Quitter » stays off.
    pub controllable: bool,
    /// Names of the screens a maximized, fullscreen or snapped window is on (shell protocol only).
    pub covered: BTreeSet<String>,
    /// The source can open App Expose on some windows (`show_windows`, version 4 of the shell
    /// protocol).
    pub expose: bool,
}

impl Windows {
    /// Number of windows that `owns` claims, and whether one of them is active.
    pub fn of(&self, owns: impl Fn(&str) -> bool) -> (usize, bool) {
        self.list
            .iter()
            .filter(|w| owns(&w.app_id))
            .fold((0, false), |(n, active), w| (n + 1, active || w.activated))
    }

    /// The windows that `owns` claims, the least recent first: by the last time they got the
    /// keyboard, then in the order they came.
    pub fn owned(&self, owns: impl Fn(&str) -> bool) -> Vec<&Window> {
        let mut owned: Vec<&Window> = self.list.iter().filter(|w| owns(&w.app_id)).collect();
        owned.sort_by_key(|w| w.recency);
        owned
    }
}

/// Recency stamps of a source: a window that gets the keyboard gets the next one.
#[derive(Debug, Default)]
pub struct Stamps(u64);

impl Stamps {
    /// The recency of a window that was `before` active and is `now`: a new stamp when it
    /// just got the keyboard, otherwise `recency` unchanged.
    pub fn follow(&mut self, before: bool, now: bool, recency: u64) -> u64 {
        if now && !before {
            self.0 += 1;
            self.0
        } else {
            recency
        }
    }
}

/// Error from a source of windows.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("cannot connect to the compositor: {0}")]
    Connect(#[from] ConnectError),
    #[error("cannot list the compositor's globals: {0}")]
    Globals(#[from] GlobalError),
    #[error("the compositor does not publish its windows (zwlr_foreign_toplevel_manager_v1): {0}")]
    Unsupported(#[from] BindError),
    #[error("lost the compositor: {0}")]
    Dispatch(#[from] DispatchError),
    #[error("the environment names no shell socket (WAYLAND_DISPLAY, XDG_RUNTIME_DIR)")]
    NoShellSocket,
    #[error("cannot connect to the shell socket: {0}")]
    ShellSocket(std::io::Error),
    #[error("the shell socket does not offer sherlock_shell_v1: {0}")]
    NoShellProtocol(BindError),
}

/// A source of open windows.
pub trait Source: Send + 'static {
    /// Reads the current windows.
    fn read(&mut self) -> Result<Windows, SourceError>;
    /// Waits until they may have changed. Returns `false` once they never will.
    fn wait(&mut self) -> bool;
}

impl<S: Source + ?Sized> Source for Box<S> {
    fn read(&mut self) -> Result<Windows, SourceError> {
        (**self).read()
    }

    fn wait(&mut self) -> bool {
        (**self).wait()
    }
}

/// Fake source: fixed windows, one per open application.
#[derive(Debug, Clone)]
pub struct Placeholder(pub Windows);

impl Placeholder {
    /// `open`: comma separated `app_id`s; `active`: the one with the focus.
    pub fn new(open: &str, active: Option<&str>) -> Self {
        let list = open
            .split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .zip(1..)
            .map(|(app_id, id)| Window::new(id, app_id, active.is_some_and(|a| a.trim() == app_id)))
            .collect();
        Self(Windows {
            list,
            ..Windows::default()
        })
    }

    /// The placeholder of `DOCK_OPEN` and `DOCK_ACTIVE`, if `DOCK_OPEN` is set.
    pub fn from_env() -> Option<Self> {
        let open = std::env::var(OPEN_VAR).ok()?;
        Some(Self::new(&open, std::env::var(ACTIVE_VAR).ok().as_deref()))
    }
}

impl Source for Placeholder {
    fn read(&mut self) -> Result<Windows, SourceError> {
        Ok(self.0.clone())
    }

    fn wait(&mut self) -> bool {
        false
    }
}

/// The windows of a wlroots compositor, from `zwlr_foreign_toplevel_manager_v1`.
pub struct Wlr {
    queue: EventQueue<Toplevels>,
    toplevels: Toplevels,
    // Kept alive: dropping it would not stop the events, but it is the object they come from.
    _manager: ZwlrForeignToplevelManagerV1,
}

impl Wlr {
    /// Connects to the compositor of `WAYLAND_DISPLAY`.
    pub fn connect() -> Result<Self, SourceError> {
        Self::with_connection(Connection::connect_to_env()?)
    }

    /// Binds the manager on `connection` and reads the windows already open.
    pub fn with_connection(connection: Connection) -> Result<Self, SourceError> {
        let (globals, mut queue) = registry_queue_init::<Toplevels>(&connection)?;
        let manager = globals.bind(&queue.handle(), 1..=3, ())?;
        let mut toplevels = Toplevels::default();
        queue.roundtrip(&mut toplevels)?;
        Ok(Self {
            queue,
            toplevels,
            _manager: manager,
        })
    }
}

impl Source for Wlr {
    fn read(&mut self) -> Result<Windows, SourceError> {
        Ok(self.toplevels.windows())
    }

    fn wait(&mut self) -> bool {
        loop {
            if let Err(error) = self.queue.blocking_dispatch(&mut self.toplevels) {
                tracing::warn!(%error, "window list lost");
                return false;
            }
            if std::mem::take(&mut self.toplevels.changed) {
                return true;
            }
            if self.toplevels.finished {
                return false;
            }
        }
    }
}

/// What the dock asks of the windows of an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Bring them forward, the last one with the keyboard.
    Activate,
    Minimize,
    Close,
}

/// What a left click on an icon does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    /// Start the application: it has no window, or its windows cannot be acted on.
    Launch,
    /// Bring these windows forward, in this order: the last one, the most recent, ends on top
    /// with the keyboard.
    Bring(Vec<u64>),
    /// Minimize its windows: already in front with a single window shown (the author's choice,
    /// as on Windows and Plasma).
    Minimize,
}

/// The click on an icon whose application has `windows`, the least recent first
/// (`Windows::owned`), when they can be acted on (`controllable`). As on macOS, an application
/// behind comes forward with all its windows shown, the most recent on top; when they are all
/// minimized, only the most recent one comes back. In front with several windows shown, a
/// click goes to the next one: the least recent, which then becomes the most recent, so that
/// clicks go round them all (lot D-21d, the author's report: « le clic qui passe d'une fenêtre
/// à l'autre »). In front with a single window shown, it is minimized.
pub fn click(windows: &[&Window], controllable: bool) -> Click {
    let Some(last) = windows.last().filter(|_| controllable) else {
        return Click::Launch;
    };
    if windows.iter().any(|w| w.activated) {
        return match windows.iter().find(|w| !w.activated && !w.minimized) {
            Some(next) => Click::Bring(vec![next.id]),
            None => Click::Minimize,
        };
    }
    let shown: Vec<u64> = windows
        .iter()
        .filter(|w| !w.minimized)
        .map(|w| w.id)
        .collect();
    if shown.is_empty() {
        Click::Bring(vec![last.id])
    } else {
        Click::Bring(shown)
    }
}

/// Which windows are an application's own, by `app_id`.
pub type Owns = Box<dyn Fn(&str) -> bool + Send + Sync>;

/// Which windows a request goes to, by `app_id` and title.
type Pick = Box<dyn Fn(&str, &str) -> bool + Send>;

/// Where an icon is on its screen, for the windows of its application.
pub struct Place {
    pub owns: Owns,
    /// Name of the screen (`wl_output` name).
    pub output: String,
    /// The icon at rest, relative to the top left corner of the screen: x, y, width, height.
    pub rectangle: (i32, i32, i32, i32),
}

/// Acts on the windows of an application. `owns` says which windows are its own.
pub trait Control: Send + Sync {
    fn send(&self, request: Request, owns: Owns);

    /// Sends `request` to `window` alone.
    fn send_to(&self, request: Request, window: &Window);

    /// Opens App Expose on the windows of `ids`. Only the shell protocol can.
    fn show_windows(&self, _ids: &[u64]) {}

    /// Tells the compositor where the icon of each window is (`set_rectangle`), for the
    /// minimize animation: the windows of the first place that owns them. Replaces the
    /// previous places, and applies to the windows that come later.
    fn place(&self, _places: Vec<Place>) {}
}

/// Acts through the shell protocol while its source runs, through the wlroots one otherwise.
#[derive(Debug, Default)]
pub struct Auto;

impl Control for Auto {
    fn send(&self, request: Request, owns: Owns) {
        match crate::shell::control() {
            Some(shell) => shell.send(request, owns),
            None => WlrControl.send(request, owns),
        }
    }

    fn send_to(&self, request: Request, window: &Window) {
        match crate::shell::control() {
            Some(shell) => shell.send_to(request, window),
            None => WlrControl.send_to(request, window),
        }
    }

    fn show_windows(&self, ids: &[u64]) {
        if let Some(shell) = crate::shell::control() {
            shell.show_windows(ids);
        }
    }

    fn place(&self, places: Vec<Place>) {
        if let Some(shell) = crate::shell::control() {
            shell.place(places);
        }
    }
}

/// Acts through `zwlr_foreign_toplevel_manager_v1`, on a thread and a connection of its own:
/// the compositor may take its time, and the handles of the watching connection stay on its
/// thread.
#[derive(Debug, Default)]
pub struct WlrControl;

impl WlrControl {
    /// Sends `request` to the windows `pick` takes, on a thread.
    fn spawn(request: Request, pick: Pick, one: bool) {
        let spawned = std::thread::Builder::new()
            .name(String::from("windows-request"))
            .spawn(move || {
                let sent = Connection::connect_to_env()
                    .map_err(SourceError::from)
                    .and_then(|connection| act_where(connection, request, &*pick, one));
                match sent {
                    Ok(count) => tracing::info!(?request, count, "windows asked"),
                    Err(error) => tracing::warn!(?request, %error, "cannot act on the windows"),
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot start the windows request");
        }
    }
}

impl Control for WlrControl {
    fn send(&self, request: Request, owns: Owns) {
        Self::spawn(request, Box::new(move |app_id, _| owns(app_id)), false);
    }

    /// The handles of another connection carry no id: the first window with the same
    /// `app_id` and title.
    fn send_to(&self, request: Request, window: &Window) {
        let (app_id, title) = (window.app_id.clone(), window.title.clone());
        Self::spawn(
            request,
            Box::new(move |a, t| a == app_id && t == title),
            true,
        );
    }
}

/// Fake control: records what is asked, acts on nothing.
#[derive(Debug, Default, Clone)]
pub struct Recorder {
    requests: Arc<Mutex<Vec<Request>>>,
    /// The requests sent to one window, with its id.
    targeted: Arc<Mutex<Vec<(Request, u64)>>>,
    /// Each App Expose asked, with its windows.
    exposed: Arc<Mutex<Vec<Vec<u64>>>>,
    /// Each `place`: the screen and rectangle of each icon, with the first `app_id` of the
    /// probe list it owns.
    places: Arc<Mutex<Vec<Vec<Placed>>>>,
    probe: Vec<String>,
}

/// A place as `Recorder` keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub app_id: Option<String>,
    pub output: String,
    pub rectangle: (i32, i32, i32, i32),
}

impl Recorder {
    /// A recorder that names each place by the first of `app_ids` it owns.
    pub fn probing(app_ids: &[&str]) -> Self {
        Self {
            probe: app_ids.iter().map(|a| String::from(*a)).collect(),
            ..Self::default()
        }
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn targeted(&self) -> Vec<(Request, u64)> {
        self.targeted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn exposed(&self) -> Vec<Vec<u64>> {
        self.exposed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn places(&self) -> Vec<Vec<Placed>> {
        self.places
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Control for Recorder {
    fn send(&self, request: Request, _: Owns) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }

    fn send_to(&self, request: Request, window: &Window) {
        self.targeted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request, window.id));
    }

    fn show_windows(&self, ids: &[u64]) {
        self.exposed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ids.to_vec());
    }

    fn place(&self, places: Vec<Place>) {
        let placed = places
            .into_iter()
            .map(|p| Placed {
                app_id: self.probe.iter().find(|a| (p.owns)(a)).cloned(),
                output: p.output,
                rectangle: p.rectangle,
            })
            .collect();
        self.places
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(placed);
    }
}

/// Sends `request` to the windows that `owns` claims, on `connection`. Returns how many were
/// asked. Activation needs a seat; without one, nothing is activated.
pub fn act(
    connection: Connection,
    request: Request,
    owns: &dyn Fn(&str) -> bool,
) -> Result<usize, SourceError> {
    act_where(connection, request, &|app_id, _| owns(app_id), false)
}

/// Sends `request` to the windows whose `app_id` and title `pick` takes, the first one only
/// with `one`.
fn act_where(
    connection: Connection,
    request: Request,
    pick: &dyn Fn(&str, &str) -> bool,
    one: bool,
) -> Result<usize, SourceError> {
    let (globals, mut queue) = registry_queue_init::<Toplevels>(&connection)?;
    let _manager: ZwlrForeignToplevelManagerV1 = globals.bind(&queue.handle(), 1..=3, ())?;
    let seat: Option<WlSeat> = globals.bind(&queue.handle(), 1..=1, ()).ok();
    let mut toplevels = Toplevels::default();
    queue.roundtrip(&mut toplevels)?;
    let mut count = 0;
    let picked = toplevels
        .list
        .iter()
        .filter(|t| t.done && pick(&t.app_id, &t.title))
        .take(if one { 1 } else { usize::MAX });
    for toplevel in picked {
        match (request, &seat) {
            (Request::Activate, Some(seat)) => toplevel.handle.activate(seat),
            (Request::Activate, None) => continue,
            (Request::Minimize, _) => toplevel.handle.set_minimized(),
            (Request::Close, _) => toplevel.handle.close(),
        }
        count += 1;
    }
    // The compositor has the requests before the connection goes.
    queue.roundtrip(&mut toplevels)?;
    Ok(count)
}

/// The source to use: the placeholder if `DOCK_OPEN` is set, otherwise our compositor's shell
/// protocol, otherwise the wlroots list if the compositor publishes one, otherwise none.
pub fn source() -> Option<Box<dyn Source>> {
    if let Some(placeholder) = Placeholder::from_env() {
        tracing::info!("open windows from {OPEN_VAR} (placeholder)");
        return Some(Box::new(placeholder));
    }
    if let Some(shell) = crate::shell::source() {
        return Some(shell);
    }
    match Wlr::connect() {
        Ok(wlr) => Some(Box::new(wlr)),
        Err(error) => {
            tracing::info!(%error, "open windows unknown");
            None
        }
    }
}

/// Opens the source that `open` gives on a thread, so that connecting never blocks the dock,
/// and turns it into a stream that only carries changes. The thread stops when the stream is
/// dropped, when there is no source, or when the windows will never change again.
pub fn watch(open: fn() -> Option<Box<dyn Source>>) -> impl Stream<Item = Windows> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(String::from("source-windows"))
        .spawn(move || {
            let Some(mut source) = open() else {
                return;
            };
            let mut last = None;
            loop {
                match source.read() {
                    Ok(windows) if last.as_ref() != Some(&windows) => {
                        if sender.unbounded_send(windows.clone()).is_err() {
                            return;
                        }
                        last = Some(windows);
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "cannot read the windows"),
                }
                if !source.wait() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "cannot start the window source");
    }
    receiver
}

/// State of the protocol objects: each toplevel and its properties, applied at `done`.
#[derive(Debug, Default)]
struct Toplevels {
    list: Vec<Toplevel>,
    /// Ids given to the toplevels: this protocol has none.
    next_id: u64,
    stamps: Stamps,
    /// A `done` or a `closed` came since the last `wait`.
    changed: bool,
    /// The compositor stopped sending.
    finished: bool,
}

#[derive(Debug)]
struct Toplevel {
    handle: ZwlrForeignToplevelHandleV1,
    id: u64,
    app_id: String,
    title: String,
    activated: bool,
    minimized: bool,
    recency: u64,
    pending_app_id: Option<String>,
    pending_title: Option<String>,
    /// Activated, minimized.
    pending_state: Option<(bool, bool)>,
    /// The first `done` came: the toplevel is described.
    done: bool,
}

impl Toplevels {
    fn windows(&self) -> Windows {
        Windows {
            list: self
                .list
                .iter()
                .filter(|t| t.done)
                .map(|t| Window {
                    id: t.id,
                    app_id: t.app_id.clone(),
                    title: t.title.clone(),
                    activated: t.activated,
                    minimized: t.minimized,
                    recency: t.recency,
                    attention: false,
                })
                .collect(),
            controllable: true,
            covered: BTreeSet::new(),
            expose: false,
        }
    }
}

/// Whether the `state` array of a toplevel holds `value`. It is a list of `u32` in the
/// machine's byte order.
fn holds(state: &[u8], value: handle::State) -> bool {
    let (values, _) = state.as_chunks::<4>();
    values
        .iter()
        .any(|c| u32::from_ne_bytes(*c) == value as u32)
}

impl Dispatch<WlRegistry, GlobalListContents> for Toplevels {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Toplevels {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for Toplevels {
    fn event(
        state: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Toplevel { toplevel } => {
                state.next_id += 1;
                state.list.push(Toplevel {
                    handle: toplevel,
                    id: state.next_id,
                    app_id: String::new(),
                    title: String::new(),
                    activated: false,
                    minimized: false,
                    recency: 0,
                    pending_app_id: None,
                    pending_title: None,
                    pending_state: None,
                    done: false,
                });
            }
            manager::Event::Finished => state.finished = true,
            _ => {}
        }
    }

    event_created_child!(Toplevels, ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for Toplevels {
    fn event(
        state: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        event: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.list.iter().position(|t| &t.handle == proxy) else {
            return;
        };
        let toplevel = &mut state.list[index];
        match event {
            handle::Event::AppId { app_id } => toplevel.pending_app_id = Some(app_id),
            handle::Event::Title { title } => toplevel.pending_title = Some(title),
            handle::Event::State { state: flags } => {
                let activated = holds(&flags, handle::State::Activated);
                toplevel.pending_state = Some((activated, holds(&flags, handle::State::Minimized)));
            }
            handle::Event::Done => {
                if let Some(app_id) = toplevel.pending_app_id.take() {
                    toplevel.app_id = app_id;
                }
                if let Some(title) = toplevel.pending_title.take() {
                    toplevel.title = title;
                }
                if let Some((activated, minimized)) = toplevel.pending_state.take() {
                    toplevel.recency =
                        state
                            .stamps
                            .follow(toplevel.activated, activated, toplevel.recency);
                    toplevel.activated = activated;
                    toplevel.minimized = minimized;
                }
                toplevel.done = true;
                state.changed = true;
            }
            handle::Event::Closed => {
                state.list.remove(index).handle.destroy();
                state.changed = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_placeholder_reads_its_variables() {
        let Placeholder(windows) = Placeholder::new("org.kde.konsole, firefox,", Some("firefox"));
        assert_eq!(windows.list.len(), 2);
        assert_eq!(windows.of(|id| id == "firefox"), (1, true));
        assert_eq!(windows.of(|id| id == "org.kde.konsole"), (1, false));
        assert_eq!(windows.of(|id| id == "kate"), (0, false));
        assert!(!windows.controllable, "a placeholder acts on nothing");
    }

    /// Three windows of one application: 7 got the keyboard last, 5 before it, 6 never.
    fn three() -> Windows {
        let window = |id, recency| Window {
            recency,
            ..Window::new(id, "app", false)
        };
        Windows {
            list: vec![window(5, 1), window(6, 0), window(7, 2)],
            controllable: true,
            ..Windows::default()
        }
    }

    #[test]
    fn a_click_with_zero_one_or_several_windows() {
        let none = Windows::default();
        assert_eq!(click(&none.owned(|_| true), true), Click::Launch);
        // One window behind: it comes forward; in front: minimized.
        let mut one = Windows {
            list: vec![Window::new(3, "app", false)],
            controllable: true,
            ..Windows::default()
        };
        assert_eq!(click(&one.owned(|_| true), true), Click::Bring(vec![3]));
        one.list[0].activated = true;
        assert_eq!(click(&one.owned(|_| true), true), Click::Minimize);
        // A source that cannot act on its windows: started, never pretended.
        assert_eq!(click(&one.owned(|_| true), false), Click::Launch);
        // Several behind: all come forward, the most recent last, on top.
        let mut several = three();
        let owned = several.owned(|_| true);
        assert_eq!(click(&owned, true), Click::Bring(vec![6, 5, 7]));
        // In front with several shown: the next one, the least recent; with one: minimized.
        let mut front = three();
        front.list[2].activated = true;
        assert_eq!(click(&front.owned(|_| true), true), Click::Bring(vec![6]));
        front.list[1].minimized = true;
        assert_eq!(click(&front.owned(|_| true), true), Click::Bring(vec![5]));
        front.list[0].minimized = true;
        assert_eq!(click(&front.owned(|_| true), true), Click::Minimize);
        // Minimized ones stay down while others are shown; all minimized, the most recent
        // comes back alone.
        several.list[2].minimized = true;
        assert_eq!(
            click(&several.owned(|_| true), true),
            Click::Bring(vec![6, 5])
        );
        several.list.iter_mut().for_each(|w| w.minimized = true);
        assert_eq!(click(&several.owned(|_| true), true), Click::Bring(vec![7]));
    }

    #[test]
    fn a_window_that_gets_the_keyboard_becomes_the_most_recent() {
        let mut stamps = Stamps::default();
        assert_eq!(stamps.follow(false, true, 0), 1);
        assert_eq!(stamps.follow(true, true, 1), 1, "still active: unchanged");
        assert_eq!(stamps.follow(false, false, 1), 1);
        assert_eq!(stamps.follow(false, true, 0), 2);
    }

    #[test]
    fn the_state_array_says_activated_and_minimized() {
        let bytes =
            |values: &[u32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_ne_bytes()).collect() };
        assert!(holds(&bytes(&[0, 2]), handle::State::Activated));
        assert!(!holds(&bytes(&[0, 1, 3]), handle::State::Activated));
        assert!(holds(&bytes(&[1]), handle::State::Minimized));
        assert!(!holds(&[], handle::State::Activated));
    }
}
