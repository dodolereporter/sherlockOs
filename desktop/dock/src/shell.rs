//! The windows of our compositor, read and acted on through `sherlock_shell_v1` on its shell
//! socket (ADR 0021): the dots, the active application, the screens a maximized, fullscreen or
//! snapped window covers (for `AutoHide=smart`), a click that brings forward or minimizes, « Quitter », and the
//! rectangle of each window's icon (`set_rectangle`, for the minimize animation).
//!
//! Requests go out on the watching connection, from any thread, while the watching thread
//! waits for events: nothing waits for the compositor, a click never blocks.

use std::collections::BTreeMap;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use shell_protocol::client::sherlock_shell_manager_v1::{self as manager, SherlockShellManagerV1};
use shell_protocol::client::sherlock_shell_toplevel_v1::{
    self as toplevel, SherlockShellToplevelV1,
};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child};

use crate::windows::{Control, Owns, Place, Request, Source, SourceError, Stamps, Window, Windows};

/// The control of the running shell source, for `windows::Auto`.
static CONTROL: Mutex<Option<ShellControl>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The control of the running shell source, if one runs.
pub fn control() -> Option<ShellControl> {
    lock(&CONTROL).clone()
}

/// The windows of the compositor's shell socket.
pub struct Shell {
    queue: EventQueue<Toplevels>,
    toplevels: Toplevels,
    control: ShellControl,
    _manager: SherlockShellManagerV1,
}

impl Shell {
    /// Connects to the shell socket of the session (`shell_protocol::socket_from_env`).
    pub fn connect() -> Result<Self, SourceError> {
        let path = shell_protocol::socket_from_env().ok_or(SourceError::NoShellSocket)?;
        let stream = UnixStream::connect(&path).map_err(SourceError::ShellSocket)?;
        Self::with_connection(Connection::from_socket(stream)?)
    }

    /// Binds the manager on `connection` and reads the windows already open.
    pub fn with_connection(connection: Connection) -> Result<Self, SourceError> {
        let (globals, mut queue) = registry_queue_init::<Toplevels>(&connection)?;
        let manager: SherlockShellManagerV1 = globals
            .bind(&queue.handle(), 1..=5, ())
            .map_err(SourceError::NoShellProtocol)?;
        // App Expose came with version 4, the attention state with version 5.
        let expose = (manager.version() >= 4).then(|| manager.clone());
        let control = ShellControl {
            connection,
            shared: Arc::new(Mutex::new(Shared::default())),
            manager: expose,
        };
        let mut toplevels = Toplevels {
            control: Some(control.clone()),
            ..Toplevels::default()
        };
        queue.roundtrip(&mut toplevels)?;
        // The windows already open are the first `read`; `wait` waits for what comes next.
        toplevels.changed = false;
        Ok(Self {
            queue,
            toplevels,
            control,
            _manager: manager,
        })
    }

    /// Acts on the windows of this connection, from any thread.
    pub fn control(&self) -> ShellControl {
        self.control.clone()
    }
}

/// The shell source, if the compositor has a shell socket; otherwise nothing. Its control
/// becomes the one of `windows::Auto`.
pub fn source() -> Option<Box<dyn Source>> {
    match Shell::connect() {
        Ok(shell) => {
            *lock(&CONTROL) = Some(shell.control());
            tracing::info!("open windows from the shell protocol");
            Some(Box::new(shell))
        }
        Err(error) => {
            tracing::info!(%error, "no shell protocol");
            None
        }
    }
}

impl Source for Shell {
    fn read(&mut self) -> Result<Windows, SourceError> {
        let list = self
            .toplevels
            .list
            .iter()
            .filter(|t| t.described)
            .map(|t| Window {
                id: t.id.into(),
                app_id: t.app_id.clone(),
                title: t.title.clone(),
                activated: t.activated,
                minimized: t.minimized,
                recency: t.recency,
                attention: t.attention,
            })
            .collect();
        let covered = self
            .toplevels
            .outputs
            .iter()
            .filter(|(_, covered)| **covered)
            .map(|(name, _)| name.clone())
            .collect();
        Ok(Windows {
            list,
            controllable: true,
            covered,
            expose: self.control.manager.is_some(),
        })
    }

    fn wait(&mut self) -> bool {
        loop {
            if let Err(error) = self.queue.blocking_dispatch(&mut self.toplevels) {
                tracing::warn!(%error, "shell window list lost");
                lock(&CONTROL).take();
                return false;
            }
            if std::mem::take(&mut self.toplevels.changed) {
                return true;
            }
        }
    }
}

