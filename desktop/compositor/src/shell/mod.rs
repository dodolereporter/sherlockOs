//! Data and control for the programs of the shell (bar, dock): the private `sherlock_shell_v1`
//! protocol (`toplevels.rs`) and the workspaces through `ext-workspace-v1` (`workspaces.rs`).
//! Both are offered only to the clients of the shell socket (ADR 0021).
//!
//! Nothing in the rest of the compositor calls this module when a window changes: after each
//! loop iteration, `State::refresh_shell` reads the windows and the screens, and each bound
//! client receives what changed since it was last told.
//!
//! The workspaces live here too (`workspaces::Workspaces`, lot D-18w): each window belongs to
//! one, and only the windows of the active one are shown, as are the windows that are not
//! minimized. Hidden windows stay in the space, at their place: they keep their commits, their
//! screen and their arrangement, and rendering, input and focus skip them (`State::is_visible`).

mod toplevels;
mod workspaces;

use std::{
    cell::RefCell,
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
    utils::{IsAlive, Logical, Point, Rectangle},
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
    /// Minimized windows: hidden, at their place in the space.
    minimized: Vec<Window>,
    /// Rectangle of the dock icon of each window, by window id: screen name and rectangle on it.
    icons: Vec<(u32, String, Rectangle<i32, Logical>)>,
    /// Listening shell socket in the loop, removed by `State::shutdown`.
    listener: Option<smithay::reexports::calloop::RegistrationToken>,
    /// The workspaces and the active one.
    workspace_model: workspaces::Workspaces,
    /// Slide from one workspace to the next.
    slide: crate::anim::WorkspaceSlide,
}

