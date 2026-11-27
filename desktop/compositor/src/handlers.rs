//! Protocol handlers: wl_compositor, wl_shm, wl_seat, wl_output, xdg-shell, and the clipboard
//! (wl_data_device and primary selection). Layer-shell is in `layer.rs`, the
//! screens and their scale in `output.rs`.

use std::{cell::Cell, os::fd::OwnedFd, sync::OnceLock};

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_cursor_shape, delegate_data_device, delegate_output,
    delegate_primary_selection, delegate_seat, delegate_shm,
    desktop::{PopupKind, PopupManager, Window, find_popup_root_surface},
    input::{
        Seat, SeatHandler, SeatState,
        pointer::{CursorImageStatus, GrabStartData},
    },
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        wayland_server::{
            Client, Resource,
            backend::ClientId,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{self, CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{
            SelectionHandler, SelectionTarget,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
            primary_selection::{
                PrimarySelectionHandler, PrimarySelectionState, set_primary_focus,
            },
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
        tablet_manager::TabletSeatHandler,
    },
    xwayland::{X11Surface, XWaylandClientData},
};

use crate::{
    ClientState, State, decorations,
    focus::KeyboardFocus,
    input::GestureSource,
    layer::PopupGrab,
    layout::{Edges, Zone},
    window_data,
};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Our clients go through `insert_client`, except XWayland, which Smithay plugs in
        // itself. The empty state only avoids panicking.
        static EMPTY: OnceLock<CompositorClientState> = OnceLock::new();
        if let Some(data) = client.get_data::<XWaylandClientData>() {
            return &data.compositor_state;
        }
        match client.get_data::<ClientState>() {
            Some(data) => &data.compositor_state,
            None => EMPTY.get_or_init(CompositorClientState::default),
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.lock_surface_committed(surface);
        if !compositor::is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = compositor::get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for(&root) {
                window.on_commit();
                // An X11 window shown before its first buffer takes the keyboard with it.
                let data = window_data(&window);
                if data.focus_on_content.get() && crate::has_buffer(&window) {
                    data.focus_on_content.set(false);
                    self.focus(Some(window.clone()));
                }
                if &root == surface {
                    self.toplevel_commit(window);
                }
            } else if &root == surface {
                self.layer_commit(surface);
            }
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
            && let Err(err) = popup.send_configure()
        {
            tracing::warn!("popup configure: {err}");
        }
    }
}

/// The largest content `window` may have in the work area `area`: the area, less its title
/// bar.
pub(crate) fn room(window: &Window, area: Rectangle<i32, Logical>) -> Size<i32, Logical> {
    let inset = crate::decorations::inset(window);
    Size::from((area.size.w, (area.size.h - inset).max(1)))
}

impl State {
    fn toplevel_commit(&mut self, window: Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        if !toplevel.is_initial_configure_sent() {
            // The room the window has (`configure_bounds`): the work area, under its title
            // bar. A client that knows it does not open larger than the screen.
            if let Some(area) = self.output_area() {
                let room = room(&window, area);
                toplevel.with_pending_state(|s| s.bounds = Some(room));
            }
            toplevel.send_configure();
            return;
        }
        let data = window_data(&window);
        let size = window.geometry().size;
        let filled = toplevel.with_pending_state(|s| {
            s.states.contains(XdgState::Maximized) || s.states.contains(XdgState::Fullscreen)
        });

        // First content: center the window (unless it takes the whole screen) and give it the
        // keyboard.
        if !data.shown.get() && size.w > 0 && size.h > 0 {
            // Larger than the work area (a size kept from a larger screen): it is asked to fit
            // and shows at its next content, its edges and corners in reach. Once only: a
            // client that keeps its size shows at it.
            if let (false, Some(area)) = (filled, self.output_area()) {
                let room = room(&window, area);
                let fit = Size::from((size.w.min(room.w), size.h.min(room.h)));
                if fit != size && !data.fitting.replace(true) {
                    tracing::debug!(?size, ?fit, "first content larger than the work area");
                    self.request_size(&window, Some(fit));
                    toplevel.send_pending_configure();
                    return;
                }
            }
            data.shown.set(true);
            self.join_active_workspace(&window);
            if let (false, Some(area)) = (filled, self.output_area()) {
                let loc = decorations::centered(&window, area, size);
                self.move_window(&window, loc, false);
            }
            self.focus(Some(window));
            return;
        }

        // Resize in progress: the edge opposite the pulled one stays in place.
        if let Some(resize) = data.resize.get() {
            let (loc, next) = resize.on_commit(size);
            data.resize.set(next);
            self.move_window(&window, loc, false);
        }
    }

