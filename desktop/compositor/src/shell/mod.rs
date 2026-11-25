//! Data and control for the programs of the shell (bar, dock): the private `sherlock_shell_v1`
//! protocol (`toplevels.rs`) and the workspaces through `ext-workspace-v1` (`workspaces.rs`).
//! Both are offered only to the clients of the shell socket (ADR 0021).
//!
//! Nothing in the rest of the compositor calls this module when a window changes: after each
//! loop iteration, `State::refresh_shell` reads the windows and the screens, and each bound
//! client receives what changed since it was last told.
//!
//! Minimizing lives here too: a minimized window leaves the space, so that rendering, input and
//! snapping no longer see it, and comes back at the same place when activated.

mod toplevels;
mod workspaces;

use std::{
    os::unix::net::UnixStream,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use smithay::{
    desktop::Window,
    reexports::{
        calloop::{EventLoop, Interest, Mode as LoopMode, PostAction, generic::Generic},
        wayland_server::{Client, DisplayHandle, ListeningSocket},
    },
    utils::{IsAlive, Logical, Rectangle},
    wayland::seat::WaylandFocus,
};

use crate::{ClientState, State, is_override_redirect, window_data};

/// What the module keeps between two refreshes.
#[derive(Default)]
pub(crate) struct Shell {
    /// Bound `sherlock_shell_manager_v1`, with what each one was told.
    managers: Vec<toplevels::Manager>,
    /// Bound `ext_workspace_manager_v1`, with what each one was told.
    workspaces: Vec<workspaces::Manager>,
    /// Minimized windows, out of the space, with the geometry and screen they had.
    minimized: Vec<Minimized>,
    /// Rectangle of the dock icon of each window, by window id: screen name and rectangle on it.
    icons: Vec<(u32, String, Rectangle<i32, Logical>)>,
    /// Listening shell socket in the loop, removed by `State::shutdown`.
    listener: Option<smithay::reexports::calloop::RegistrationToken>,
}

struct Minimized {
    window: Window,
    geometry: Rectangle<i32, Logical>,
    output: String,
}

impl Shell {
    /// Creates the globals of the shell: only clients of the shell socket see them.
    pub(crate) fn new(dh: &DisplayHandle) -> Self {
        toplevels::create_global(dh);
        workspaces::create_global(dh);
        Self::default()
    }
}

/// Whether `client` connected through the shell socket. XWayland, whose client data is
/// smithay's, never did.
fn is_shell_client(client: &Client) -> bool {
    client.get_data::<ClientState>().is_some_and(|d| d.shell)
}

/// Stable id of a window, given at the first look and never given again while the compositor
/// runs.
#[derive(Debug, Clone, Copy)]
struct WindowId(u32);

fn window_id(window: &Window) -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    window
        .user_data()
        .get_or_insert(|| WindowId(NEXT.fetch_add(1, Ordering::Relaxed)))
        .0
}

impl State {
    /// Accepts the clients of the shell on a socket at `path`. They get every global of the
    /// ordinary socket, plus those of this module.
    pub fn listen_shell_at(
        &mut self,
        path: &Path,
        event_loop: &EventLoop<'static, State>,
    ) -> Result<(), crate::Error> {
        let socket = ListeningSocket::bind_absolute(path.to_path_buf())?;
        let source = Generic::new(socket, Interest::READ, LoopMode::Level);
        let token = event_loop
            .handle()
            .insert_source(source, |_, socket, state| {
                while let Some(stream) = socket.accept()? {
                    state.insert_shell_client(stream);
                }
                Ok(PostAction::Continue)
            })
            .map_err(|err| err.error)?;
        self.shell.listener = Some(token);
        tracing::info!(?path, "shell socket open");
        Ok(())
    }

    /// Opens the shell socket next to the Wayland socket at `socket`. Without it the session
    /// goes on, and the shell programs show no window data.
    pub(crate) fn listen_shell_next_to(
        &mut self,
        socket: &Path,
        event_loop: &EventLoop<'static, State>,
    ) {
        let path = shell_protocol::socket_path(socket);
        if let Err(err) = self.listen_shell_at(&path, event_loop) {
            tracing::warn!(?path, "no shell socket: {err}");
        }
    }

