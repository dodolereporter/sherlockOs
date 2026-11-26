//! Wayland compositor of the desktop, written with Smithay.
//!
//! The library holds the state and the logic, the binary picks the backend.
//! Architecture and tests: `docs/compositor.md`.

pub mod anim;
pub mod card;
mod cursor;
pub mod dmabuf;
pub mod focus;
mod handlers;
mod input;
pub mod keyboard;
mod layer;
pub mod layout;
pub mod mesa;
pub mod options;
pub mod output;
pub mod render;
mod shell;
pub mod stats;
pub mod udev;
pub mod wallpaper;
pub mod winit;
mod xwayland;

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    ffi::{OsStr, OsString},
    fmt,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use smithay::{
    desktop::{PopupManager, Space, Window, layer_map_for_output},
    input::{Seat, SeatState, keyboard::Keycode, pointer::CursorImageStatus},
    output::Output,
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

/// What keeps the compositor from starting.
#[derive(thiserror::Error)]
pub enum Error {
    #[error(
        "usage: compositor [--nested [--outputs N]] [--xwayland] [--measure] [--scale S]… \
         [--workspaces N] [--snap-gap PX] [command [arguments…]]"
    )]
    Usage,
    #[error("real session: {0}")]
    Session(String),
    #[error(
        "a graphical session already runs here (DISPLAY or WAYLAND_DISPLAY is set): the real \
         session starts from a text console or the login screen; use --nested to open the \
         compositor in a window"
    )]
    InSession,
    #[error("XWayland is off in the real session, unless --xwayland is given")]
    NoXwayland,
    #[error("cannot create the Wayland display: {0}")]
    Display(#[from] smithay::reexports::wayland_server::backend::InitError),
    #[error("cannot open the Wayland socket: {0}")]
    Socket(#[from] smithay::reexports::wayland_server::BindError),
    #[error("event loop: {0}")]
    EventLoop(#[from] smithay::reexports::calloop::Error),
    #[error("keyboard: {0}")]
    Keyboard(#[from] smithay::input::keyboard::Error),
    #[error("windowed backend: {0}")]
    Winit(#[from] smithay::backend::winit::Error),
    #[error("render programs: {0}")]
    Render(#[from] smithay::backend::renderer::gles::GlesError),
    #[error("cannot launch the command: {0}")]
    Spawn(#[from] std::io::Error),
    #[error("cannot launch Xwayland: {0}")]
    Xwayland(std::io::Error),
}

// `main` prints the error with Debug: put the readable message there.
impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// State of the compositor, shared by the event loop.
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
    /// GPU buffers (linux-dmabuf), offered once a renderer can import them.
    dmabuf: dmabuf::Dmabufs,
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
    /// Cursor requested by the client that has the pointer.
    cursor_status: CursorImageStatus,
    /// Keys of a shortcut: their release does not go to the client, which did not see the press.
    intercepted_keys: Vec<Keycode>,
    /// The compositor runs in a window of another session (`--nested`).
    nested: bool,
    /// Switches to another console (Ctrl + Alt + Fn), in a real session.
    switch_vt: Option<Box<dyn FnMut(i32)>>,
    /// Frame statistics, with `--measure`.
    pub frame_stats: Option<stats::FrameStats>,
    /// Wallpaper setting and the images decoded for it.
    pub wallpaper: wallpaper::Wallpaper,
    /// X11 window manager, once XWayland is ready.
    xwm: Option<X11Wm>,
    /// Number of the X11 display (`DISPLAY=:N`), once XWayland is ready.
    pub x_display: Option<u32>,
    /// XWayland source in the loop, to stop it.
    xwayland: Option<RegistrationToken>,
    /// Listening Wayland socket in the loop, removed by `shutdown`.
    listener: Option<RegistrationToken>,
    /// Windows, screens and workspaces for the shell programs (ADR 0021).
    shell: shell::Shell,
    /// Margin around snapped windows (`--snap-gap`, `SNAP_GAP` by default).
    snap_gap: i32,
}

/// Data attached to each window.
#[derive(Default)]
struct WindowData {
    /// The window has received its first content and has been placed. An X11 window is placed
    /// on its map request, and shown when XWayland associates its Wayland surface with it.
    shown: Cell<bool>,
    /// Geometry to return to when leaving maximized, fullscreen or a zone.
    restore: Cell<Option<Rectangle<i32, Logical>>>,
    /// Half or quarter of the screen where the window is snapped. Maximized, on the other hand,
    /// is an xdg-shell state that the client knows about.
    zone: Cell<Option<Zone>>,
    /// Resize in progress, kept until the client's last commit.
    resize: Cell<Option<Resize>>,
    /// Animated opening and geometry changes.
    motion: RefCell<anim::WindowMotion>,
    /// When the window was last given the keyboard, as a count of `State::focus` calls; 0 if
    /// never.
    focused_at: Cell<u64>,
}

fn window_data(window: &Window) -> &WindowData {
    window.user_data().get_or_insert(WindowData::default)
}

impl State {
    /// Creates the Wayland display, its protocols and the keyboard and mouse seat.
    pub fn new(event_loop: &EventLoop<'static, State>) -> Result<Self, Error> {
        let display = Display::<State>::new()?;
        let dh = display.handle();

        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(&dh, "seat0");
        // AZERTY by default, unless the environment or the system sets a layout. A layout xkb
        // does not know falls back to AZERTY: a session that does not start would be worse.
        let layout = keyboard::Layout::system();
        if let Err(err) = seat.add_keyboard(layout.xkb(), 200, 25) {
            tracing::warn!(
                ?layout,
                "keyboard layout refused ({err}): French AZERTY instead"
            );
            seat.add_keyboard(keyboard::Layout::azerty().xkb(), 200, 25)?;
        }
        seat.add_pointer();

        event_loop
            .handle()
            .insert_source(
                Generic::new(display, Interest::READ, LoopMode::Level),
                |_, display, state| {
                    // SAFETY: the display is neither replaced nor freed; we only read its clients.
                    let display = unsafe { display.get_mut() };
                    if let Err(err) = display.dispatch_clients(state) {
                        tracing::warn!("reading Wayland clients: {err}");
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|err| err.error)?;

        Ok(Self {
            compositor_state: CompositorState::new::<Self>(&dh),
            xdg_shell_state: XdgShellState::new::<Self>(&dh),
            shm_state: ShmState::new::<Self>(&dh, vec![]),
            dmabuf: dmabuf::Dmabufs::default(),
            // Clients that know it name their cursor instead of drawing one.
            _cursor_shape_state: CursorShapeManagerState::new::<Self>(&dh),
            // Only XWayland sees this protocol: it associates its X11 windows with its surfaces.
            xwayland_shell_state: XWaylandShellState::new::<Self>(&dh),
            // Anchored surfaces of the shell (ADR 0005).
            layer_shell_state: WlrLayerShellState::new::<Self>(&dh),
            // Logical place and size of each screen, which toolkits read to size their surfaces.
            _output_manager_state: OutputManagerState::new_with_xdg_output::<Self>(&dh),
            // Fractional scale of each surface's screen, and buffers cropped and scaled by the
            // client (viewporter), with which it draws at that scale.
            _fractional_scale_state: FractionalScaleManagerState::new::<Self>(&dh),
            _viewporter_state: ViewporterState::new::<Self>(&dh),
            // Clipboard (CLIPBOARD) and primary selection, which XWayland relays too.
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
            switch_vt: None,
            frame_stats: None,
            wallpaper: wallpaper::Wallpaper::default(),
            xwm: None,
            x_display: None,
            xwayland: None,
            listener: None,
            shell: shell::Shell::new(&dh),
            snap_gap: sherlock_ui::window::SNAP_GAP,
            loop_signal: event_loop.get_signal(),
            loop_handle: event_loop.handle(),
            start_time: Instant::now(),
            display_handle: dh,
        })
    }

    /// Opens a `wayland-N` socket and accepts clients on it. Returns the socket name.
    pub fn listen(&mut self, event_loop: &EventLoop<'static, State>) -> Result<OsString, Error> {
        // Same range as smithay's `ListeningSocketSource::new_auto`: wayland-0 is skipped so
        // that clients without WAYLAND_DISPLAY do not end up here by accident.
        let socket = ListeningSocket::bind_auto("wayland", 1..33)?;
        let name = socket
            .socket_name()
            .map(OsStr::to_os_string)
            .unwrap_or_default();
        tracing::info!(socket = ?name, "Wayland socket open");
        self.accept_on(socket, event_loop)?;
        if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            self.listen_shell_next_to(&PathBuf::from(dir).join(&name), event_loop);
        }
        Ok(name)
    }

    /// Accepts clients on a socket at `path` (its lock file is `path` with the `.lock`
    /// extension). Used by the tests, which may run without `XDG_RUNTIME_DIR`.
    pub fn listen_at(
        &mut self,
        path: PathBuf,
        event_loop: &EventLoop<'static, State>,
    ) -> Result<(), Error> {
        let socket = ListeningSocket::bind_absolute(path.clone())?;
        self.accept_on(socket, event_loop)?;
        self.listen_shell_next_to(&path, event_loop);
        Ok(())
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
        self.stop_shell_socket();
        if let Some(token) = self.listener.take() {
            self.loop_handle.remove(token);
        }
    }

    /// Plugs in an already connected client.
    pub fn insert_client(&mut self, stream: UnixStream) {
        let data = Arc::new(ClientState::default());
        if let Err(err) = self.display_handle.insert_client(stream, data) {
            tracing::warn!("client refused: {err}");
        }
    }

    /// Window whose main surface is `surface`, Wayland or X11.
    pub fn window_for(&self, surface: &WlSurface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.wl_surface().as_deref() == Some(surface))
            .cloned()
    }

    /// Window of the X11 window `surface`.
    pub fn x11_window(&self, surface: &X11Surface) -> Option<Window> {
        self.space
            .elements()
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }

    /// Window that has the keyboard.
    pub fn focused_window(&self) -> Option<Window> {
        match self.seat.get_keyboard()?.current_focus()? {
            KeyboardFocus::Wayland(surface) => self.window_for(&surface),
            KeyboardFocus::X11(surface) => self.x11_window(&surface),
        }
    }

    /// The shown window that last had the keyboard, X11 menus aside: the one that takes the
    /// keyboard back. Among windows that never had it, the topmost one.
    pub(crate) fn top_window(&self) -> Option<Window> {
        self.space
            .elements()
            .filter(|w| !is_override_redirect(w) && self.is_visible(w))
            .max_by_key(|w| window_data(w).focused_at.get())
            .cloned()
    }

    /// Title of a window, as the client gave it.
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

    /// Gives the keyboard to a window and raises it, or takes the keyboard away from everyone.
    pub fn focus(&mut self, window: Option<Window>) {
        static FOCUS_COUNT: AtomicU64 = AtomicU64::new(1);
        match &window {
            Some(window) => {
                let count = FOCUS_COUNT.fetch_add(1, Ordering::Relaxed);
                window_data(window).focused_at.set(count);
                self.space.raise_element(window, true);
            }
            None => self.space.elements().for_each(|w| {
                w.set_activated(false);
            }),
        }
        let target = window.and_then(|w| match (w.x11_surface(), w.toplevel()) {
            (Some(surface), _) => {
                // The window order on the X side follows ours.
                if let Some(Err(err)) = self.xwm.as_mut().map(|xwm| xwm.raise_window(surface)) {
                    tracing::warn!("raising the X11 window: {err}");
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

    /// Geometry and opacity to draw for `window` at time `now`, counted from startup, in space
    /// coordinates. Advances its animations. `None` for a window without content, which has
    /// nothing to show, not even its shadow.
    pub fn window_visual(&self, window: &Window, now: Duration) -> Option<anim::Visual> {
        let geometry = self.space.element_geometry(window)?;
        if geometry.size.is_empty() || window.wl_surface().is_none() {
            return None;
        }
        // X11 menu or tooltip: drawn as is, without animation, like a Wayland menu.
        if is_override_redirect(window) {
            return Some(anim::Visual {
                rect: geometry.to_f64(),
                opacity: 1.0,
                moving: false,
                decoration: 0.0,
            });
        }
        if !window_data(window).shown.get() {
            return None;
        }
        let arrangement = handlers::Arrangement::of(window);
        let mut motion = window_data(window).motion.borrow_mut();
        motion.set_bare(arrangement.maximized || arrangement.fullscreen);
        Some(motion.frame(now, geometry))
    }

    /// Margin around windows snapped into a half or a quarter of a screen.
    pub fn snap_gap(&self) -> i32 {
        self.snap_gap
    }

    /// Changes the margin around snapped windows; those already snapped take it at once.
    pub fn set_snap_gap(&mut self, gap: i32) {
        self.snap_gap = gap.max(0);
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for output in &outputs {
            self.reflow(output);
        }
    }

    /// Frame callbacks of `window`, drawn on `output`, at `time` from startup. A window out
    /// of view (minimized, or on another workspace) gets one per `HIDDEN_FRAME_INTERVAL` only:
    /// it saves the battery, and a client that waits for its callback before drawing again
    /// (Mesa's EGL in `eglSwapBuffers`) is slowed down, never blocked.
    pub fn send_window_frame(&self, window: &Window, output: &Output, time: Duration) {
        if self.workspace_shift(window, time).is_some() {
            window.send_frame(output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        } else {
            window.send_frame(output, time, Some(HIDDEN_FRAME_INTERVAL), |_, _| None);
        }
    }

    /// Cleanup after each loop iteration: dead windows, popups, flush to the clients.
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
        self.refresh_shell();
        if let Err(err) = self.display_handle.flush_clients() {
            tracing::warn!("flushing Wayland clients: {err}");
        }
    }
}

/// Pace of the frame callbacks of a window out of view.
pub const HIDDEN_FRAME_INTERVAL: Duration = Duration::from_secs(1);

/// X11 window that bypasses the window manager: menu, tooltip, drop-down list.
pub(crate) fn is_override_redirect(window: &Window) -> bool {
    window
        .x11_surface()
        .is_some_and(X11Surface::is_override_redirect)
}

/// Data of each connected client.
#[derive(Default)]
pub struct ClientState {
    compositor_state: CompositorClientState,
    /// Connected through the shell socket: sees the globals of the shell (ADR 0021).
    shell: bool,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