    /// Places the window at `loc`; with `activate`, raises it and makes it the active one. An
    /// X11 window learns its place: XWayland needs it to place its menus and translate the
    /// pointer.
    pub(crate) fn move_window(
        &mut self,
        window: &Window,
        loc: Point<i32, Logical>,
        activate: bool,
    ) {
        // The space raises every window it maps: those that were above go back above it.
        let above: Vec<Window> = match activate {
            true => Vec::new(),
            false => self
                .space
                .elements()
                .skip_while(|w| *w != window)
                .skip(1)
                .cloned()
                .collect(),
        };
        self.space.map_element(window.clone(), loc, activate);
        for other in &above {
            self.space.raise_element(other, false);
        }
        if let Some(surface) = window.x11_surface() {
            configure_x11(surface, Rectangle::new(loc, surface.geometry().size));
        }
    }

    /// Asks the client for the size `size`. `None` lets it choose: an X11 window then keeps its
    /// own. A Wayland window learns it with its next configure, an X11 window right away.
    pub(crate) fn request_size(&self, window: &Window, size: Option<Size<i32, Logical>>) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| s.size = size);
        } else if let (Some(surface), Some(size)) = (window.x11_surface(), size) {
            let loc = self.space.element_location(window).unwrap_or_default();
            configure_x11(surface, Rectangle::new(loc, size));
        }
    }

    /// Gesture requested by an xdg-shell window, which draws its title bar and its edges itself.
    /// As xdg-shell requires, it is only granted on our seat, and with the serial of the press
    /// in progress.
    fn xdg_gesture(
        &mut self,
        surface: &ToplevelSurface,
        seat: &wl_seat::WlSeat,
        serial: Serial,
        edges: Option<Edges>,
    ) {
        let start = Seat::<State>::from_resource(seat)
            .filter(|seat| *seat == self.seat)
            .and_then(|seat| seat.get_pointer())
            .filter(|pointer| pointer.has_grab(serial))
            .and_then(|pointer| pointer.grab_start_data());
        tracing::debug!(?serial, "gesture requested by a Wayland client");
        self.client_gesture(self.window_for(surface.wl_surface()), start, edges);
    }

    /// Gesture requested by an X11 window (`_NET_WM_MOVERESIZE`). The X server gives no serial:
    /// a button must be held on the surface of this window. All X11 clients
    /// share one Wayland client (XWayland), so "same client" would let an X11 window move
    /// itself during a click in another one.
    pub(crate) fn x11_gesture(&mut self, surface: &X11Surface, edges: Option<Edges>) {
        let own = surface.wl_surface();
        let start = self
            .seat
            .get_pointer()
            .filter(|pointer| pointer.is_grabbed())
            .and_then(|pointer| pointer.grab_start_data())
            .filter(|start| {
                own.is_some() && start.focus.as_ref().map(|(focus, _)| focus) == own.as_ref()
            });
        tracing::debug!("gesture requested by an X11 client");
        self.client_gesture(self.x11_window(surface), start, edges);
    }

    /// Gesture requested by the client. It is only granted while a button is pressed on a
    /// surface of this client (`start`), and never in fullscreen: the client keeps its press.
    fn client_gesture(
        &mut self,
        window: Option<Window>,
        start: Option<GrabStartData<State>>,
        edges: Option<Edges>,
    ) {
        let surface = window.as_ref().and_then(|w| w.wl_surface().map(|s| s.id()));
        let same_client = start.as_ref().is_some_and(|start| {
            start.focus.as_ref().is_some_and(|(focus, _)| {
                surface
                    .as_ref()
                    .is_some_and(|id| focus.id().same_client_as(id))
            })
        });
        let window = window.filter(|w| !is_fullscreen(w));
        let (Some(pointer), Some(start), Some(window), true, false) = (
            self.seat.get_pointer(),
            start,
            window,
            same_client,
            self.gesture_active(),
        ) else {
            tracing::debug!(?edges, "client gesture request refused");
            return;
        };
        // Take the pointer back from the client: it receives nothing more until the release.
        let time = self.start_time.elapsed().as_millis() as u32;
        pointer.unset_grab(self, SERIAL_COUNTER.next_serial(), time);
        self.start_gesture(
            window,
            start.location,
            start.button,
            edges,
            GestureSource::Client,
        );
        self.pointer_motion(pointer.current_location(), time);
    }

    /// Changes the state of a window (maximized, fullscreen, snapped into a zone) then places
    /// it: over its whole screen, in its zone, or at its previous place. That place is kept when
    /// the window leaves the floating state, and given back when it returns to it.
    pub(crate) fn rearrange(
        &mut self,
        window: &Window,
        change: impl FnOnce(&mut Arrangement, &Cell<Option<Zone>>),
    ) {
        self.rearrange_on(window, None, change);
    }

    /// Same as `rearrange`, on `output` rather than the window's screen. A fullscreen window
    /// covers the whole screen; a maximized or snapped one stays in its work area, out of the
    /// exclusive zones of anchored surfaces.
    pub(crate) fn rearrange_on(
        &mut self,
        window: &Window,
        output: Option<Output>,
        change: impl FnOnce(&mut Arrangement, &Cell<Option<Zone>>),
    ) {
        let Some(output) = output.or_else(|| self.window_output(window)) else {
            return;
        };
        let (Some(screen), Some(work)) =
            (self.space.output_geometry(&output), self.work_area(&output))
        else {
            return;
        };
        let data = window_data(window);
        let mut arrangement = Arrangement::of(window);
        change(&mut arrangement, &data.zone);
        tracing::debug!(
            fullscreen = arrangement.fullscreen,
            maximized = arrangement.maximized,
            x11 = window.x11_surface().is_some(),
            "window rearranged"
        );
        arrangement.apply(window);
        let target = match (
            arrangement.fullscreen,
            arrangement.maximized,
            data.zone.get(),
        ) {
            (true, ..) => Some(screen),
            (false, true, _) => Some(work),
            (false, false, zone) => zone.map(|zone| zone.geometry(work, self.snap_gap)),
        };
        // The zone takes the whole window: the content goes under its title bar, if the
        // compositor draws one.
        let target = target.map(|rect| decorations::content(window, rect));
        // Snap, maximize, fullscreen or restore: the window slides to its place, at once, even
        // when only its size changes (an exclusive zone that goes away under a maximized
        // window). Restored to a size its client chooses, it slides once that size is known.
        let restore = data.restore.get();
        let known = restore.filter(|r| r.size.w > 0 && r.size.h > 0);
        match target.or(known) {
            Some(rect) => data.motion.borrow_mut().animate_to(rect),
            None if restore.is_some() => data.motion.borrow_mut().animate_next_change(),
            None => {}
        }
        if let Some(rect) = target {
            if data.restore.get().is_none() {
                data.restore.set(self.space.element_geometry(window));
            }
            self.request_size(window, Some(rect.size));
            self.move_window(window, rect.loc, false);
        } else if let Some(restore) = data.restore.take() {
            // Zero size: the window had no content yet, the client chooses.
            let size = (restore.size.w > 0 && restore.size.h > 0).then_some(restore.size);
            self.request_size(window, size);
            self.move_window(window, restore.loc, false);
        }
        if let Some(toplevel) = window.toplevel()
            && toplevel.is_initial_configure_sent()
        {
            toplevel.send_pending_configure();
        }
    }

    /// Snaps the window into `zone` of `output` (its own screen otherwise), or gives it back its
    /// previous place. No effect in fullscreen.
    pub(crate) fn snap(&mut self, window: &Window, zone: Option<Zone>, output: Option<Output>) {
        if is_fullscreen(window) {
            return;
        }
        self.rearrange_on(window, output, |a, current| {
            a.maximized = zone == Some(Zone::Maximized);
            current.set(zone.filter(|z| *z != Zone::Maximized));
        });
    }

    /// Closes the menu chain that waits for an outside click.
    pub(crate) fn dismiss_popups(&mut self) {
        if let Some(grab) = self.popup_grab.take() {
            // The shell's menus close with their last image (lot UI-06).
            self.ghost_popups_of(&grab.root);
            PopupManager::dismiss_popup(&grab.root, &grab.popup).ok();
        }
    }

    fn surface_window(&self, surface: &ToplevelSurface) -> Option<Window> {
        self.window_for(surface.wl_surface())
    }
}