    /// Removes the shell socket from the loop, which deletes it and its lock.
    pub(crate) fn stop_shell_socket(&mut self) {
        if let Some(token) = self.shell.listener.take() {
            self.loop_handle.remove(token);
        }
    }

    /// Plugs in an already connected client of the shell.
    pub fn insert_shell_client(&mut self, stream: UnixStream) {
        let data = Arc::new(ClientState {
            shell: true,
            ..ClientState::default()
        });
        if let Err(err) = self.display_handle.insert_client(stream, data) {
            tracing::warn!("shell client refused: {err}");
        }
    }

    /// Whether `client` sees the globals of the shell: it connected through the shell socket.
    pub fn is_shell_client(client: &Client) -> bool {
        is_shell_client(client)
    }

    /// Rectangle of the dock icon that the shell gave for `window`: screen name and rectangle
    /// on that screen. For the minimize animation (lot D-19).
    pub fn icon_rectangle(&self, window: &Window) -> Option<(String, Rectangle<i32, Logical>)> {
        let id = window_id(window);
        let icon = self.shell.icons.iter().find(|i| i.0 == id)?;
        Some((icon.1.clone(), icon.2))
    }

    /// Whether `window` is minimized.
    pub fn is_minimized(&self, window: &Window) -> bool {
        self.shell.minimized.iter().any(|m| &m.window == window)
    }

