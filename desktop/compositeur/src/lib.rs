//! Compositeur Wayland du desktop, écrit avec Smithay.
//!
//! La bibliothèque porte l'état et la logique, le binaire choisit le backend.
//! Architecture et tests : `docs/compositeur.md`.

mod handlers;
mod input;
pub mod layout;
pub mod winit;

use std::{cell::Cell, ffi::OsString, fmt, os::unix::net::UnixStream, sync::Arc, time::Instant};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState, keyboard::XkbConfig},
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopSignal, Mode as LoopMode, PostAction, generic::Generic,
        },
        wayland_server::{
            Display, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER, Size, Transform},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        shell::xdg::{XdgShellState, XdgToplevelSurfaceData},
        shm::ShmState,
        socket::ListeningSocketSource,
    },
};

use crate::layout::Edges;

/// Ce qui empêche le compositeur de démarrer.
#[derive(thiserror::Error)]
pub enum Error {
    #[error("usage : compositeur --imbrique [commande [arguments…]]")]
    Usage,
    #[error("impossible de créer l'affichage Wayland : {0}")]
    Display(#[from] smithay::reexports::wayland_server::backend::InitError),
    #[error("impossible d'ouvrir la socket Wayland : {0}")]
    Socket(#[from] smithay::reexports::wayland_server::BindError),
    #[error("boucle d'événements : {0}")]
    EventLoop(#[from] smithay::reexports::calloop::Error),
    #[error("clavier : {0}")]
    Keyboard(#[from] smithay::input::keyboard::Error),
    #[error("backend fenêtré : {0}")]
    Winit(#[from] smithay::backend::winit::Error),
    #[error("impossible de lancer la commande : {0}")]
    Spawn(#[from] std::io::Error),
}

// `main` affiche l'erreur avec Debug : on y met le message lisible.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// État du compositeur, partagé par la boucle d'événements.
pub struct State {
    pub display_handle: DisplayHandle,
    pub space: Space<Window>,
    pub seat: Seat<State>,
    pub loop_signal: LoopSignal,
    start_time: Instant,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    seat_state: SeatState<State>,
    popups: PopupManager,
}

/// Données qu'on accroche à chaque fenêtre.
#[derive(Default)]
struct WindowData {
    /// La fenêtre a reçu son premier contenu et a été placée.
    shown: Cell<bool>,
    /// Géométrie à retrouver en quittant l'agrandissement ou le plein écran.
    restore: Cell<Option<Rectangle<i32, Logical>>>,
    /// Redimensionnement en cours : bords tirés, géométrie de départ, geste terminé.
    resize: Cell<Option<ResizeState>>,
}

/// Bords tirés, géométrie de départ, et vrai une fois le geste terminé.
type ResizeState = (Edges, Rectangle<i32, Logical>, bool);

fn window_data(window: &Window) -> &WindowData {
    window.user_data().get_or_insert(WindowData::default)
}

impl State {
    /// Crée l'affichage Wayland, ses protocoles et le siège clavier et souris.
    pub fn new(event_loop: &EventLoop<'static, State>) -> Result<Self, Error> {
        let display = Display::<State>::new()?;
        let dh = display.handle();

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        // AZERTY par défaut, sauf si l'environnement impose une disposition (XKB_DEFAULT_LAYOUT).
        let layout = if std::env::var_os("XKB_DEFAULT_LAYOUT").is_some() {
            ""
        } else {
            "fr"
        };
        seat.add_keyboard(
            XkbConfig {
                layout,
                ..XkbConfig::default()
            },
            200,
            25,
        )?;
        seat.add_pointer();

        event_loop
            .handle()
            .insert_source(
                Generic::new(display, Interest::READ, LoopMode::Level),
                |_, display, state| {
                    // SAFETY: on ne remplace ni ne libère l'affichage ; on ne fait que lire ses clients.
                    let display = unsafe { display.get_mut() };
                    if let Err(err) = display.dispatch_clients(state) {
                        tracing::warn!("lecture des clients Wayland : {err}");
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| err.error)?;

        Ok(Self {
            compositor_state: CompositorState::new::<Self>(&dh),
            xdg_shell_state: XdgShellState::new::<Self>(&dh),
            shm_state: ShmState::new::<Self>(&dh, vec![]),
            seat_state,
            seat,
            space: Space::default(),
            popups: PopupManager::default(),
            loop_signal: event_loop.get_signal(),
            start_time: Instant::now(),
            display_handle: dh,
        })
    }

    /// Ouvre une socket `wayland-N` et y accepte les clients. Rend le nom de la socket.
    pub fn listen(&self, event_loop: &EventLoop<'static, State>) -> Result<OsString, Error> {
        let socket = ListeningSocketSource::new_auto()?;
        let name = socket.socket_name().to_os_string();
        event_loop
            .handle()
            .insert_source(socket, |stream, _, state| state.insert_client(stream))
            .map_err(|err| err.error)?;
        Ok(name)
    }

    /// Branche un client déjà connecté.
    pub fn insert_client(&mut self, stream: UnixStream) {
        let data = Arc::new(ClientState::default());
        if let Err(err) = self.display_handle.insert_client(stream, data) {
            tracing::warn!("client refusé : {err}");
        }
    }

    /// Ajoute un écran de la taille donnée, posé en (0, 0).
    pub fn add_output(&mut self, size: Size<i32, Physical>, transform: Transform) -> Output {
        let properties = PhysicalProperties {
            size: (0, 0).into(),
            subpixel: Subpixel::Unknown,
            make: "compositeur".into(),
            model: "virtuel".into(),
        };
        let output = Output::new("virtuel-1".into(), properties);
        output.create_global::<State>(&self.display_handle);
        let mode = Mode {
            size,
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), Some(transform), None, Some((0, 0).into()));
        output.set_preferred(mode);
        self.space.map_output(&output, (0, 0));
        output
    }

    /// Zone du premier écran, où l'on pose et agrandit les fenêtres.
    pub fn output_area(&self) -> Option<Rectangle<i32, Logical>> {
        let output = self.space.outputs().next()?;
        self.space.output_geometry(output)
    }

    /// Fenêtre dont la surface principale est `surface`.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.toplevel().is_some_and(|t| t.wl_surface() == surface))
            .cloned()
    }

    /// Titre d'une fenêtre, tel que le client l'a donné.
    pub fn title(window: &Window) -> Option<String> {
        let surface = window.toplevel()?.wl_surface();
        smithay::wayland::compositor::with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()?
                .lock()
                .ok()?
                .title
                .clone()
        })
    }

    /// Passe le clavier à une fenêtre et la met au premier plan, ou retire le clavier à tout le monde.
    pub fn focus(&mut self, window: Option<Window>) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        match &window {
            Some(window) => self.space.raise_element(window, true),
            None => self.space.elements().for_each(|w| {
                w.set_activated(false);
            }),
        }
        let surface = window.and_then(|w| w.toplevel().map(|t| t.wl_surface().clone()));
        keyboard.set_focus(self, surface, serial);
        for window in self.space.elements() {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }

    /// Demande à la fenêtre qui a le clavier de se fermer.
    pub fn close_focused(&mut self) {
        let focused = self.seat.get_keyboard().and_then(|k| k.current_focus());
        if let Some(window) = focused.and_then(|s| self.window_for(&s)) {
            Self::close(&window);
        }
    }

    /// Demande à une fenêtre de se fermer : le client décide (enregistrer, confirmer…).
    pub fn close(window: &Window) {
        if let Some(toplevel) = window.toplevel() {
            toplevel.send_close();
        }
    }

    /// Surface sous le pointeur, avec la position de son origine.
    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let (window, location) = self.space.element_under(pos)?;
        window
            .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
            .map(|(surface, p)| (surface, (p + location).to_f64()))
    }

    /// Ménage après chaque tour de boucle : fenêtres mortes, popups, envoi aux clients.
    pub fn refresh(&mut self) {
        self.space.refresh();
        self.popups.cleanup();
        if let Err(err) = self.display_handle.flush_clients() {
            tracing::warn!("envoi aux clients Wayland : {err}");
        }
    }
}

/// Données de chaque client connecté.
#[derive(Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
