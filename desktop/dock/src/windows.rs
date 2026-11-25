//! Open windows, for the dot under open applications, the active one, and « Quitter ».
//!
//! A source implements `Source`, like the bar's sources. Two exist:
//!
//! - `Wlr`, the `zwlr_foreign_toplevel_manager_v1` protocol, on its own Wayland connection. Our
//!   compositor does not offer it yet, nor does KWin; wlroots compositors do.
//! - `Placeholder`, a fake list set by two environment variables, for tests and screenshots.
//!
//! Without either, the dock does not know the windows: no dot, « Quitter » disabled. What our
//! compositor will have to provide is in `docs/dock.md`.

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    /// Wayland `app_id`, or X11 class through XWayland.
    pub app_id: String,
    /// Has the keyboard focus.
    pub activated: bool,
}

/// The open windows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Windows {
    pub list: Vec<Window>,
    /// Whether the source can close windows. The placeholder cannot: « Quitter » stays off.
    pub closable: bool,
}

impl Windows {
    /// Number of windows that `owns` claims, and whether one of them is active.
    pub fn of(&self, owns: impl Fn(&str) -> bool) -> (usize, bool) {
        self.list
            .iter()
            .filter(|w| owns(&w.app_id))
            .fold((0, false), |(n, active), w| (n + 1, active || w.activated))
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
            .map(|app_id| Window {
                app_id: app_id.to_owned(),
                activated: active.is_some_and(|a| a.trim() == app_id),
            })
            .collect();
        Self(Windows {
            list,
            closable: false,
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

/// Closes the windows that `owns` claims, on a connection of its own: the handles of the
/// watching connection stay on its thread. Returns how many were asked to close.
pub fn close(connection: Connection, owns: &dyn Fn(&str) -> bool) -> Result<usize, SourceError> {
    let mut source = Wlr::with_connection(connection)?;
    let mut count = 0;
    for toplevel in source.toplevels.list.iter().filter(|t| t.done) {
        if owns(&toplevel.app_id) {
            toplevel.handle.close();
            count += 1;
        }
    }
    // The compositor has the requests before the connection goes.
    source.queue.roundtrip(&mut source.toplevels)?;
    Ok(count)
}

/// The source to use: the placeholder if `DOCK_OPEN` is set, otherwise the compositor's list if
/// it publishes one, otherwise none.
pub fn source() -> Option<Box<dyn Source>> {
    if let Some(placeholder) = Placeholder::from_env() {
        tracing::info!("open windows from {OPEN_VAR} (placeholder)");
        return Some(Box::new(placeholder));
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
    /// A `done` or a `closed` came since the last `wait`.
    changed: bool,
    /// The compositor stopped sending.
    finished: bool,
}

#[derive(Debug)]
struct Toplevel {
    handle: ZwlrForeignToplevelHandleV1,
    app_id: String,
    activated: bool,
    pending_app_id: Option<String>,
    pending_activated: Option<bool>,
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
                    app_id: t.app_id.clone(),
                    activated: t.activated,
                })
                .collect(),
            closable: true,
        }
    }
}

/// Whether the `state` array of a toplevel holds `activated`. It is a list of `u32` in the
/// machine's byte order.
fn is_activated(state: &[u8]) -> bool {
    let (values, _) = state.as_chunks::<4>();
    values
        .iter()
        .any(|c| u32::from_ne_bytes(*c) == handle::State::Activated as u32)
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
            manager::Event::Toplevel { toplevel } => state.list.push(Toplevel {
                handle: toplevel,
                app_id: String::new(),
                activated: false,
                pending_app_id: None,
                pending_activated: None,
                done: false,
            }),
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
            handle::Event::State { state: flags } => {
                toplevel.pending_activated = Some(is_activated(&flags));
            }
            handle::Event::Done => {
                if let Some(app_id) = toplevel.pending_app_id.take() {
                    toplevel.app_id = app_id;
                }
                if let Some(activated) = toplevel.pending_activated.take() {
                    toplevel.activated = activated;
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
        assert!(!windows.closable, "a placeholder closes nothing");
    }

    #[test]
    fn the_state_array_says_activated() {
        let bytes =
            |values: &[u32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_ne_bytes()).collect() };
        assert!(is_activated(&bytes(&[0, 2])));
        assert!(!is_activated(&bytes(&[0, 1, 3])));
        assert!(!is_activated(&[]));
    }
}
