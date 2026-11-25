//! Réponses aux protocoles : wl_compositor, wl_shm, wl_seat, wl_output, xdg-shell, et le
//! presse-papiers (wl_data_device et sélection primaire).

use std::{cell::Cell, os::fd::OwnedFd, sync::OnceLock};

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_cursor_shape, delegate_data_device, delegate_output,
    delegate_primary_selection, delegate_seat, delegate_shm, delegate_xdg_shell,
    desktop::{PopupKind, Window},
    input::{
        Seat, SeatHandler, SeatState,
        pointer::{CursorImageStatus, GrabStartData},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Size},
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
    ClientState, State,
    focus::KeyboardFocus,
    input::GestureSource,
    layout::{self, Edges, Zone},
    window_data,
};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Nos clients passent par `insert_client`, sauf XWayland, que Smithay branche lui-même.
        // L'état vide évite seulement de paniquer.
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
        if !compositor::is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = compositor::get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for(&root) {
                window.on_commit();
                if &root == surface {
                    self.toplevel_commit(window);
                }
            }
        }
        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
            && let Err(err) = popup.send_configure()
        {
            tracing::warn!("configuration du popup : {err}");
        }
    }
}

impl State {
    fn toplevel_commit(&mut self, window: Window) {
        let Some(toplevel) = window.toplevel() else {
            return;
        };
        if !toplevel.is_initial_configure_sent() {
            toplevel.send_configure();
            return;
        }
        let data = window_data(&window);
        let size = window.geometry().size;
        let filled = toplevel.with_pending_state(|s| {
            s.states.contains(XdgState::Maximized) || s.states.contains(XdgState::Fullscreen)
        });

        // Premier contenu : on centre la fenêtre (sauf si elle prend tout l'écran) et on lui
        // donne le clavier.
        if !data.shown.get() && size.w > 0 && size.h > 0 {
            data.shown.set(true);
            if let (false, Some(area)) = (filled, self.output_area()) {
                let loc = layout::centered(area, size);
                self.move_window(&window, loc, false);
            }
            self.focus(Some(window));
            return;
        }

        // Redimensionnement en cours : le bord opposé à celui qu'on tire reste en place.
        if let Some(resize) = data.resize.get() {
            let (loc, next) = resize.on_commit(size);
            data.resize.set(next);
            self.move_window(&window, loc, false);
        }
    }

    /// Pose la fenêtre en `loc` et la passe au premier plan. Une fenêtre X11 apprend sa place :
    /// XWayland en a besoin pour placer ses menus et traduire le pointeur.
    pub(crate) fn move_window(
        &mut self,
        window: &Window,
        loc: Point<i32, Logical>,
        activate: bool,
    ) {
        self.space.map_element(window.clone(), loc, activate);
        if let Some(surface) = window.x11_surface() {
            configure_x11(surface, Rectangle::new(loc, surface.geometry().size));
        }
    }

    /// Demande au client la taille `size`. `None` le laisse choisir : une fenêtre X11 garde
    /// alors la sienne. Une fenêtre Wayland l'apprend au prochain envoi de sa configuration,
    /// une fenêtre X11 tout de suite.
    pub(crate) fn request_size(&self, window: &Window, size: Option<Size<i32, Logical>>) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.with_pending_state(|s| s.size = size);
        } else if let (Some(surface), Some(size)) = (window.x11_surface(), size) {
            let loc = self.space.element_location(window).unwrap_or_default();
            configure_x11(surface, Rectangle::new(loc, size));
        }
    }

    /// Geste demandé par une fenêtre xdg-shell, qui dessine elle-même sa barre de titre et ses
    /// bords. Comme le veut xdg-shell, il n'est accordé que sur notre siège, et avec le serial
    /// de l'appui en cours.
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
        tracing::debug!(?serial, "geste demandé par un client Wayland");
        self.client_gesture(self.window_for(surface.wl_surface()), start, edges);
    }

    /// Geste demandé par une fenêtre X11 (`_NET_WM_MOVERESIZE`). Le serveur X ne donne pas de
    /// serial : il faut qu'un bouton soit tenu sur une surface de XWayland.
    pub(crate) fn x11_gesture(&mut self, surface: &X11Surface, edges: Option<Edges>) {
        let start = self
            .seat
            .get_pointer()
            .filter(|pointer| pointer.is_grabbed())
            .and_then(|pointer| pointer.grab_start_data());
        tracing::debug!("geste demandé par un client X11");
        self.client_gesture(self.x11_window(surface), start, edges);
    }

    /// Geste demandé par le client. Il n'est accordé que pendant l'appui d'un bouton sur une
    /// surface de ce client (`start`), et jamais en plein écran : le client garde son appui.
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
            tracing::debug!(?edges, "demande de geste du client refusée");
            return;
        };
        // On reprend le pointeur au client : il ne reçoit plus rien jusqu'au relâchement.
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

    /// Change l'état d'une fenêtre (agrandie, plein écran, calée dans une zone) puis la pose :
    /// sur tout l'écran, dans sa zone, ou à sa place d'avant. Cette place est gardée quand la
    /// fenêtre quitte l'état flottant, et rendue quand elle y revient.
    pub(crate) fn rearrange(
        &mut self,
        window: &Window,
        change: impl FnOnce(&mut Arrangement, &Cell<Option<Zone>>),
    ) {
        let Some(area) = self.output_area() else {
            return;
        };
        let data = window_data(window);
        let mut arrangement = Arrangement::of(window);
        change(&mut arrangement, &data.zone);
        arrangement.apply(window);
        let target = match (
            arrangement.maximized || arrangement.fullscreen,
            data.zone.get(),
        ) {
            (true, _) => Some(area),
            (false, zone) => zone.map(|zone| zone.geometry(area)),
        };
        if target.is_some() || data.restore.get().is_some() {
            // Calage, agrandissement, plein écran ou retour : la fenêtre glisse à sa place.
            data.motion.borrow_mut().animate_next_change();
        }
        if let Some(rect) = target {
            if data.restore.get().is_none() {
                data.restore.set(self.space.element_geometry(window));
            }
            self.request_size(window, Some(rect.size));
            self.move_window(window, rect.loc, true);
        } else if let Some(restore) = data.restore.take() {
            // Taille nulle : la fenêtre n'avait pas encore de contenu, le client choisit.
            let size = (restore.size.w > 0 && restore.size.h > 0).then_some(restore.size);
            self.request_size(window, size);
            self.move_window(window, restore.loc, true);
        }
        if let Some(toplevel) = window.toplevel()
            && toplevel.is_initial_configure_sent()
        {
            toplevel.send_pending_configure();
        }
    }

    /// Cale la fenêtre dans `zone`, ou lui rend sa place d'avant. Sans effet en plein écran.
    pub(crate) fn snap(&mut self, window: &Window, zone: Option<Zone>) {
        if is_fullscreen(window) {
            return;
        }
        self.rearrange(window, |a, current| {
            a.maximized = zone == Some(Zone::Maximized);
            current.set(zone.filter(|z| *z != Zone::Maximized));
        });
    }

    fn surface_window(&self, surface: &ToplevelSurface) -> Option<Window> {
        self.window_for(surface.wl_surface())
    }
}