/// States a window knows about: those of xdg-shell, or `_NET_WM_STATE` for an X11 window.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Arrangement {
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Arrangement {
    /// States of the window, including those it has not received yet.
    pub fn of(window: &Window) -> Self {
        match (window.toplevel(), window.x11_surface()) {
            (Some(toplevel), _) => toplevel.with_pending_state(|s| Self {
                maximized: s.states.contains(XdgState::Maximized),
                fullscreen: s.states.contains(XdgState::Fullscreen),
            }),
            (None, Some(surface)) => Self {
                maximized: surface.is_maximized(),
                fullscreen: surface.is_fullscreen(),
            },
            (None, None) => Self {
                maximized: false,
                fullscreen: false,
            },
        }
    }

    /// Gives these states to the window. A Wayland window receives them with its next
    /// configure, an X11 window right away.
    pub fn apply(self, window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| {
                for (state, on) in [
                    (XdgState::Maximized, self.maximized),
                    (XdgState::Fullscreen, self.fullscreen),
                ] {
                    if on {
                        s.states.set(state);
                    } else {
                        s.states.unset(state);
                    }
                }
            });
        } else if let Some(surface) = window.x11_surface() {
            let done = surface
                .set_maximized(self.maximized)
                .and_then(|()| surface.set_fullscreen(self.fullscreen));
            if let Err(err) = done {
                tracing::warn!("X11 window states: {err}");
            }
        }
    }
}

