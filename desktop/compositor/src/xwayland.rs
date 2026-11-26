//! XWayland: the X11 server that runs X11 applications in the compositor, and Smithay's X11
//! window manager (`X11Wm`) that turns them into windows like the others.
//!
//! XWayland is a Wayland client of the compositor. Each X11 window has a Wayland surface there,
//! which the xwayland-shell protocol associates with the X11 window. Managed windows go through
//! `Window` and have the same gestures, the same rendering and the same states as xdg-shell
//! windows. Override-redirect windows (menus, tooltips) place themselves.

use std::{
    fs::{self, DirBuilder, Permissions},
    io,
    os::{
        fd::OwnedFd,
        unix::fs::{DirBuilderExt, PermissionsExt},
    },
    path::Path,
    process::Stdio,
};

use smithay::{
    delegate_xwayland_shell,
    desktop::{Window, space::RenderZindex},
    reexports::{
        calloop::EventLoop,
        wayland_server::{backend::DisconnectReason, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Rectangle},
    wayland::{
        selection::{
            SelectionTarget,
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWayland, XWaylandEvent, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use crate::{
    Error, State,
    focus::KeyboardFocus,
    handlers::{Arrangement, configure_x11},
    layout::{self, Edges},
    mesa, window_data,
};

impl State {
    /// Launches XWayland. Once it is ready, its window manager starts, `x_display` gives its
    /// display number, then `ready` is called with that number (`None` if it failed).
    pub fn start_xwayland(
        &mut self,
        event_loop: &EventLoop<'static, State>,
        ready: impl FnOnce(&mut State, Option<u32>) + 'static,
    ) -> Result<(), Error> {
        ensure_x11_unix_dir().map_err(Error::Xwayland)?;
        // Smithay starts Xwayland with an empty environment: it gets what keeps it off the
        // NVIDIA card (`mesa`).
        let var = |key: &str| std::env::var_os(key);
        let env = mesa::xwayland_env(var, mesa::backends_without_nvidia(var));
        tracing::debug!(?env, "Xwayland environment");
        let (xwayland, client) = XWayland::spawn(
            &self.display_handle,
            None,
            env,
            true,
            Stdio::null(),
            // Its errors stay visible, on our standard error.
            Stdio::inherit(),
            |_| (),
        )
        .map_err(Error::Xwayland)?;
        let mut ready = Some(ready);
        let token = event_loop
            .handle()
            .insert_source(xwayland, move |event, _, state| {
                let display = match event {
                    XWaylandEvent::Ready {
                        x11_socket,
                        display_number,
                    } => {
                        match X11Wm::start_wm(state.loop_handle.clone(), x11_socket, client.clone())
                        {
                            Ok(xwm) => {
                                tracing::info!(display = display_number, "XWayland ready");
                                state.xwm = Some(xwm);
                                state.x_display = Some(display_number);
                                Some(display_number)
                            }
                            Err(err) => {
                                tracing::warn!("X11 window manager: {err}");
                                // `start_wm` may fail after tagging the XWayland client with
                                // its XwmId: smithay's xwayland-shell hook would then ask for
                                // a window manager that does not exist. Disconnect the client
                                // right away, so that none of its requests is dispatched, and
                                // stop XWayland once this callback has returned.
                                state
                                    .display_handle
                                    .backend_handle()
                                    .kill_client(client.id(), DisconnectReason::ConnectionClosed);
                                state.loop_handle.insert_idle(State::stop_xwayland);
                                None
                            }
                        }
                    }
                    XWaylandEvent::Error => {
                        tracing::warn!("XWayland stopped during its startup");
                        state.loop_handle.insert_idle(State::stop_xwayland);
                        None
                    }
                };
                if let Some(ready) = ready.take() {
                    ready(state, display);
                }
            })
            .map_err(|err| err.error)?;
        self.xwayland = Some(token);
        Ok(())
    }

    /// Stops XWayland: it exits, and its lock and its socket (`/tmp/.X11-unix/X<N>`) go away.
    /// To call when quitting: the X11Wm source keeps a handle to the loop, which is therefore
    /// never freed, nor XWayland with it.
    pub fn stop_xwayland(&mut self) {
        if let Some(token) = self.xwayland.take() {
            self.loop_handle.remove(token);
        }
    }
}

/// Creates `/tmp/.X11-unix`, where the X server sockets go, if it does not exist: on a machine
/// without an X server, like the CI one, nobody has created it. It belongs to everyone, with the
/// sticky bit, as systemd creates it.
fn ensure_x11_unix_dir() -> io::Result<()> {
    let dir = Path::new("/tmp/.X11-unix");
    if dir.is_dir() {
        return Ok(());
    }
    DirBuilder::new().mode(0o1777).create(dir)?;
    // The process umask may have removed permissions at creation.
    fs::set_permissions(dir, Permissions::from_mode(0o1777))
}

impl State {
    /// A window of the X server behind `xwm` has the keyboard.
    fn x11_has_keyboard(&self, xwm: XwmId) -> bool {
        let focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        matches!(focus, Some(KeyboardFocus::X11(surface)) if surface.xwm_id() == Some(xwm))
    }
}

/// Edges pulled by a `_NET_WM_MOVERESIZE` request.
fn x11_edges(edge: ResizeEdge) -> Edges {
    let (top, bottom, left, right) = match edge {
        ResizeEdge::Top => (true, false, false, false),
        ResizeEdge::Bottom => (false, true, false, false),
        ResizeEdge::Left => (false, false, true, false),
        ResizeEdge::Right => (false, false, false, true),
        ResizeEdge::TopLeft => (true, false, true, false),
        ResizeEdge::TopRight => (true, false, false, true),
        ResizeEdge::BottomLeft => (false, true, true, false),
        ResizeEdge::BottomRight => (false, true, false, true),
    };
    Edges {
        top,
        bottom,
        left,
        right,
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        match self.xwm.as_mut() {
            Some(wm) => wm,
            // Smithay calls this from two places only: the X11Wm's own event source, which
            // exists only once `start_wm` has succeeded (and the X11Wm is stored right after),
            // and the xwayland-shell commit hook, which only runs for requests of the XWayland
            // client. When `start_wm` fails, that client is disconnected before any of its
            // requests can be dispatched (see `start_xwayland`), so this branch cannot be
            // reached. The trait leaves no way out without a window manager to lend: there is
            // no X11Wm to return, so the only option left is to stop.
            None => {
                tracing::error!(?xwm, "X11 window manager requested but missing");
                unreachable!("X11 window manager requested before it was started")
            }
        }
    }

    fn new_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    /// A managed window asks to be mapped: center it, at the size it asks for. It is shown and
    /// gets the keyboard when XWayland associates its surface with it.
    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Err(err) = surface.set_mapped(true) {
            tracing::warn!("mapping the X11 window: {err}");
            return;
        }
        let size = surface.geometry().size;
        let loc = self
            .output_area()
            .map(|area| layout::centered(area, size))
            .unwrap_or_default();
        tracing::debug!(title = surface.title(), ?loc, ?size, "X11 window");
        self.space
            .map_element(Window::new_x11_window(surface.clone()), loc, false);
        configure_x11(&surface, Rectangle::new(loc, size));
    }

    /// A menu or a tooltip shows up at the place it chose, above the windows.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let loc = surface.geometry().loc;
        let window = Window::new_x11_window(surface);
        window.override_z_index(RenderZindex::Overlay as u8);
        self.space.map_element(window.clone(), loc, false);
        self.join_active_workspace(&window);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            let focused = self.focused_window().as_ref() == Some(&window);
            self.space.unmap_elem(&window);
            if focused {
                self.focus(self.top_window());
            }
        }
        if !surface.is_override_redirect()
            && let Err(err) = surface.set_mapped(false)
        {
            tracing::warn!("unmapping the X11 window: {err}");
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    /// A floating window takes the size it asks for, but not the place: the compositor places
    /// it. Maximized, snapped or fullscreen, it keeps its own size.
    fn configure_request(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let mut rect = surface.geometry();
        let arranged = self.x11_window(&surface).is_some_and(|window| {
            let a = Arrangement::of(&window);
            a.maximized || a.fullscreen || window_data(&window).zone.get().is_some()
        });
        if !arranged {
            rect.size.w = w.map_or(rect.size.w, |w| w as i32);
            rect.size.h = h.map_or(rect.size.h, |h| h as i32);
        }
        // Even when refused, the request gets an answer: the current geometry.
        if let Err(err) = surface.configure(rect) {
            tracing::warn!("X11 window configure: {err}");
        }
    }

    /// A menu moves by itself; a managed window stays at the place it was given.
    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        if surface.is_override_redirect()
            && let Some(window) = self.x11_window(&surface)
        {
            self.space.map_element(window, geometry.loc, false);
        }
    }

    fn maximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.rearrange(&window, |a, _| a.maximized = true);
        }
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.rearrange(&window, |a, zone| {
                a.maximized = false;
                zone.set(None);
            });
        }
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.rearrange(&window, |a, _| a.fullscreen = true);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Some(window) = self.x11_window(&surface) {
            self.rearrange(&window, |a, _| a.fullscreen = false);
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, surface: X11Surface, _button: u32, edge: ResizeEdge) {
        self.x11_gesture(&surface, Some(x11_edges(edge)));
    }

    fn move_request(&mut self, _xwm: XwmId, surface: X11Surface, _button: u32) {
        self.x11_gesture(&surface, None);
    }

    /// An X11 client only reads the clipboard if an X11 window has the keyboard, like a Wayland
    /// client.
    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        self.x11_has_keyboard(xwm)
    }

    /// An X11 client reads the selection of a Wayland client.
    fn send_selection(
        &mut self,
        _xwm: XwmId,
        selection: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
    ) {
        let sent = match selection {
            SelectionTarget::Clipboard => {
                request_data_device_client_selection(&self.seat, mime_type, fd)
                    .map_err(|e| e.to_string())
            }
            SelectionTarget::Primary => request_primary_client_selection(&self.seat, mime_type, fd)
                .map_err(|e| e.to_string()),
        };
        if let Err(err) = sent {
            tracing::warn!(?selection, "Wayland selection read by XWayland: {err}");
        }
    }

    /// An X11 client set a selection: the compositor sets it on the Wayland side on its
    /// behalf, but only while one of this X server's windows has the keyboard. That is the
    /// rule smithay applies to Wayland clients, and the one used for reading above: a
    /// background X11 program cannot replace what the user pastes elsewhere. No exception
    /// for command line tools such as `xclip` run from a Wayland terminal.
    fn new_selection(&mut self, xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        if !self.x11_has_keyboard(xwm) {
            tracing::debug!(
                ?selection,
                "X11 selection refused: no X11 window has the keyboard"
            );
            return;
        }
        let dh = &self.display_handle;
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(dh, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(dh, &self.seat, mime_types, ()),
        }
    }

    /// The X11 client that held the selection has released it. Only the one set on its behalf
    /// is cleared, not one from a Wayland client that came since.
    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        let dh = &self.display_handle;
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(dh, &self.seat);
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(dh, &self.seat);
                }
            }
        }
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    /// The X11 window has its surface: it shows up and gets the keyboard, like a Wayland window
    /// at its first content. Before that, XWayland would not have received the keyboard input.
    fn surface_associated(&mut self, _xwm: XwmId, _surface: WlSurface, surface: X11Surface) {
        let Some(window) = self.x11_window(&surface) else {
            return;
        };
        let data = window_data(&window);
        if !surface.is_override_redirect() && !data.shown.get() {
            data.shown.set(true);
            self.join_active_workspace(&window);
            self.focus(Some(window));
        }
    }
}

delegate_xwayland_shell!(State);
