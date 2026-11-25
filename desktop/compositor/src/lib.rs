//! Compositeur Wayland du desktop, écrit avec Smithay.
//!
//! La bibliothèque porte l'état et la logique, le binaire choisit le backend.
//! Architecture et tests : `docs/compositeur.md`.

pub mod anim;
pub mod focus;
mod handlers;
mod input;
mod layer;
pub mod layout;
pub mod output;
pub mod render;
pub mod stats;
pub mod winit;
mod xwayland;

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    ffi::{OsStr, OsString},
    fmt,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use smithay::{
    desktop::{PopupManager, Space, Window, layer_map_for_output},
    input::{
        Seat, SeatState,
        keyboard::{Keycode, XkbConfig},
        pointer::CursorImageStatus,
    },
    reexports::{
        calloop::{
            EventLoop, Interest, LoopHandle, LoopSignal, Mode as LoopMode, PostAction,
            RegistrationToken, generic::Generic,
        },
        wayland_server::{
            Display, DisplayHandle, ListeningSocket,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{IsAlive, Logical, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        cursor_shape::CursorShapeManagerState,
        fractional_scale::FractionalScaleManagerState,
        output::OutputManagerState,
        seat::WaylandFocus,
        selection::{data_device::DataDeviceState, primary_selection::PrimarySelectionState},
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, XdgToplevelSurfaceData},
        },
        shm::ShmState,
        viewporter::ViewporterState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Surface, X11Wm},
};

use crate::{
    focus::KeyboardFocus,
    layout::{Resize, Zone},
};