/// Place and size of an X11 window, in space coordinates: XWayland reads the place of each
/// screen from xdg-output.
pub(crate) fn configure_x11(surface: &X11Surface, rect: Rectangle<i32, Logical>) {
    if surface.is_override_redirect() || surface.geometry() == rect {
        return;
    }
    if let Err(err) = surface.configure(rect) {
        tracing::warn!("X11 window configure: {err}");
    }
}

/// The window is fullscreen, or about to be.
pub(crate) fn is_fullscreen(window: &Window) -> bool {
    Arrangement::of(window).fullscreen
}

/// The window is maximized, or about to be.
pub(crate) fn is_maximized(window: &Window) -> bool {
    Arrangement::of(window).maximized
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_client(&mut self, client: smithay::wayland::shell::xdg::ShellClient) {
        self.shell_client_bound(client);
    }

    fn client_pong(&mut self, client: smithay::wayland::shell::xdg::ShellClient) {
        self.shell_client_pong(&client);
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Placed for good at its first content, when its size is known.
        let loc = self.output_area().map(|a| a.loc).unwrap_or_default();
        self.space
            .map_element(Window::new_wayland_window(surface), loc, false);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        // The keyboard goes back to a window only if this one had it: a window closing on a
        // hidden workspace, or while an anchored surface has the keyboard, takes nothing.
        if let Some(window) = self.window_for(surface.wl_surface()) {
            let focused = self.focused_window().as_ref() == Some(&window);
            self.start_closing(&window);
            self.space.unmap_elem(&window);
            if focused {
                self.focus(self.top_window());
            }
        }
    }

    /// A menu goes: one of the shell's closes with its last image (lot UI-06).
    fn popup_destroyed(&mut self, surface: PopupSurface) {
        self.ghost_popup(&PopupKind::Xdg(surface));
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!("popup refused: {err}");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    /// A menu asks to close on an outside click (the menus of the bar). A click on a surface of
    /// another client closes the whole chain; the client handles clicks on its own surfaces.
    /// The serial is not checked: iced opens its menus on release, after the press it names.
    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, _serial: Serial) {
        let popup = PopupKind::Xdg(surface);
        let Ok(root) = find_popup_root_surface(&popup) else {
            return;
        };
        if Seat::<State>::from_resource(&seat).as_ref() != Some(&self.seat) {
            PopupManager::dismiss_popup(&root, &popup).ok();
            return;
        }
        // A menu closed in the same batch as this one opens (a panel of the bar replaced by
        // another) no longer holds the grab: the new one takes it.
        if self.popup_grab.as_ref().is_some_and(|g| !g.popup.alive()) {
            self.popup_grab = None;
        }
        // A submenu keeps the first menu of the chain: closing that one closes them all.
        if self
            .popup_grab
            .as_ref()
            .is_none_or(|grab| grab.root != root)
        {
            self.dismiss_popups();
            self.popup_grab = Some(PopupGrab { root, popup });
        }
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.xdg_gesture(&surface, &seat, serial, None);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: ResizeEdge,
    ) {
        let edges = Edges::from_xdg_bits(edges as u32);
        if edges != Edges::default() {
            self.xdg_gesture(&surface, &seat, serial, Some(edges));
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface) {
            self.rearrange(&window, |a, _| a.maximized = true);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface) {
            self.client_unmaximize(&window);
        }
    }

    /// The application asks to be minimized (its minimize button): like the shell does it. A
    /// window without content yet is not, nor a window being dragged.
    fn minimize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface)
            && window_data(&window).shown.get()
        {
            self.minimize(&window);
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        if let Some(window) = self.surface_window(&surface) {
            let screen = output.as_ref().and_then(Output::from_resource);
            surface.with_pending_state(|s| s.fullscreen_output = output);
            self.rearrange_on(&window, screen, |a, _| a.fullscreen = true);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface) {
            surface.with_pending_state(|s| s.fullscreen_output = None);
            self.rearrange(&window, |a, _| a.fullscreen = false);
        }
    }
}