/// Sends requests on the watching connection.
#[derive(Clone)]
pub struct ShellControl {
    connection: Connection,
    shared: Arc<Mutex<Shared>>,
    /// The manager, when it offers `show_windows` (version 4).
    manager: Option<SherlockShellManagerV1>,
}

/// What the requests need, kept by the watching thread at each `done`.
#[derive(Default)]
struct Shared {
    /// The described windows.
    windows: Vec<Handle>,
    /// The icons, from the last `place`.
    places: Vec<Place>,
}

struct Handle {
    toplevel: SherlockShellToplevelV1,
    id: u32,
    app_id: String,
    /// The screen and rectangle last sent with `set_rectangle`.
    sent: Option<(String, (i32, i32, i32, i32))>,
}

impl Shared {
    /// Sends the rectangle of each window whose icon moved, appeared or went.
    fn place(&mut self) {
        for window in &mut self.windows {
            let wanted = self
                .places
                .iter()
                .find(|p| (p.owns)(&window.app_id))
                .map(|p| (p.output.clone(), p.rectangle));
            if wanted == window.sent {
                continue;
            }
            match (&wanted, &window.sent) {
                (Some((output, (x, y, width, height))), _) => {
                    window
                        .toplevel
                        .set_rectangle(output.clone(), *x, *y, *width, *height);
                }
                // No icon any more: a size of 0 removes the rectangle.
                (None, Some((output, _))) => {
                    window.toplevel.set_rectangle(output.clone(), 0, 0, 0, 0);
                }
                (None, None) => {}
            }
            window.sent = wanted;
        }
    }
}

impl ShellControl {
    fn flush(&self) {
        if let Err(error) = self.connection.flush() {
            tracing::warn!(%error, "cannot send to the compositor");
        }
    }
}

impl Control for ShellControl {
    fn send(&self, request: Request, owns: Owns) {
        let shared = lock(&self.shared);
        let mut count = 0;
        for window in shared.windows.iter().filter(|w| owns(&w.app_id)) {
            match request {
                Request::Activate => window.toplevel.activate(),
                Request::Minimize => window.toplevel.minimize(),
                Request::Close => window.toplevel.close(),
            }
            count += 1;
        }
        drop(shared);
        self.flush();
        tracing::info!(?request, count, "windows asked");
    }

    fn send_to(&self, request: Request, window: &Window) {
        let shared = lock(&self.shared);
        let Some(handle) = shared.windows.iter().find(|w| u64::from(w.id) == window.id) else {
            tracing::info!(?request, id = window.id, "the window is gone");
            return;
        };
        match request {
            Request::Activate => handle.toplevel.activate(),
            Request::Minimize => handle.toplevel.minimize(),
            Request::Close => handle.toplevel.close(),
        }
        drop(shared);
        self.flush();
        tracing::info!(?request, id = window.id, "window asked");
    }

    fn show_windows(&self, ids: &[u64]) {
        let Some(manager) = &self.manager else {
            tracing::info!("the compositor has no App Expose (sherlock_shell_v1 version 4)");
            return;
        };
        let bytes: Vec<u8> = ids
            .iter()
            .filter_map(|id| u32::try_from(*id).ok())
            .flat_map(u32::to_ne_bytes)
            .collect();
        manager.show_windows(bytes);
        self.flush();
        tracing::info!(?ids, "app expose asked");
    }

    fn place(&self, places: Vec<Place>) {
        let mut shared = lock(&self.shared);
        shared.places = places;
        shared.place();
        drop(shared);
        self.flush();
    }
}

/// The windows and screens as the protocol describes them, applied at each `done` of the
/// manager.
#[derive(Default)]
struct Toplevels {
    list: Vec<Toplevel>,
    stamps: Stamps,
    /// Each screen, and whether a maximized or fullscreen window covers it.
    outputs: BTreeMap<String, bool>,
    /// Screens changed in this batch; `None` for a screen gone.
    pending_outputs: BTreeMap<String, Option<bool>>,
    /// A `done` came since the last `wait`: only it publishes the state.
    changed: bool,
    control: Option<ShellControl>,
}

#[derive(Debug)]
struct Toplevel {
    handle: SherlockShellToplevelV1,
    id: u32,
    app_id: String,
    title: String,
    activated: bool,
    minimized: bool,
    /// Asks for attention (version 5, lot Q-52).
    attention: bool,
    recency: u64,
    pending_app_id: Option<String>,
    pending_title: Option<String>,
    /// Activated, minimized, asks for attention.
    pending_state: Option<(bool, bool, bool)>,
    /// A `done` came after it appeared.
    described: bool,
}

