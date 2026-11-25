//! XWayland : le serveur X11 qui fait tourner les applications X11 dans le compositeur, et le
//! gestionnaire de fenêtres X11 de Smithay (`X11Wm`) qui en fait des fenêtres comme les autres.
//!
//! XWayland est un client Wayland du compositeur. Chaque fenêtre X11 y a une surface Wayland,
//! que le protocole xwayland-shell associe à la fenêtre X11. Les fenêtres gérées passent par
//! `Window` et ont les mêmes gestes, le même rendu et les mêmes états que les fenêtres
//! xdg-shell. Les fenêtres override-redirect (menus, bulles) se placent elles-mêmes.

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
    reexports::{calloop::EventLoop, wayland_server::protocol::wl_surface::WlSurface},
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
    window_data,
};

impl State {
    /// Lance XWayland. Une fois prêt, son gestionnaire de fenêtres démarre, `x_display` donne
    /// son numéro d'affichage, puis `ready` est appelé avec ce numéro (`None` s'il a échoué).
    pub fn start_xwayland(
        &mut self,
        event_loop: &EventLoop<'static, State>,
        ready: impl FnOnce(&mut State, Option<u32>) + 'static,
    ) -> Result<(), Error> {
        ensure_x11_unix_dir().map_err(Error::Xwayland)?;
        let (xwayland, client) = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::null(),
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
                                tracing::info!(display = display_number, "XWayland prêt");
                                state.xwm = Some(xwm);
                                state.x_display = Some(display_number);
                                Some(display_number)
                            }
                            Err(err) => {
                                tracing::warn!("gestionnaire de fenêtres X11 : {err}");
                                None
                            }
                        }
                    }
                    XWaylandEvent::Error => {
                        tracing::warn!("XWayland s'est arrêté pendant son démarrage");
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

    /// Arrête XWayland : il se termine, et son verrou et sa socket (`/tmp/.X11-unix/X<N>`)
    /// disparaissent. À appeler en quittant : la source de X11Wm garde une poignée de la
    /// boucle, qui n'est donc jamais libérée, ni XWayland avec elle.
    pub fn stop_xwayland(&mut self) {
        if let Some(token) = self.xwayland.take() {
            self.loop_handle.remove(token);
        }
    }
}

/// Crée `/tmp/.X11-unix`, où vont les sockets des serveurs X, s'il n'existe pas : sur une
/// machine sans serveur X, comme celle de la CI, personne ne l'a créé. Il est à tout le monde,
/// avec le bit collant, comme le crée systemd.
fn ensure_x11_unix_dir() -> io::Result<()> {
    let dir = Path::new("/tmp/.X11-unix");
    if dir.is_dir() {
        return Ok(());
    }
    DirBuilder::new().mode(0o1777).create(dir)?;
    // Le masque du processus a pu retirer des droits à la création.
    fs::set_permissions(dir, Permissions::from_mode(0o1777))
}

/// Bords tirés d'une demande `_NET_WM_MOVERESIZE`.
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
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        match self.xwm.as_mut() {
            Some(xwm) => xwm,
            // Smithay n'appelle ce gestionnaire que pour un X11Wm qui tourne, et on le range
            // dans `self.xwm` avant de rendre la main à la boucle : il ne peut pas manquer.
            None => unreachable!("gestionnaire de fenêtres X11 absent"),
        }
    }

    fn new_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    fn new_override_redirect_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    /// Une fenêtre gérée demande à s'afficher : on la centre, à la taille qu'elle demande. Elle
    /// est montrée et reçoit le clavier quand XWayland lui associe sa surface.
    fn map_window_request(&mut self, _xwm: XwmId, surface: X11Surface) {
        if let Err(err) = surface.set_mapped(true) {
            tracing::warn!("affichage de la fenêtre X11 : {err}");
            return;
        }
        let size = surface.geometry().size;
        let loc = self
            .output_area()
            .map(|area| layout::centered(area, size))
            .unwrap_or_default();
        tracing::debug!(title = surface.title(), ?loc, ?size, "fenêtre X11");
        self.space
            .map_element(Window::new_x11_window(surface.clone()), loc, false);
        configure_x11(&surface, Rectangle::new(loc, size));
    }

    /// Un menu ou une bulle s'affiche à la place qu'il a choisie, au-dessus des fenêtres.
    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, surface: X11Surface) {
        let loc = surface.geometry().loc;
        let window = Window::new_x11_window(surface);
        window.override_z_index(RenderZindex::Overlay as u8);
        self.space.map_element(window, loc, false);
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
            tracing::warn!("retrait de la fenêtre X11 : {err}");
        }
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _surface: X11Surface) {}

    /// Une fenêtre flottante prend la taille qu'elle demande, mais pas la place : c'est le
    /// compositeur qui la pose. Agrandie, calée ou en plein écran, elle garde la sienne.
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
        // Même refusée, la demande reçoit une réponse : la géométrie actuelle.
        if let Err(err) = surface.configure(rect) {
            tracing::warn!("configuration de la fenêtre X11 : {err}");
        }
    }

    /// Un menu bouge de lui-même ; une fenêtre gérée, elle, est à la place qu'on lui a donnée.
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

    /// Un client X11 ne lit le presse-papiers que si une fenêtre X11 a le clavier, comme un
    /// client Wayland.
    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        let focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        matches!(focus, Some(KeyboardFocus::X11(surface)) if surface.xwm_id() == Some(xwm))
    }

    /// Un client X11 lit la sélection d'un client Wayland.
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
            tracing::warn!(?selection, "sélection Wayland lue par XWayland : {err}");
        }
    }

    /// Un client X11 pose une sélection : le compositeur la pose côté Wayland pour son compte.
    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        let dh = &self.display_handle;
        match selection {
            SelectionTarget::Clipboard => set_data_device_selection(dh, &self.seat, mime_types, ()),
            SelectionTarget::Primary => set_primary_selection(dh, &self.seat, mime_types, ()),
        }
    }

    /// Le client X11 qui tenait la sélection l'a lâchée. Seule celle qu'on avait posée pour lui
    /// est effacée, pas celle d'un client Wayland arrivée depuis.
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

    /// La fenêtre X11 a sa surface : elle se montre et reçoit le clavier, comme une fenêtre
    /// Wayland à son premier contenu. Avant, XWayland n'aurait pas reçu l'entrée du clavier.
    fn surface_associated(&mut self, _xwm: XwmId, _surface: WlSurface, surface: X11Surface) {
        let Some(window) = self.x11_window(&surface) else {
            return;
        };
        let data = window_data(&window);
        if !surface.is_override_redirect() && !data.shown.get() {
            data.shown.set(true);
            self.focus(Some(window));
        }
    }
}

delegate_xwayland_shell!(State);