/// Workspace of a window, by id: the active one when the window shows up.
struct WindowWorkspace(RefCell<String>);

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

    /// Keeps `rect`, on the screen named `output`, as the dock icon of `window`, or forgets it
    /// (`None`): what `set_rectangle` does, for the tests that have no shell client.
    pub fn set_icon_rectangle(
        &mut self,
        window: &Window,
        output: &str,
        rect: Option<Rectangle<i32, Logical>>,
    ) {
        let id = window_id(window);
        self.shell.icons.retain(|i| i.0 != id);
        if let Some(rect) = rect.filter(|r| !r.size.is_empty()) {
            self.shell.icons.push((id, output.to_owned(), rect));
        }
    }

    /// The workspaces, in order, and the active one.
    pub fn workspaces(&self) -> &workspaces::Workspaces {
        &self.shell.workspace_model
    }

    /// Makes the workspace `id` active: its windows show up and the others slide away. The
    /// keyboard goes to the window of that workspace that had it last, and a window being
    /// dragged comes along.
    pub fn activate_workspace(&mut self, id: &str) {
        self.queue_redraw();
        if self.switch_workspace(id) {
            self.focus(self.top_window());
            self.refresh_pointer_focus();
        }
    }

    /// Makes the workspace `id` active, without touching the keyboard nor the pointer. `false`
    /// if it already was, or does not exist.
    fn switch_workspace(&mut self, id: &str) -> bool {
        let model = &mut self.shell.workspace_model;
        let from = model.active_index();
        if !model.activate(id) || model.active_index() == from {
            return false;
        }
        tracing::debug!(id, "workspace activated");
        let now = self.start_time.elapsed();
        let to = model.active_index();
        self.shell.slide.go(now, from, to);
        if let Some(window) = self.gesture_window() {
            *self.window_workspace(&window).0.borrow_mut() = id.to_owned();
        }
        self.dismiss_popups();
        true
    }

    /// Activates the workspace `steps` away from the active one, if there is one.
    pub fn activate_workspace_by(&mut self, steps: isize) {
        let model = &self.shell.workspace_model;
        let index = model.active_index().checked_add_signed(steps);
        if let Some(id) = index.and_then(|i| model.id_at(i)).map(str::to_owned) {
            self.activate_workspace(&id);
        }
    }

    /// Sets the number of workspaces, from 1 to 9, numbered from 1. The windows of a removed
    /// workspace go to the active one; if that was the active one, the first one becomes active,
    /// without sliding.
    pub fn set_workspace_count(&mut self, count: usize) {
        self.queue_redraw();
        let before = self.shell.workspace_model.active().to_owned();
        self.shell.workspace_model.set_count(count);
        let windows: Vec<_> = self.space.elements().cloned().collect();
        let active = self.shell.workspace_model.active().to_owned();
        for window in &windows {
            if !self
                .shell
                .workspace_model
                .contains(&self.workspace_of(window))
            {
                *self.window_workspace(window).0.borrow_mut() = active.clone();
            }
        }
        if active != before {
            self.shell.slide.stop();
            self.focus(self.top_window());
        }
        // Windows of a removed workspace may now be under the pointer.
        self.refresh_pointer_focus();
    }

    /// Id of the workspace `window` belongs to.
    pub fn workspace_of(&self, window: &Window) -> String {
        self.window_workspace(window).0.borrow().clone()
    }

    /// Puts `window` on the workspace `id`, if it exists. Leaving the active workspace, it hides,
    /// and if it was the active window, the keyboard goes to the window that had it before.
    /// Nothing happens during a gesture, which would go on with a hidden window.
    pub fn move_to_workspace(&mut self, window: &Window, id: &str) {
        self.queue_redraw();
        if !self.shell.workspace_model.contains(id)
            || self.workspace_of(window) == id
            || self.gesture_active()
        {
            return;
        }
        let active = is_activated(window) || self.focused_window().as_ref() == Some(window);
        *self.window_workspace(window).0.borrow_mut() = id.to_owned();
        if active && !self.is_visible(window) {
            self.focus(self.top_window());
        }
        self.refresh_pointer_focus();
    }

    /// Puts a window that shows up on the active workspace. An X11 menu or tooltip goes on
    /// the workspace of the window it names as its parent (`WM_TRANSIENT_FOR`), if that one is
    /// known.
    pub(crate) fn join_active_workspace(&self, window: &Window) {
        let parent = window.x11_surface().and_then(|s| s.is_transient_for());
        let parent = parent.and_then(|id| {
            let mut windows = self.space.elements();
            windows.find(|w| w.x11_surface().is_some_and(|s| s.window_id() == id))
        });
        let workspace = match parent {
            Some(parent) if parent != window => self.workspace_of(parent),
            _ => self.shell.workspace_model.active().to_owned(),
        };
        *self.window_workspace(window).0.borrow_mut() = workspace;
    }

    /// Whether `window` is shown: on the active workspace, and not minimized.
    pub fn is_visible(&self, window: &Window) -> bool {
        !self.is_minimized(window) && self.workspace_of(window) == self.workspaces().active()
    }

    /// Horizontal shift at `now` of `window` for the slide between workspaces, `None` when it
    /// is not on the screen: minimized, or on a workspace out of view.
    pub fn workspace_shift(&self, window: &Window, now: std::time::Duration) -> Option<f64> {
        if self.is_minimized(window) {
            return None;
        }
        let model = &self.shell.workspace_model;
        let index = model.index(&self.workspace_of(window))?;
        let screen = self.window_output(window);
        let width = screen.and_then(|o| self.space.output_geometry(&o));
        let width = width.map_or(0.0, |g| f64::from(g.size.w));
        let active = model.active_index();
        self.shell.slide.shift(now, index, active, width)
    }

    /// The workspaces are sliding at `now`.
    pub fn workspaces_sliding(&self, now: std::time::Duration) -> bool {
        self.shell.slide.is_animating(now)
    }

    /// Where the row of workspaces is shown at `now`, in workspaces: the index of the active
    /// one at rest, in between during a slide or a swipe.
    pub fn workspace_view(&self, now: std::time::Duration) -> f64 {
        let active = self.shell.workspace_model.active_index();
        self.shell.slide.position(now, active)
    }

    /// A swipe holds the row of workspaces at `view` (lot D-31).
    pub(crate) fn hold_workspaces(&mut self, view: f64) {
        self.queue_redraw();
        self.shell.slide.hold(view);
    }

    /// The swipe lets go of the row with `velocity`, in workspaces per second, and lands on
    /// the workspace at `index`, which becomes active.
    pub(crate) fn release_workspaces(&mut self, index: usize, velocity: f64) {
        self.queue_redraw();
        let now = self.start_time.elapsed();
        let model = &self.shell.workspace_model;
        self.shell
            .slide
            .release(now, model.active_index(), velocity);
        if let Some(id) = model.id_at(index).map(str::to_owned) {
            self.activate_workspace(&id);
        }
    }

    /// The pointer goes to what is under it now that windows came or went.
    pub(crate) fn refresh_pointer_focus(&mut self) {
        if let Some(pointer) = self.seat.get_pointer() {
            let time = self.start_time.elapsed().as_millis() as u32;
            self.pointer_motion(pointer.current_location(), time);
        }
    }

    fn window_workspace<'a>(&self, window: &'a Window) -> &'a WindowWorkspace {
        let active = self.shell.workspace_model.active();
        window
            .user_data()
            .get_or_insert(|| WindowWorkspace(RefCell::new(active.to_owned())))
    }

    /// Whether `window` is minimized.
    pub fn is_minimized(&self, window: &Window) -> bool {
        self.shell.minimized.contains(window)
    }

    /// Hides `window` until it is activated. If it was the active one, the window on top
    /// becomes active.
    /// Nothing happens during a gesture, which would put the window back.
    pub fn minimize(&mut self, window: &Window) {
        self.queue_redraw();
        if self.space.element_geometry(window).is_none()
            || self.is_minimized(window)
            || is_override_redirect(window)
            || self.gesture_active()
        {
            return;
        }
        let active = is_activated(window) || self.focused_window().as_ref() == Some(window);
        self.shell.minimized.push(window.clone());
        self.start_minimize_effect(window, true);
        window.set_activated(false);
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_pending_configure();
        }
        if let Some(surface) = window.x11_surface()
            && let Err(err) = surface.set_suspended(true)
        {
            tracing::warn!("hiding the X11 window: {err}");
        }
        if active {
            self.focus(self.top_window());
        }
        self.refresh_pointer_focus();
    }

    /// Brings `window` back if minimized, goes to its workspace, raises it and gives it the
    /// keyboard.
    pub fn activate(&mut self, window: &Window) {
        self.queue_redraw();
        if !self.space.elements().any(|w| w == window) {
            return;
        }
        if self.is_minimized(window) {
            self.shell.minimized.retain(|w| w != window);
            self.start_minimize_effect(window, false);
            if let Some(surface) = window.x11_surface()
                && let Err(err) = surface.set_suspended(false)
            {
                tracing::warn!("showing the X11 window: {err}");
            }
        }
        // Straight to the window: no other one of its workspace gets the keyboard on the way.
        let workspace = self.workspace_of(window);
        self.switch_workspace(&workspace);
        self.focus(Some(window.clone()));
        self.refresh_pointer_focus();
    }

    /// Asks the application of `window` to close it.
    pub(crate) fn close(window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        } else if let Some(Err(err)) = window.x11_surface().map(|s| s.close()) {
            tracing::warn!("closing the X11 window: {err}");
        }
    }

    /// The window of id `id`, shown or hidden.
    fn window_by_id(&self, id: u32) -> Option<Window> {
        self.space.elements().find(|w| window_id(w) == id).cloned()
    }

    /// Tells the clients of the shell what changed since the last call. Called after each loop
    /// iteration, before the replies are flushed.
    pub(crate) fn refresh_shell(&mut self) {
        // A window destroyed, or withdrawn by its X11 client, while minimized is forgotten. A
        // withdrawn X11 window loses `_NET_WM_STATE_HIDDEN`: smithay keeps it for the life of the
        // surface, and the window mapped again would believe it is still minimized.
        self.shell.minimized.retain(|window| {
            let kept = window.alive() && window.x11_surface().is_none_or(|s| s.is_mapped());
            if let (false, Some(surface)) = (kept, window.x11_surface())
                && surface.alive()
                && let Err(err) = surface.set_suspended(false)
            {
                tracing::warn!("showing the withdrawn X11 window: {err}");
            }
            kept
        });
        self.shell.managers.retain(|m| m.alive());
        self.shell.workspaces.retain(|m| m.alive());
        let icons = std::mem::take(&mut self.shell.icons);
        self.shell.icons = icons
            .into_iter()
            .filter(|i| self.window_by_id(i.0).is_some())
            .collect();
        if !self.shell.managers.is_empty() {
            let (windows, outputs) = self.snapshot();
            let dh = self.display_handle.clone();
            for manager in &mut self.shell.managers {
                manager.update(&dh, &windows, &outputs);
            }
        }
        if !self.shell.workspaces.is_empty() {
            let outputs: Vec<_> = self.space.outputs().cloned().collect();
            let dh = self.display_handle.clone();
            for manager in &mut self.shell.workspaces {
                manager.update(&dh, &outputs, &self.shell.workspace_model);
            }
        }
    }

    /// Name and origin of a screen. Without any screen, an empty name and the origin of the
    /// space: the window stays listed rather than being announced closed.
    fn screen(&self, output: Option<smithay::output::Output>) -> (String, Point<i32, Logical>) {
        let origin = output.as_ref().and_then(|o| self.space.output_geometry(o));
        let name = output.map(|o| o.name()).unwrap_or_default();
        (name, origin.map(|g| g.loc).unwrap_or_default())
    }

    /// Every window the shell sees, and the state of every screen.
    fn snapshot(&self) -> (toplevels::Windows, toplevels::Outputs) {
        use toplevels::{OutputState, ToplevelState};
        let windows: Vec<_> = self
            .space
            .elements()
            .filter(|w| !is_override_redirect(w) && window_data(w).shown.get())
            .filter_map(|w| {
                let (name, screen) = self.screen(self.window_output(w));
                let mut geometry = self.space.element_geometry(w)?;
                geometry.loc -= screen;
                Some((w, name, geometry, self.is_minimized(w)))
            })
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
                    workspace: self.workspace_of(window),
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
                    (OutputState::Snapped, ToplevelState::Snapped),
                ] {
                    let active = self.shell.workspace_model.active();
                    let on = windows.iter().any(|(_, w)| {
                        w.output == name
                            && w.workspace == active
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