impl SeatHandler for State {
    type KeyboardFocus = KeyboardFocus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    /// Only the client that has the keyboard reads and sets the clipboard and the primary
    /// selection. While the session is locked, no client is offered either: not even the lock
    /// program, which has the keyboard (lot D-29).
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&KeyboardFocus>) {
        let dh = &self.display_handle;
        let client = focused
            .and_then(WaylandFocus::wl_surface)
            .and_then(|surface| dh.get_client(surface.id()).ok())
            .filter(|_| !self.is_locked());
        tracing::debug!(?focused, "keyboard");
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        // Each shape a client names, accepted by smithay (lot UI-07): `RUST_LOG=compositor=trace`.
        tracing::trace!(?image, "cursor image");
        self.cursor_status = image;
    }
}

// Required by the cursor shape protocol, which also accepts styluses. With no tablet
// advertised, no client has one.
impl TabletSeatHandler for State {}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl OutputHandler for State {}

/// Where a new selection comes from.
#[derive(Debug, Clone)]
pub enum SelectionOrigin {
    /// A Wayland client: the one that has the keyboard, the only one smithay lets set it.
    Wayland(Option<ClientId>),
    /// An X11 application, through XWayland.
    X11,
    /// The compositor itself: a screenshot.
    Compositor,
}

/// The type a password manager (KeePassXC, KDE's) adds to a secret it copies: a clipboard
/// history must not keep it. Wayland clients only: smithay passes on from X11 the types with
/// a `/` alone.
pub const SECRET_HINT: &str = "x-kde-passwordManagerHint";

/// The selection offers the secret hint.
pub fn is_secret(mime_types: &[String]) -> bool {
    mime_types.iter().any(|m| m == SECRET_HINT)
}

impl State {
    /// Every new selection passes here, CLIPBOARD and PRIMARY, from Wayland, X11 or the
    /// compositor, with its types (none: a Wayland client cleared it). The clipboard history (lot D-42) plugs in
    /// here. Called before the selection is in place on the Wayland side.
    pub(crate) fn selection_set(
        &mut self,
        target: SelectionTarget,
        origin: SelectionOrigin,
        mime_types: &[String],
    ) {
        let secret = is_secret(mime_types);
        tracing::debug!(?target, ?origin, ?mime_types, secret, "new selection");
    }

    /// No client is offered the selections any more.
    pub(crate) fn unset_selection_focus(&mut self) {
        set_data_device_focus(&self.display_handle, &self.seat, None);
        set_primary_focus(&self.display_handle, &self.seat, None);
    }

