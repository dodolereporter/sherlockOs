//! The windows of our compositor, read through `sherlock_shell_v1` on its shell socket (ADR
//! 0021), for the bounce of an application that starts: it ends when the application's first
//! window comes.
//!
//! Read only, for now: the dots, a click that brings forward or minimizes, and « Quitter »
//! over this protocol are batch D-18c. The source says its windows cannot be acted on, so
//! nothing of the dock depends on it beyond the bounce.

use std::os::unix::net::UnixStream;

use shell_protocol::client::sherlock_shell_manager_v1::{self as manager, SherlockShellManagerV1};
use shell_protocol::client::sherlock_shell_toplevel_v1::{
    self as toplevel, SherlockShellToplevelV1,
};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, event_created_child};

use crate::windows::{Source, SourceError, Window, Windows};

/// The windows of the compositor's shell socket.
pub struct Shell {
    queue: EventQueue<Toplevels>,
    toplevels: Toplevels,
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
        let manager = globals
            .bind(&queue.handle(), 1..=1, ())
            .map_err(SourceError::NoShellProtocol)?;
        let mut toplevels = Toplevels::default();
        queue.roundtrip(&mut toplevels)?;
        // The windows already open are the first `read`; `wait` waits for what comes next.
        toplevels.changed = false;
        Ok(Self {
            queue,
            toplevels,
            _manager: manager,
        })
    }
}

/// The shell source, if the compositor has a shell socket; otherwise nothing.
pub fn source() -> Option<Box<dyn Source>> {
    match Shell::connect() {
        Ok(shell) => Some(Box::new(shell)),
        Err(error) => {
            tracing::info!(%error, "no shell protocol: a started application bounces until its timeout");
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
                app_id: t.app_id.clone(),
                activated: t.activated,
            })
            .collect();
        Ok(Windows {
            list,
            controllable: false,
        })
    }

    fn wait(&mut self) -> bool {
        loop {
            if let Err(error) = self.queue.blocking_dispatch(&mut self.toplevels) {
                tracing::warn!(%error, "shell window list lost");
                return false;
            }
            if std::mem::take(&mut self.toplevels.changed) {
                return true;
            }
        }
    }
}

/// The windows as the protocol describes them, applied at each `done` of the manager.
#[derive(Debug, Default)]
struct Toplevels {
    list: Vec<Toplevel>,
    /// A `done` came since the last `wait`: only it publishes the state.
    changed: bool,
}

#[derive(Debug)]
struct Toplevel {
    handle: SherlockShellToplevelV1,
    app_id: String,
    activated: bool,
    pending_app_id: Option<String>,
    pending_activated: Option<bool>,
    /// A `done` came after it appeared.
    described: bool,
}

/// Whether a `state` array, `u32` values in the machine's byte order, holds `activated`.
fn is_activated(state: &[u8]) -> bool {
    let (values, _) = state.as_chunks::<4>();
    values
        .iter()
        .any(|c| u32::from_ne_bytes(*c) == toplevel::State::Activated as u32)
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
                app_id: String::new(),
                activated: false,
                pending_app_id: None,
                pending_activated: None,
                described: false,
            }),
            manager::Event::Done => {
                for toplevel in &mut state.list {
                    if let Some(app_id) = toplevel.pending_app_id.take() {
                        toplevel.app_id = app_id;
                    }
                    if let Some(activated) = toplevel.pending_activated.take() {
                        toplevel.activated = activated;
                    }
                    toplevel.described = true;
                }
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
            toplevel::Event::AppId { app_id } => toplevel.pending_app_id = Some(app_id),
            toplevel::Event::State { state: flags } => {
                toplevel.pending_activated = Some(is_activated(&flags));
            }
            // Gone, but published with the rest of its batch: only `done` makes a consistent
            // state (a window closing and another opening must not read as one window less).
            toplevel::Event::Closed => state.list.remove(index).handle.destroy(),
            _ => {}
        }
    }
}