/// Ce qui empêche le compositeur de démarrer.
#[derive(thiserror::Error)]
pub enum Error {
    #[error(
        "usage : compositeur --imbrique [--mesurer] [--ecrans N] [--echelle E]… [commande [arguments…]]"
    )]
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
    #[error("programmes du rendu : {0}")]
    Render(#[from] smithay::backend::renderer::gles::GlesError),
    #[error("impossible de lancer la commande : {0}")]
    Spawn(#[from] std::io::Error),
    #[error("impossible de lancer Xwayland : {0}")]
    Xwayland(std::io::Error),
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
    loop_handle: LoopHandle<'static, State>,
    start_time: Instant,
    compositor_state: CompositorState,
    xdg_shell_state: XdgShellState,
    shm_state: ShmState,
    _cursor_shape_state: CursorShapeManagerState,
    xwayland_shell_state: XWaylandShellState,
    layer_shell_state: WlrLayerShellState,
    _output_manager_state: OutputManagerState,
    _fractional_scale_state: FractionalScaleManagerState,
    _viewporter_state: ViewporterState,
    data_device_state: DataDeviceState,
    primary_selection_state: PrimarySelectionState,
    seat_state: SeatState<State>,
    popups: PopupManager,
    /// Menu that a click elsewhere closes.
    popup_grab: Option<layer::PopupGrab>,
    /// Anchored surfaces unmapped by a null buffer, with their screen: their next commit maps
    /// them again.
    unmapped_layers: Vec<(smithay::desktop::LayerSurface, smithay::output::Output)>,
    /// Screens and where they draw in the host frame.
    screens: Vec<output::Screen>,
    gesture: Option<input::Gesture>,
    /// Curseur demandé par le client qui a le pointeur.
    cursor_status: CursorImageStatus,
    /// Touches d'un raccourci : leur relâchement ne va pas au client, qui n'a pas vu l'appui.
    intercepted_keys: Vec<Keycode>,
    /// Le compositeur tourne dans une fenêtre d'une autre session (`--imbrique`).
    nested: bool,
    /// Statistiques des images, avec `--mesurer`.
    pub frame_stats: Option<stats::FrameStats>,
    /// Gestionnaire des fenêtres X11, une fois XWayland prêt.
    xwm: Option<X11Wm>,
    /// Numéro de l'affichage X11 (`DISPLAY=:N`), une fois XWayland prêt.
    pub x_display: Option<u32>,
    /// Source de XWayland dans la boucle, pour l'arrêter.
    xwayland: Option<RegistrationToken>,
    /// Listening Wayland socket in the loop, removed by `shutdown`.
    listener: Option<RegistrationToken>,
}

/// Données qu'on accroche à chaque fenêtre.
#[derive(Default)]
struct WindowData {
    /// La fenêtre a reçu son premier contenu et a été placée. Une fenêtre X11 est placée à sa
    /// demande d'affichage, et montrée quand XWayland lui associe sa surface Wayland.
    shown: Cell<bool>,
    /// Géométrie à retrouver en quittant l'agrandissement, le plein écran ou une zone.
    restore: Cell<Option<Rectangle<i32, Logical>>>,
    /// Moitié ou quart de l'écran où la fenêtre est calée. L'agrandissement, lui, est un état
    /// xdg-shell que le client connaît.
    zone: Cell<Option<Zone>>,
    /// Redimensionnement en cours, gardé jusqu'au dernier envoi du client.
    resize: Cell<Option<Resize>>,
    /// Ouverture et changements de géométrie animés.
    motion: RefCell<anim::WindowMotion>,
}

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
            // Les clients qui le connaissent nomment leur curseur au lieu d'en dessiner un.
            _cursor_shape_state: CursorShapeManagerState::new::<Self>(&dh),
            // Seul XWayland voit ce protocole : il associe ses fenêtres X11 à ses surfaces.
            xwayland_shell_state: XWaylandShellState::new::<Self>(&dh),
            // Anchored surfaces of the shell (ADR 0005).
            layer_shell_state: WlrLayerShellState::new::<Self>(&dh),
            // Logical place and size of each screen, which toolkits read to size their surfaces.
            _output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&dh),
            // Fractional scale of each surface's screen, and buffers cropped and scaled by the
            // client (viewporter), with which it draws at that scale.
            _fractional_scale_state: FractionalScaleManagerState::new::<Self>(&dh),
            _viewporter_state: ViewporterState::new::<Self>(&dh),
            // Presse-papiers (CLIPBOARD) et sélection primaire, que XWayland relaie aussi.
            data_device_state: DataDeviceState::new::<Self>(&dh),
            primary_selection_state: PrimarySelectionState::new::<Self>(&dh),
            seat_state,
            seat,
            space: Space::default(),
            popups: PopupManager::default(),
            popup_grab: None,
            unmapped_layers: Vec::new(),
            screens: Vec::new(),
            gesture: None,
            cursor_status: CursorImageStatus::default_named(),
            intercepted_keys: Vec::new(),
            nested: false,
            frame_stats: None,
            xwm: None,
            x_display: None,
            xwayland: None,
            listener: None,
            loop_signal: event_loop.get_signal(),
            loop_handle: event_loop.handle(),
            start_time: Instant::now(),
            display_handle: dh,
        })
    }

    /// Ouvre une socket `wayland-N` et y accepte les clients. Rend le nom de la socket.
    pub fn listen(&mut self, event_loop: &EventLoop<'static, State>) -> Result<OsString, Error> {
        // Same range as smithay's `ListeningSocketSource::new_auto`: wayland-0 is skipped so
        // that clients without WAYLAND_DISPLAY do not end up here by accident.
        let socket = ListeningSocket::bind_auto("wayland", 1..33)?;
        let name = socket
            .socket_name()
            .map(OsStr::to_os_string)
            .unwrap_or_default();
        tracing::info!(socket = ?name, "socket Wayland ouverte");
        self.accept_on(socket, event_loop)?;
        Ok(name)
    }

    /// Accepts clients on a socket at `path` (its lock file is `path` with the `.lock`
    /// extension). Used by the tests, which may run without `XDG_RUNTIME_DIR`.
    pub fn listen_at(
        &mut self,
        path: PathBuf,
        event_loop: &EventLoop<'static, State>,
    ) -> Result<(), Error> {
        let socket = ListeningSocket::bind_absolute(path)?;
        self.accept_on(socket, event_loop)
    }

    fn accept_on(
        &mut self,
        socket: ListeningSocket,
        event_loop: &EventLoop<'static, State>,
    ) -> Result<(), Error> {
        let source = Generic::new(socket, Interest::READ, LoopMode::Level);
        let token = event_loop
            .handle()
            .insert_source(source, |_, socket, state| {
                while let Some(stream) = socket.accept()? {
                    state.insert_client(stream);
                }
                Ok(PostAction::Continue)
            })
            .map_err(|err| err.error)?;
        self.listener = Some(token);
        Ok(())
    }

    /// Releases what the compositor leaves on disk: the Wayland socket and its lock, and
    /// XWayland with its own lock and socket. Call it when leaving the event loop.
    ///
    /// Dropping the event loop is not enough: smithay's `X11Wm` source keeps a handle to the
    /// loop, so the loop, and every source in it, is never freed. Removing a source drops it.
    pub fn shutdown(&mut self) {
        self.stop_xwayland();
        if let Some(token) = self.listener.take() {
            self.loop_handle.remove(token);
        }
    }

    /// Branche un client déjà connecté.
    pub fn insert_client(&mut self, stream: UnixStream) {
        let data = Arc::new(ClientState::default());
        if let Err(err) = self.display_handle.insert_client(stream, data) {
            tracing::warn!("client refusé : {err}");
        }
    }

    /// Fenêtre dont la surface principale est `surface`, Wayland ou X11.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
    }

    /// Fenêtre de la fenêtre X11 `surface`.
    pub fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }

    /// Fenêtre qui a le clavier.
    pub fn focused_window(&self) -> Option<Window> {
        match self.seat.get_keyboard()?.current_focus()? {
            KeyboardFocus::Wayland(surface) => self.window_for(&surface),
            KeyboardFocus::X11(surface) => self.x11_window(&surface),
        }
    }

    /// Fenêtre la plus haute, hors menus X11 : celle qui reprend le clavier.
    pub(crate) fn top_window(&self) -> Option<Window> {
        self.space
            .elements()
            .rev()
            .find(|w| !is_override_redirect(w))
            .cloned()
    }

    /// Titre d'une fenêtre, tel que le client l'a donné.
    pub fn title(window: &Window) -> Option<String> {
        if let Some(surface) = window.x11_surface() {
            return Some(surface.title());
        }
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
        match &window {
            Some(window) => self.space.raise_element(window, true),
            None => self.space.elements().for_each(|w| {
                w.set_activated(false);
            }),
        }
        let target = window.and_then(|w| match (w.x11_surface(), w.toplevel()) {
            (Some(surface), _) => {
                // L'ordre des fenêtres côté X suit le nôtre.
                if let Some(Err(err)) = self.xwm.as_mut().map(|xwm| xwm.raise_window(surface)) {
                    tracing::warn!("premier plan de la fenêtre X11 : {err}");
                }
                Some(KeyboardFocus::X11(surface.clone()))
            }
            (None, toplevel) => toplevel.map(|t| KeyboardFocus::Wayland(t.wl_surface().clone())),
        });
        self.set_keyboard_focus(target);
        for window in self.space.elements() {
            if let Some(toplevel) = window.toplevel() {
                toplevel.send_pending_configure();
            }
        }
    }

    /// Gives the keyboard to `target`, unless an anchored surface with an exclusive keyboard
    /// keeps it.
    pub(crate) fn set_keyboard_focus(&mut self, target: Option<KeyboardFocus>) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let target = self
            .exclusive_layer()
            .map(KeyboardFocus::Wayland)
            .or(target);
        keyboard.set_focus(self, target, SERIAL_COUNTER.next_serial());
    }

    /// Surface that has the keyboard.
    pub fn keyboard_focus(&self) -> Option<WlSurface> {
        let focus = self.seat.get_keyboard()?.current_focus()?;
        focus.wl_surface().map(Cow::into_owned)
    }

    /// Géométrie et opacité à dessiner pour `window` à l'instant `now`, compté depuis le
    /// démarrage, dans les coordonnées de l'espace. Fait avancer ses animations. `None` pour
    /// une fenêtre sans contenu, qui n'a rien à montrer, pas même son ombre.
    pub fn window_visual(&self, window: &Window, now: Duration) -> Option<anim::Visual> {
        let geometry = self.space.element_geometry(window)?;
        if geometry.size.is_empty() || window.wl_surface().is_none() {
            return None;
        }
        // Menu ou bulle X11 : dessiné tel quel, sans animation, comme un menu Wayland.
        if is_override_redirect(window) {
            return Some(anim::Visual {
                rect: geometry.to_f64(),
                opacity: 1.0,
                moving: false,
            });
        }
        if !window_data(window).shown.get() {
            return None;
        }
        Some(window_data(window).motion.borrow_mut().frame(now, geometry))
    }

    /// Ménage après chaque tour de boucle : fenêtres mortes, popups, envoi aux clients.
    pub fn refresh(&mut self) {
        self.space.refresh();
        for output in self.space.outputs() {
            layer_map_for_output(output).cleanup();
        }
        self.popups.cleanup();
        self.unmapped_layers.retain(|(layer, _)| layer.alive());
        if self.popup_grab.as_ref().is_some_and(|g| !g.popup.alive()) {
            self.popup_grab = None;
        }
        self.update_scales();
        if let Err(err) = self.display_handle.flush_clients() {
            tracing::warn!("envoi aux clients Wayland : {err}");
        }
    }
}

/// Fenêtre X11 qui se passe du gestionnaire de fenêtres : menu, bulle, liste déroulante.
pub(crate) fn is_override_redirect(window: &Window) -> bool {
    window
        .x11_surface()
        .is_some_and(X11Surface::is_override_redirect)
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