impl Toplevels {
    /// The batch is complete: the windows the requests act on, and their rectangles.
    fn settle(&mut self) {
        let Some(control) = &self.control else {
            return;
        };
        let mut shared = lock(&control.shared);
        let mut old = std::mem::take(&mut shared.windows);
        shared.windows = self
            .list
            .iter()
            .filter(|t| t.described)
            .map(|t| Handle {
                toplevel: t.handle.clone(),
                id: t.id,
                app_id: t.app_id.clone(),
                sent: old
                    .iter_mut()
                    .find(|h| h.toplevel == t.handle)
                    .and_then(|h| h.sent.take()),
            })
            .collect();
        shared.place();
        drop(shared);
        control.flush();
    }
}

/// Whether a `state` array, `u32` values in the machine's byte order, holds `value`.
fn holds(state: &[u8], value: u32) -> bool {
    let (values, _) = state.as_chunks::<4>();
    values.iter().any(|c| u32::from_ne_bytes(*c) == value)
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

impl Dispatch<SherlockShellManagerV1, ()> for Toplevels {
    fn event(
        state: &mut Self,
        _: &SherlockShellManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            manager::Event::Toplevel { toplevel } => state.list.push(Toplevel {
                handle: toplevel,
                id: 0,
                app_id: String::new(),
                title: String::new(),
                activated: false,
                minimized: false,
                attention: false,
                recency: 0,
                pending_app_id: None,
                pending_title: None,
                pending_state: None,
                described: false,
            }),
            manager::Event::Output { name, state: flags } => {
                // A snapped window covers the screen like a maximized one: it takes the whole
                // height once the dock hides (version 2 of the protocol).
                let covered = [
                    manager::OutputState::Maximized,
                    manager::OutputState::Fullscreen,
                    manager::OutputState::Snapped,
                ]
                .into_iter()
                .any(|flag| holds(&flags, flag as u32));
                state.pending_outputs.insert(name, Some(covered));
            }
            manager::Event::OutputRemoved { name } => {
                state.pending_outputs.insert(name, None);
            }
            manager::Event::Done => {
                for toplevel in &mut state.list {
                    if let Some(app_id) = toplevel.pending_app_id.take() {
                        toplevel.app_id = app_id;
                    }
                    if let Some(title) = toplevel.pending_title.take() {
                        toplevel.title = title;
                    }
                    if let Some((activated, minimized, attention)) = toplevel.pending_state.take() {
                        toplevel.recency =
                            state
                                .stamps
                                .follow(toplevel.activated, activated, toplevel.recency);
                        toplevel.activated = activated;
                        toplevel.minimized = minimized;
                        toplevel.attention = attention;
                    }
                    toplevel.described = true;
                }
                for (name, covered) in std::mem::take(&mut state.pending_outputs) {
                    match covered {
                        Some(covered) => state.outputs.insert(name, covered),
                        None => state.outputs.remove(&name),
                    };
                }
                state.settle();
                state.changed = true;
            }
            _ => {}
        }
    }

    event_created_child!(Toplevels, SherlockShellManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (SherlockShellToplevelV1, ()),
    ]);
}

impl Dispatch<SherlockShellToplevelV1, ()> for Toplevels {
    fn event(
        state: &mut Self,
        proxy: &SherlockShellToplevelV1,
        event: toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.list.iter().position(|t| &t.handle == proxy) else {
            return;
        };
        let toplevel = &mut state.list[index];
        match event {
            toplevel::Event::Id { id } => toplevel.id = id,
            toplevel::Event::AppId { app_id } => toplevel.pending_app_id = Some(app_id),
            toplevel::Event::Title { title } => toplevel.pending_title = Some(title),
            toplevel::Event::State { state: flags } => {
                let activated = holds(&flags, toplevel::State::Activated as u32);
                let minimized = holds(&flags, toplevel::State::Minimized as u32);
                let attention = holds(&flags, toplevel::State::Attention as u32);
                toplevel.pending_state = Some((activated, minimized, attention));
            }
            // Gone, but published with the rest of its batch: only `done` makes a consistent
            // state (a window closing and another opening must not read as one window less).
            toplevel::Event::Closed => state.list.remove(index).handle.destroy(),
            _ => {}
        }
    }
}
