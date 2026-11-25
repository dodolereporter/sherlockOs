//! Réponses aux protocoles : wl_compositor, wl_shm, wl_seat, wl_output et xdg-shell.

use std::{cell::Cell, sync::OnceLock};

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_cursor_shape, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell,
    desktop::{PopupKind, Window},
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::{ResizeEdge, State as XdgState},
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{self, CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelState, ToplevelSurface, XdgShellHandler,
            XdgShellState,
        },
        shm::{ShmHandler, ShmState},
        tablet_manager::TabletSeatHandler,
    },
};

use crate::{
    ClientState, State,
    input::GestureSource,
    layout::{self, Edges, Zone},
    window_data,
};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Tous nos clients passent par `insert_client` ; l'état vide évite seulement de paniquer.
        static EMPTY: OnceLock<CompositorClientState> = OnceLock::new();
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
                self.space.map_element(window.clone(), loc, false);
            }
            self.focus(Some(window));
            return;
        }

        // Redimensionnement en cours : le bord opposé à celui qu'on tire reste en place.
        if let Some(resize) = data.resize.get() {
            let (loc, next) = resize.on_commit(size);
            data.resize.set(next);
            self.space.map_element(window, loc, false);
        }
    }

    /// Geste demandé par le client, qui dessine lui-même sa barre de titre et ses bords.
    ///
    /// Comme le veut xdg-shell, il n'est accordé que sur notre siège, pendant l'appui d'un bouton
    /// sur une surface de ce client, et avec le serial de cet appui.
    fn client_gesture(
        &mut self,
        surface: &ToplevelSurface,
        seat: &wl_seat::WlSeat,
        serial: Serial,
        edges: Option<Edges>,
    ) {
        let pointer = Seat::<State>::from_resource(seat)
            .filter(|seat| *seat == self.seat)
            .and_then(|seat| seat.get_pointer());
        let start = pointer
            .as_ref()
            .filter(|pointer| pointer.has_grab(serial))
            .and_then(|pointer| pointer.grab_start_data());
        let same_client = start.as_ref().is_some_and(|start| {
            start
                .focus
                .as_ref()
                .is_some_and(|(focus, _)| focus.id().same_client_as(&surface.wl_surface().id()))
        });
        // Une fenêtre en plein écran ne bouge pas : le client garde son appui.
        let window = self
            .window_for(surface.wl_surface())
            .filter(|w| !is_fullscreen(w));
        let (Some(pointer), Some(start), Some(window), true, false) =
            (pointer, start, window, same_client, self.gesture_active())
        else {
            tracing::debug!(?serial, ?edges, "demande de geste du client refusée");
            return;
        };
        // On reprend le pointeur au client : il ne reçoit plus rien jusqu'au relâchement.
        let time = self.start_time.elapsed().as_millis() as u32;
        pointer.unset_grab(self, serial, time);
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
        change: impl FnOnce(&mut ToplevelState, &Cell<Option<Zone>>),
    ) {
        let (Some(toplevel), Some(area)) = (window.toplevel(), self.output_area()) else {
            return;
        };
        let data = window_data(window);
        let target = toplevel.with_pending_state(|s| {
            change(s, &data.zone);
            let filled =
                s.states.contains(XdgState::Maximized) || s.states.contains(XdgState::Fullscreen);
            match (filled, data.zone.get()) {
                (true, _) => Some(area),
                (false, zone) => zone.map(|zone| zone.geometry(area)),
            }
        });
        if let Some(rect) = target {
            if data.restore.get().is_none() {
                data.restore.set(self.space.element_geometry(window));
            }
            toplevel.with_pending_state(|s| s.size = Some(rect.size));
            self.space.map_element(window.clone(), rect.loc, true);
        } else if let Some(restore) = data.restore.take() {
            // Taille nulle : la fenêtre n'avait pas encore de contenu, le client choisit.
            let size = (restore.size.w > 0 && restore.size.h > 0).then_some(restore.size);
            toplevel.with_pending_state(|s| s.size = size);
            self.space.map_element(window.clone(), restore.loc, true);
        }
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
    }

    /// Cale la fenêtre dans `zone`, ou lui rend sa place d'avant. Sans effet en plein écran.
    pub(crate) fn snap(&mut self, window: &Window, zone: Option<Zone>) {
        if is_fullscreen(window) {
            return;
        }
        self.rearrange(window, |s, current| {
            s.states.unset(XdgState::Maximized);
            current.set(None);
            match zone {
                Some(Zone::Maximized) => {
                    s.states.set(XdgState::Maximized);
                }
                other => current.set(other),
            }
        });
    }

    fn surface_window(&self, surface: &ToplevelSurface) -> Option<Window> {
        self.window_for(surface.wl_surface())
    }
}

/// La fenêtre est en plein écran, ou va l'être.
pub(crate) fn is_fullscreen(window: &Window) -> bool {
    window
        .toplevel()
        .is_some_and(|t| t.with_pending_state(|s| s.states.contains(XdgState::Fullscreen)))
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
            let next = self.space.elements().next_back().cloned();
            self.focus(next);
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
        self.client_gesture(&surface, &seat, serial, None);
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
            self.client_gesture(&surface, &seat, serial, Some(edges));
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface) {
            self.rearrange(&window, |s, _| {
                s.states.set(XdgState::Maximized);
            });
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface) {
            self.rearrange(&window, |s, zone| {
                s.states.unset(XdgState::Maximized);
                zone.set(None);
            });
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        if let Some(window) = self.surface_window(&surface) {
            self.rearrange(&window, |s, _| {
                s.states.set(XdgState::Fullscreen);
                s.fullscreen_output = output;
            });
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.surface_window(&surface) {
            self.rearrange(&window, |s, _| {
                s.states.unset(XdgState::Fullscreen);
                s.fullscreen_output = None;
            });
        }
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
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

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_output!(State);
delegate_xdg_shell!(State);
delegate_cursor_shape!(State);