/// États qu'une fenêtre connaît : ceux de xdg-shell, ou `_NET_WM_STATE` pour une fenêtre X11.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Arrangement {
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Arrangement {
    /// États de la fenêtre, y compris ceux qu'elle n'a pas encore reçus.
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

    /// Donne ces états à la fenêtre. Une fenêtre Wayland les reçoit avec sa prochaine
    /// configuration, une fenêtre X11 tout de suite.
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
                tracing::warn!("états de la fenêtre X11 : {err}");
            }
        }
    }
}

/// Place et taille d'une fenêtre X11, dans les coordonnées de l'écran, qui sont celles de
/// l'espace : l'écran est posé en (0, 0).
pub(crate) fn configure_x11(surface: &X11Surface, rect: Rectangle<i32, Logical>) {
    if surface.is_override_redirect() || surface.geometry() == rect {
        return;
    }
    if let Err(err) = surface.configure(rect) {
        tracing::warn!("configuration de la fenêtre X11 : {err}");
    }
}

/// La fenêtre est en plein écran, ou va l'être.
pub(crate) fn is_fullscreen(window: &Window) -> bool {
    Arrangement::of(window).fullscreen
}

/// La fenêtre est agrandie, ou va l'être.
pub(crate) fn is_maximized(window: &Window) -> bool {
    Arrangement::of(window).maximized
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Posée pour de bon à son premier contenu, quand sa taille est connue.
        let loc = self.output_area().map(|a| a.loc).unwrap_or_default();
        self.space
            .map_element(Window::new_wayland_window(surface), loc, false);
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for(surface.wl_surface()) {
            self.space.unmap_elem(&window);
            self.focus(self.top_window());
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        if let Err(err) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!("popup refusé : {err}");
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

    fn grab(&mut self, _surface: PopupSurface, _seat: wl_seat::WlSeat, _serial: Serial) {
        // Pas de saisie exclusive pour les menus : ils se ferment quand le client le décide.
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
            self.rearrange(&window, |a, zone| {
                a.maximized = false;
                zone.set(None);
            });
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        if let Some(window) = self.surface_window(&surface) {
            surface.with_pending_state(|s| s.fullscreen_output = output);
            self.rearrange(&window, |a, _| a.fullscreen = true);
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

    /// Seul le client qui a le clavier lit et pose le presse-papiers et la sélection primaire.
    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&KeyboardFocus>) {
        let dh = &self.display_handle;
        let client = focused
            .and_then(WaylandFocus::wl_surface)
            .and_then(|surface| dh.get_client(surface.id()).ok());
        set_data_device_focus(dh, seat, client.clone());
        set_primary_focus(dh, seat, client);
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        self.cursor_status = image;
    }
}

// Exigé par le protocole des formes de curseur, qui accepte aussi les stylets. Sans tablette
// annoncée, aucun client n'en a.
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

/// Le presse-papiers d'un client Wayland est relayé à XWayland, et l'inverse : quand un client
/// X11 le pose, le compositeur le pose côté Wayland pour son compte (voir `xwayland.rs`).
impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        target: SelectionTarget,
        source: Option<smithay::wayland::selection::SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.new_selection(target, source.map(|s| s.mime_types()))
        {
            tracing::warn!(?target, "sélection passée à XWayland : {err}");
        }
    }

    fn send_selection(
        &mut self,
        target: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut()
            && let Err(err) = xwm.send_selection(target, mime_type, fd, self.loop_handle.clone())
        {
            tracing::warn!(?target, "sélection lue chez XWayland : {err}");
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

// Le glisser-déposer entre clients Wayland suit le comportement par défaut de Smithay. Entre
// Wayland et X11, il vient au lot D-29.
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
delegate_xdg_shell!(State);
delegate_cursor_shape!(State);
delegate_data_device!(State);
delegate_primary_selection!(State);