    /// Tells XWayland what the selection `target` now offers (`None`: nothing). The X11
    /// clients that read it then ask the compositor (`XwmHandler::send_selection`).
    pub(crate) fn offer_to_xwayland(
        &mut self,
        target: SelectionTarget,
        mimes: Option<Vec<String>>,
    ) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.new_selection(target, mimes)
        {
            tracing::warn!(?target, "selection passed to XWayland: {err}");
        }
    }
}

/// The clipboard of a Wayland client is relayed to XWayland, and the other way round: when an
/// X11 client sets it, the compositor sets it on the Wayland side on its behalf (see
/// `xwayland.rs`).
/// The compositor sets a selection of its own too: a screenshot, as `image/png` (lot D-35c).
/// Its user data holds the bytes; `None` is a selection set for XWayland.
///
/// While the session is locked, the selections may still change (the lock program has the
/// keyboard, a screenshot taken before may land), and XWayland still learns of it, but nothing
/// is read: no Wayland client is offered one (`focus_changed`), no X11 client may read
/// (`allow_selection_access`), and an offer received before the lock is refused here.
impl SelectionHandler for State {
    type SelectionUserData = Option<std::sync::Arc<[u8]>>;

    fn new_selection(
        &mut self,
        target: SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        seat: Seat<Self>,
    ) {
        let mime_types = source.map(|s| s.mime_types());
        let client = seat
            .get_keyboard()
            .and_then(|k| k.current_focus())
            .and_then(|f| f.wl_surface().and_then(|s| s.client()))
            .map(|c| c.id());
        let origin = SelectionOrigin::Wayland(client);
        self.selection_set(target, origin, mime_types.as_deref().unwrap_or_default());
        self.offer_to_xwayland(target, mime_types);
    }

    fn send_selection(
        &mut self,
        target: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        user_data: &Self::SelectionUserData,
    ) {
        if self.is_locked() {
            // Dropping `fd` ends the transfer: the client reads nothing.
            tracing::debug!(?target, "selection not read: the session is locked");
            return;
        }
        if let Some(bytes) = user_data {
            self.paste_png(fd, bytes.clone());
            return;
        }
        // Set for XWayland: smithay reads the text, the compositor the other types.
        if let Some(display) = self.x_display
            && !crate::x11_paste::read_by_smithay(&mime_type)
        {
            crate::x11_paste::paste(display, target, mime_type, fd);
            return;
        }
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.send_selection(target, mime_type, fd, self.loop_handle.clone())
        {
            tracing::warn!(?target, "selection read from XWayland: {err}");
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

// Drag and drop between Wayland clients follows Smithay's default behavior. Between Wayland
// and X11, smithay 0.7 has no XDND: it is left to lot D-29c (see `docs/compositor.md`).
impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

impl PrimarySelectionHandler for State {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_output!(State);
// `delegate_xdg_shell!`, but the binding of `xdg_wm_base`, which `lock.rs` reads first to know
// the shell client of the lock program (lot D-25f).
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_wm_base::XdgWmBase:
        smithay::wayland::shell::xdg::XdgWmBaseUserData
] => XdgShellState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_positioner::XdgPositioner:
        smithay::wayland::shell::xdg::XdgPositionerUserData
] => XdgShellState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_popup::XdgPopup:
        smithay::wayland::shell::xdg::XdgShellSurfaceUserData
] => XdgShellState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_surface::XdgSurface:
        smithay::wayland::shell::xdg::XdgSurfaceUserData
] => XdgShellState);
smithay::reexports::wayland_server::delegate_dispatch!(State: [
    smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel:
        smithay::wayland::shell::xdg::XdgShellSurfaceUserData
] => XdgShellState);
delegate_cursor_shape!(State);
delegate_data_device!(State);
delegate_primary_selection!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_copied_is_recognized() {
        let types = |list: &[&str]| list.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        assert!(is_secret(&types(&["text/plain", SECRET_HINT])));
        assert!(!is_secret(&types(&[
            "text/plain;charset=utf-8",
            "image/png"
        ])));
        assert!(!is_secret(&[]));
    }
}