    /// Hides `window` until it is activated. If it was the active one, the window on top
    /// becomes active.
    pub fn minimize(&mut self, window: &Window) {
        let Some(geometry) = self.space.element_geometry(window) else {
            return;
        };
        if is_override_redirect(window) {
            return;
        }
        let output = self.window_output(window).map(|o| o.name());
        let active = is_activated(window) || self.focused_window().as_ref() == Some(window);
        self.space.unmap_elem(window);
        window.set_activated(false);
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_pending_configure();
        }
        if let Some(surface) = window.x11_surface()
            && let Err(err) = surface.set_suspended(true)
        {
            tracing::warn!("hiding the X11 window: {err}");
        }
        self.shell.minimized.push(Minimized {
            window: window.clone(),
            geometry,
            output: output.unwrap_or_default(),
        });
        if active {
            self.focus(self.top_window());
        }
    }

    /// Brings `window` back if minimized, raises it and gives it the keyboard.
    pub fn activate(&mut self, window: &Window) {
        if let Some(i) = self
            .shell
            .minimized
            .iter()
            .position(|m| &m.window == window)
        {
            let minimized = self.shell.minimized.remove(i);
            if let Some(surface) = window.x11_surface()
                && let Err(err) = surface.set_suspended(false)
            {
                tracing::warn!("showing the X11 window: {err}");
            }
            self.move_window(window, minimized.geometry.loc, false);
        }
        if self.space.elements().any(|w| w == window) {
            self.focus(Some(window.clone()));
        }
    }

    /// Asks the application of `window` to close it.
    fn close(window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        } else if let Some(Err(err)) = window.x11_surface().map(|s| s.close()) {
            tracing::warn!("closing the X11 window: {err}");
        }
    }

    /// The window of id `id`, shown or minimized.
    fn window_by_id(&self, id: u32) -> Option<Window> {
        let minimized = self.shell.minimized.iter().map(|m| &m.window);
        self.space
            .elements()
            .chain(minimized)
            .find(|w| window_id(w) == id)
            .cloned()
    }

    /// Tells the clients of the shell what changed since the last call. Called after each loop
    /// iteration, before the replies are flushed.
    pub(crate) fn refresh_shell(&mut self) {
        // A window destroyed, or withdrawn by its X11 client, while minimized is forgotten.
        self.shell
            .minimized
            .retain(|m| m.window.alive() && m.window.x11_surface().is_none_or(|s| s.is_mapped()));
        self.shell.managers.retain(|m| m.alive());
        self.shell.workspaces.retain(|m| m.alive());
        if !self.shell.managers.is_empty() {
            let (windows, outputs) = self.snapshot();
            let ids: Vec<u32> = windows.iter().map(|w| w.0).collect();
            self.shell.icons.retain(|i| ids.contains(&i.0));
            let dh = self.display_handle.clone();
            for manager in &mut self.shell.managers {
                manager.update(&dh, &windows, &outputs);
            }
        }
        if !self.shell.workspaces.is_empty() {
            let outputs: Vec<_> = self.space.outputs().cloned().collect();
            let dh = self.display_handle.clone();
            for manager in &mut self.shell.workspaces {
                manager.update(&dh, &outputs);
            }
        }
    }

    /// Every window the shell sees, and the state of every screen.
    fn snapshot(&self) -> (toplevels::Windows, toplevels::Outputs) {
        use toplevels::{OutputState, ToplevelState};
        let shown = self
            .space
            .elements()
            .filter(|w| !is_override_redirect(w) && window_data(w).shown.get())
            .filter_map(|w| {
                let output = self.window_output(w)?;
                let screen = self.space.output_geometry(&output)?;
                let mut geometry = self.space.element_geometry(w)?;
                geometry.loc -= screen.loc;
                Some((w, output.name(), geometry, false))
            });
        let minimized = self.shell.minimized.iter().filter_map(|m| {
            let screen = self.space.outputs().find(|o| o.name() == m.output);
            let mut geometry = m.geometry;
            geometry.loc -= screen.and_then(|o| self.space.output_geometry(o))?.loc;
            Some((&m.window, m.output.clone(), geometry, true))
        });
        let windows: Vec<_> = shown
            .chain(minimized)
            .map(|(window, output, geometry, minimized)| {
                let mut states = Vec::new();
                let arrangement = crate::handlers::Arrangement::of(window);
                let flags = [
                    (ToplevelState::Maximized, arrangement.maximized),
                    (ToplevelState::Fullscreen, arrangement.fullscreen),
                    (
                        ToplevelState::Snapped,
                        window_data(window).zone.get().is_some(),
                    ),
                    (ToplevelState::Minimized, minimized),
                    (ToplevelState::Activated, !minimized && is_activated(window)),
                ];
                states.extend(flags.iter().filter(|f| f.1).map(|f| f.0 as u32));
                let snapshot = toplevels::Snapshot {
                    title: State::title(window).unwrap_or_default(),
                    app_id: app_id(window),
                    states,
                    output,
                    geometry,
                };
                (window_id(window), snapshot)
            })
            .collect();
        let outputs = self
            .space
            .outputs()
            .map(|output| {
                let name = output.name();
                let mut state = Vec::new();
                for (flag, value) in [
                    (OutputState::Maximized, ToplevelState::Maximized),
                    (OutputState::Fullscreen, ToplevelState::Fullscreen),
                ] {
                    let on = windows.iter().any(|(_, w)| {
                        w.output == name
                            && w.states.contains(&(value as u32))
                            && !w.states.contains(&(ToplevelState::Minimized as u32))
                    });
                    if on {
                        state.push(flag as u32);
                    }
                }
                (name, state)
            })
            .collect();
        (windows, outputs)
    }
}

/// The window is the active one: the last that was given the keyboard. It stays so while an
/// anchored surface has the keyboard.
fn is_activated(window: &Window) -> bool {
    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    match (window.toplevel(), window.x11_surface()) {
        (Some(toplevel), _) => {
            toplevel.with_pending_state(|s| s.states.contains(XdgState::Activated))
        }
        (None, Some(surface)) => surface.is_activated(),
        (None, None) => false,
    }
}

/// The xdg-shell app_id, or the class of an X11 window.
fn app_id(window: &Window) -> String {
    if let Some(surface) = window.x11_surface() {
        return surface.class();
    }
    let Some(surface) = window.wl_surface() else {
        return String::new();
    };
    smithay::wayland::compositor::with_states(&surface, |states| {
        states
            .data_map
            .get::<smithay::wayland::shell::xdg::XdgToplevelSurfaceData>()
            .and_then(|data| data.lock().ok()?.app_id.clone())
    })
    .unwrap_or_default()
}
