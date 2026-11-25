//! Réponses aux protocoles : wl_compositor, wl_shm, wl_seat, wl_output et xdg-shell.

use std::sync::OnceLock;

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_output, delegate_seat, delegate_shm, delegate_xdg_shell,
    desktop::{PopupKind, Window},
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState,
        wayland_server::{
            Client,
            protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
        },
    },
    utils::Serial,
    wayland::{
        buffer::BufferHandler,
        compositor::{self, CompositorClientState, CompositorHandler, CompositorState},
        output::OutputHandler,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::{ClientState, State, layout, window_data};

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

        // Redimensionnement par le haut ou la gauche : le bord opposé reste en place.
        if let Some((edges, initial, done)) = data.resize.get() {
            if done {
                data.resize.set(None);
            }
            let loc = layout::anchored(initial, edges, size);
            self.space.map_element(window, loc, false);
        }
    }

    /// Fait entrer la fenêtre dans l'état `mode` (agrandie ou plein écran), ou l'en fait sortir.
    fn set_filled(
        &mut self,
        surface: &ToplevelSurface,
        mode: XdgState,
        on: bool,
        output: Option<WlOutput>,
    ) {
        let (Some(window), Some(area)) =
            (self.window_for(surface.wl_surface()), self.output_area())
        else {
            return;
        };
        let data = window_data(&window);
        let still_filled = surface.with_pending_state(|s| {
            if on {
                s.states.set(mode);
            } else {
                s.states.unset(mode);
            }
            if mode == XdgState::Fullscreen {
                s.fullscreen_output = output;
            }
            s.states.contains(XdgState::Maximized) || s.states.contains(XdgState::Fullscreen)
        });
        if still_filled {
            if data.restore.get().is_none() {
                data.restore.set(self.space.element_geometry(&window));
            }
            surface.with_pending_state(|s| s.size = Some(area.size));
            self.space.map_element(window, area.loc, true);
        } else if let Some(restore) = data.restore.take() {
            // Taille nulle : la fenêtre n'avait pas encore de contenu, le client choisit.
            let size = (restore.size.w > 0 && restore.size.h > 0).then_some(restore.size);
            surface.with_pending_state(|s| s.size = size);
            self.space.map_element(window, restore.loc, true);
        }
        if surface.is_initial_configure_sent() {
            surface.send_pending_configure();
        }
    }
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

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.set_filled(&surface, XdgState::Maximized, true, None);
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.set_filled(&surface, XdgState::Maximized, false, None);
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, output: Option<WlOutput>) {
        self.set_filled(&surface, XdgState::Fullscreen, true, output);
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.set_filled(&surface, XdgState::Fullscreen, false, None);
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
