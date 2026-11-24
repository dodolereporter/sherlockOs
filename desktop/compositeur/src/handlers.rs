//! Réponses aux protocoles : wl_compositor, wl_shm, wl_seat, wl_output et xdg-shell.

use std::sync::OnceLock;

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell,
    desktop::{PopupKind, Window},
    input::{
        Seat, SeatHandler, SeatState,
        pointer::{CursorImageStatus, Focus, GrabStartData},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            Client, Resource,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface,
        },
        output::OutputHandler,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::{
    ClientState, State,
    input::{GrabKind, WindowGrab},
    layout::{self, Edges},
    window_data,
};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // Tous nos clients passent par `insert_client` ; l'état vide ne sert qu'à ne jamais paniquer.
        static EMPTY: OnceLock<CompositorClientState> = OnceLock::new();
        match client.get_data::<ClientState>() {
            Some(data) => &data.compositor_state,
            None => EMPTY.get_or_init(CompositorClientState::default),
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
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

        // Premier contenu : on centre la fenêtre, sauf si elle a déjà demandé tout l'écran,
        // et on lui donne le clavier.
        if !data.shown.get() && size.w > 0 && size.h > 0 {
            data.shown.set(true);
            let filled = toplevel.with_pending_state(|s| {
                s.states.contains(xdg_toplevel::State::Maximized)
                    || s.states.contains(xdg_toplevel::State::Fullscreen)
            });
            if let (false, Some(area)) = (filled, self.output_area()) {
                self.space
                    .map_element(window.clone(), layout::centered(area, size), false);
            }
            self.focus(Some(window));
            return;
        }

        // Redimensionnement par le haut ou la gauche : on recale la fenêtre sur sa nouvelle taille.
        if let Some((edges, initial, done)) = data.resize.get() {
            if done {
                data.resize.set(None);
            }
            if edges.top || edges.left {
                let loc = layout::anchored(initial, edges, size);
                self.space.map_element(window, loc, false);
            }
        }
    }

    /// Agrandit la fenêtre ou la passe en plein écran : elle prend tout l'écran.
    fn fill(
        &mut self,
        surface: &ToplevelSurface,
        mode: xdg_toplevel::State,
        output: Option<WlOutput>,
    ) {
        let (Some(window), Some(area)) =
            (self.window_for(surface.wl_surface()), self.output_area())
        else {
            return;
        };
        let data = window_data(&window);
        if data.restore.get().is_none() {
            data.restore.set(self.space.element_geometry(&window));
        }
        surface.with_pending_state(|s| {
            s.states.set(mode);
            s.size = Some(area.size);
            if mode == xdg_toplevel::State::Fullscreen {
                s.fullscreen_output = output;
            }
        });
        self.space.map_element(window, area.loc, true);
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }

    /// Quitte l'agrandissement ou le plein écran, et rend sa géométrie à la fenêtre.
    fn unfill(&mut self, surface: &ToplevelSurface, mode: xdg_toplevel::State) {
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        let still_filled = surface.with_pending_state(|s| {
            s.states.unset(mode);
            if mode == xdg_toplevel::State::Fullscreen {
                s.fullscreen_output = None;
            }
            s.states.contains(xdg_toplevel::State::Maximized)
                || s.states.contains(xdg_toplevel::State::Fullscreen)
        });
        if !still_filled && let Some(restore) = window_data(&window).restore.take() {
            let size = (restore.size.w > 0 && restore.size.h > 0).then_some(restore.size);
            surface.with_pending_state(|s| s.size = size);
            self.space.map_element(window, restore.loc, true);
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }

    /// Lance un geste demandé par le client (barre de titre ou bord dessinés par lui).
    fn client_grab(
        &mut self,
        surface: &ToplevelSurface,
        seat: &wl_seat::WlSeat,
        serial: Serial,
        edges: Option<Edges>,
    ) {
        let Some(seat) = Seat::<State>::from_resource(seat) else {
            return;
        };
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        let Some(window) = self.window_for(surface.wl_surface()) else {
            return;
        };
        // Le client n'a droit au geste que si son bouton est encore enfoncé sur lui.
        if !pointer.has_grab(serial) {
            return;
        }
        let Some(start) = pointer.grab_start_data() else {
            return;
        };
        let same_client = start
            .focus
            .as_ref()
            .is_some_and(|(focus, _)| focus.id().same_client_as(&surface.wl_surface().id()));
        if !same_client {
            return;
        }
        let start = GrabStartData {
            focus: None,
            ..start
        };
        if let Some(grab) = self.start_grab(start, window, edges) {
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    /// Prépare le geste de déplacement (`edges` vide) ou de redimensionnement d'une fenêtre.
    pub(crate) fn start_grab(
        &mut self,
        start: GrabStartData<State>,
        window: Window,
        edges: Option<Edges>,
    ) -> Option<WindowGrab> {
        let rect = self.space.element_geometry(&window)?;
        let kind = match edges {
            None => GrabKind::Move { initial: rect.loc },
            Some(edges) => {
                window_data(&window).resize.set(Some((edges, rect, false)));
                GrabKind::Resize {
                    edges,
                    initial: rect,
                    last: rect.size,
                }
            }
        };
        Some(WindowGrab {
            start,
            window,
            kind,
        })
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
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

    fn title_changed(&mut self, surface: ToplevelSurface) {
        if let Some(window) = self.window_for(surface.wl_surface()) {
            tracing::debug!(titre = ?State::title(&window), "nouveau titre");
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
        // Les menus se ferment quand le client le décide : pas de saisie exclusive pour l'instant.
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.client_grab(&surface, &seat, serial, None);
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let edges = Edges::from_xdg_bits(edges as u32);
        self.client_grab(&surface, &seat, serial, Some(edges));
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.fill(&surface, xdg_toplevel::State::Maximized, None);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.unfill(&surface, xdg_toplevel::State::Maximized);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        self.fill(&surface, xdg_toplevel::State::Fullscreen, output);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.unfill(&surface, xdg_toplevel::State::Fullscreen);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
}

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
