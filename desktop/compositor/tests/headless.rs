//! The compositor starts without a screen or backend, a test client connects, creates a window
//! and receives its configures. Everything runs in a single thread: the compositor loop and the
//! client reads alternate, which lets the tests look at the state on both sides.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::net::UnixStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use std::io::Write as _;

    use compositor::State;
    use sherlock_ui::color::{Color, Palette};
    use smithay::reexports::wayland_server::Resource as _;
    use smithay::{
        backend::input::{ButtonState, KeyState},
        input::{keyboard::Keycode, pointer::CursorIcon},
        reexports::calloop::EventLoop,
        utils::{Logical, Point, Rectangle},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy as _, QueueHandle, WEnum, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_callback::{self, WlCallback},
            wl_compositor::WlCompositor,
            wl_keyboard::{self, WlKeyboard},
            wl_pointer::{self, WlPointer},
            wl_registry,
            wl_seat::WlSeat,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::wp::fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{self, WpFractionalScaleV1},
    };
    use wayland_protocols::xdg::decoration::zv1::client::{
        zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        zxdg_toplevel_decoration_v1::{self, Mode as DecorationMode, ZxdgToplevelDecorationV1},
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::{self, ResizeEdge, State as XdgState, XdgToplevel},
        xdg_wm_base::{self, XdgWmBase},
    };
    use wayland_protocols_misc::server_decoration::client::{
        org_kde_kwin_server_decoration::{self as kde, OrgKdeKwinServerDecoration},
        org_kde_kwin_server_decoration_manager::{
            self as kde_manager, OrgKdeKwinServerDecorationManager,
        },
    };
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
    };

    /// What the client has received.
    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        /// Each configure of the window: width, height, states.
        configures: Vec<(i32, i32, Vec<XdgState>)>,
        /// Serial of the last button press received.
        button_serial: Option<u32>,
        /// Keys received: evdev code, pressed or released.
        keys: Vec<(u32, bool)>,
        /// The compositor has answered the last `wl_display.sync`.
        synced: bool,
        /// Each configure of the anchored surface: width, height.
        layer_configures: Vec<(u32, u32)>,
        /// Last scale announced by `wp_fractional_scale_v1`, in 120ths.
        preferred_scale: Option<u32>,
        /// Anchored surfaces closed by the compositor.
        layer_closed: usize,
        /// Each decoration mode xdg-decoration gave.
        decorations: Vec<DecorationMode>,
        /// Default mode of KDE's protocol, then each mode it gave, as numbers.
        kde_default: Option<u32>,
        kde_modes: Vec<u32>,
        /// The compositor asked the window to close.
        closed: bool,
    }

    impl Dispatch<ZxdgToplevelDecorationV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZxdgToplevelDecorationV1,
            event: zxdg_toplevel_decoration_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zxdg_toplevel_decoration_v1::Event::Configure {
                mode: WEnum::Value(mode),
            } = event
            {
                client.decorations.push(mode);
            }
        }
    }

    impl Dispatch<OrgKdeKwinServerDecorationManager, ()> for Client {
        fn event(
            client: &mut Self,
            _: &OrgKdeKwinServerDecorationManager,
            event: kde_manager::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let kde_manager::Event::DefaultMode { mode } = event {
                client.kde_default = Some(mode.into());
            }
        }
    }

    impl Dispatch<OrgKdeKwinServerDecoration, ()> for Client {
        fn event(
            client: &mut Self,
            _: &OrgKdeKwinServerDecoration,
            event: kde::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let kde::Event::Mode { mode } = event {
                client.kde_modes.push(mode.into());
            }
        }
    }

    impl Dispatch<ZwlrLayerSurfaceV1, ()> for Client {
        fn event(
            client: &mut Self,
            surface: &ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } = event
            {
                surface.ack_configure(serial);
                client.layer_configures.push((width, height));
            }
            if let zwlr_layer_surface_v1::Event::Closed = event {
                client.layer_closed += 1;
            }
        }
    }

    impl Dispatch<WpFractionalScaleV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WpFractionalScaleV1,
            event: wp_fractional_scale_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wp_fractional_scale_v1::Event::PreferredScale { scale } = event {
                client.preferred_scale = Some(scale);
            }
        }
    }

    impl Dispatch<WlPointer, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WlPointer,
            event: wl_pointer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_pointer::Event::Button { serial, .. } = event {
                client.button_serial = Some(serial);
            }
        }
    }

    impl Dispatch<WlKeyboard, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WlKeyboard,
            event: wl_keyboard::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_keyboard::Event::Key { key, state, .. } = event {
                let pressed = state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                client.keys.push((key, pressed));
            }
        }
    }

    impl Dispatch<WlCallback, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WlCallback,
            event: wl_callback::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_callback::Event::Done { .. } = event {
                client.synced = true;
            }
        }
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Client {
        fn event(
            client: &mut Self,
            _: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                client.globals.push((name, interface, version));
            }
        }
    }

    impl Dispatch<XdgWmBase, ()> for Client {
        fn event(
            _: &mut Self,
            base: &XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                base.pong(serial);
            }
        }
    }

    impl Dispatch<XdgSurface, ()> for Client {
        fn event(
            _: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
            }
        }
    }

    impl Dispatch<XdgToplevel, ()> for Client {
        fn event(
            client: &mut Self,
            _: &XdgToplevel,
            event: xdg_toplevel::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_toplevel::Event::Close = event {
                client.closed = true;
            } else if let xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } = event
            {
                let states = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter_map(|b| XdgState::try_from(u32::from_ne_bytes(*b)).ok())
                    .collect();
                client.configures.push((width, height, states));
            }
        }
    }

    delegate_noop!(Client: ZxdgDecorationManagerV1);

    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlSeat);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);
    delegate_noop!(Client: ZwlrLayerShellV1);
    delegate_noop!(Client: WpFractionalScaleManagerV1);

    /// Shared memory for a buffer of `size` bytes, without a file left on disk.
    /// Tests run in parallel: each one has its own file name.
    fn shm_file(size: u64) -> File {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("compositor-test-{}-{n}", std::process::id());
        let path = std::env::temp_dir().join(name);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        file.set_len(size).unwrap();
        file
    }

    const BTN_LEFT: u32 = 0x110;
    const BTN_RIGHT: u32 = 0x111;
    // xkb codes (evdev + 8) of left Super and of the arrows. Without `--nested`, the gesture
    // key is Super.
    const KEY_SUPER: u32 = 133;
    const KEY_UP: u32 = 111;
    const KEY_LEFT: u32 = 113;
    const KEY_DOWN: u32 = 116;
    const KEY_SHIFT: u32 = 50;

    /// Window shown by `Harness::show_window`: 100 × 80, centered at (350, 260) in the 800 × 600
    /// screen. The buffer file lives as long as it does.
    struct Shown {
        seat: WlSeat,
        toplevel: XdgToplevel,
        surface: WlSurface,
        shm: WlShm,
        _file: File,
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
    }

    impl Harness {
        fn start() -> Self {
            Self::start_scaled(1.0)
        }

        /// One screen of 800 × 600 pixels at `scale`.
        fn start_scaled(scale: f64) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), scale);
            let (server_side, client_side) = UnixStream::pair().unwrap();
            state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            Self {
                event_loop,
                state,
                conn,
                queue,
                client: Client::default(),
            }
        }

        /// Runs compositor and client until `done` is true.
        fn run_until(&mut self, what: &str, done: impl Fn(&Client, &State) -> bool) {
            for _ in 0..200 {
                self.conn.flush().unwrap();
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if let Some(guard) = self.queue.prepare_read() {
                    // Nothing to read is not an error: try again on the next turn.
                    let _ = guard.read();
                }
                self.queue.dispatch_pending(&mut self.client).unwrap();
                if done(&self.client, &self.state) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        fn bind<I: wayland_client::Proxy + 'static>(&self, registry: &wl_registry::WlRegistry) -> I
        where
            Client: Dispatch<I, ()>,
        {
            let name = I::interface().name;
            let (id, _, version) = self.client.globals.iter().find(|g| g.1 == name).unwrap();
            registry.bind(
                *id,
                (*version).min(I::interface().version),
                &self.queue.handle(),
                (),
            )
        }

        fn last_configure(&self) -> &(i32, i32, Vec<XdgState>) {
            self.client.configures.last().unwrap()
        }

        fn show_window(&mut self) -> Shown {
            self.show_window_with(false)
        }

        /// A client creates a window, shows it with a 100 × 80 buffer and listens to the pointer
        /// and the keyboard. `maximized`: it asks to be maximized before its first content.
        fn show_window_with(&mut self, maximized: bool) -> Shown {
            let qh = self.queue.handle();
            let registry = self.conn.display().get_registry(&qh, ());
            self.run_until("the globals", |c, _| {
                c.globals.iter().any(|g| g.1 == "wl_shm")
            });
            let compositor: WlCompositor = self.bind(&registry);
            let wm_base: XdgWmBase = self.bind(&registry);
            let shm: WlShm = self.bind(&registry);
            let seat: WlSeat = self.bind(&registry);
            let _pointer = seat.get_pointer(&qh, ());
            let _keyboard = seat.get_keyboard(&qh, ());
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            if maximized {
                toplevel.set_maximized();
            }
            surface.commit();
            self.run_until("the first configure", |c, _| !c.configures.is_empty());
            let file = shm_file(100 * 80 * 4);
            let pool = shm.create_pool(file.as_fd(), 100 * 80 * 4, &qh, ());
            let buffer = pool.create_buffer(0, 100, 80, 400, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            let placed = if maximized { (0, 0) } else { (350, 260) };
            self.run_until("the placement", |_, s| location(s) == Some(placed.into()));
            Shown {
                seat,
                toplevel,
                surface,
                shm,
                _file: file,
            }
        }

        /// A client creates a window of 400 × 300 and asks the compositor for its decoration
        /// through xdg-decoration before showing it. The whole window, its bar of 42 included,
        /// is centered: at (200, 129), the content at (200, 171).
        fn show_decorated(&mut self) -> (Shown, ZxdgToplevelDecorationV1) {
            self.show_decorated_in(Palette::DARK.window)
        }

        /// `show_decorated`, the content filled with `fill`.
        fn show_decorated_in(&mut self, fill: Color) -> (Shown, ZxdgToplevelDecorationV1) {
            let qh = self.queue.handle();
            let registry = self.conn.display().get_registry(&qh, ());
            self.run_until("the globals", |c, _| {
                c.globals
                    .iter()
                    .any(|g| g.1 == "zxdg_decoration_manager_v1")
            });
            let compositor: WlCompositor = self.bind(&registry);
            let wm_base: XdgWmBase = self.bind(&registry);
            let manager: ZxdgDecorationManagerV1 = self.bind(&registry);
            let shm: WlShm = self.bind(&registry);
            let seat: WlSeat = self.bind(&registry);
            let _pointer = seat.get_pointer(&qh, ());
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title("Konsole".into());
            let decoration = manager.get_toplevel_decoration(&toplevel, &qh, ());
            surface.commit();
            self.run_until("the first configure", |c, _| !c.configures.is_empty());
            let mut file = shm_file(400 * 300 * 4);
            // Argb8888 is little-endian: blue, green, red, alpha.
            let pixel = [fill.b, fill.g, fill.r, 255];
            file.write_all(&pixel.repeat(400 * 300)).unwrap();
            let pool = shm.create_pool(file.as_fd(), 400 * 300 * 4, &qh, ());
            let buffer = pool.create_buffer(0, 400, 300, 1600, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self.run_until("the placement", |_, s| {
                location(s) == Some((200, 171).into())
            });
            let shown = Shown {
                seat,
                toplevel,
                surface,
                shm,
                _file: file,
            };
            (shown, decoration)
        }

        /// A click of the left button at (`x`, `y`), at `time` ms.
        fn click(&mut self, x: f64, y: f64, time: u32) {
            self.state.pointer_motion((x, y).into(), time);
            self.state
                .pointer_button(BTN_LEFT, ButtonState::Pressed, time);
            self.state
                .pointer_button(BTN_LEFT, ButtonState::Released, time + 1);
        }

        /// The client redraws its window at `w` × `h` and commits it. The buffer file must live
        /// as long as the buffer.
        fn redraw(&mut self, shown: &Shown, w: i32, h: i32) -> File {
            let qh = self.queue.handle();
            let file = shm_file((w * h * 4) as u64);
            let pool = shown.shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            shown.surface.attach(Some(&buffer), 0, 0);
            shown.surface.commit();
            self.run_until("the new content", |_, s| {
                s.space
                    .elements()
                    .next()
                    .is_some_and(|w_| w_.geometry().size == (w, h).into())
            });
            file
        }

        /// What is drawn of the window at `t` ms, and its real geometry.
        fn drawn(&self, t: u64) -> (Rectangle<f64, Logical>, Rectangle<f64, Logical>) {
            let window = self.state.space.elements().next().unwrap();
            let visual = self
                .state
                .window_visual(window, Duration::from_millis(t))
                .unwrap();
            let real = self.state.space.element_geometry(window).unwrap();
            (visual.rect, real.to_f64())
        }

        /// Draws the window at `t` ms and checks that it is drawn exactly at its place.
        fn assert_follows(&self, t: u64, what: &str) {
            let (drawn, real) = self.drawn(t);
            assert_eq!(drawn, real, "{what}: drawn away from its geometry");
        }

        /// Left button press at (`x`, `y`), on a surface of the client: returns its serial.
        fn press(&mut self, x: f64, y: f64) -> u32 {
            self.client.button_serial = None;
            self.state.pointer_motion((x, y).into(), 1);
            self.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
            self.run_until("the press", |c, _| c.button_serial.is_some());
            self.client.button_serial.unwrap()
        }

        fn release(&mut self) {
            self.state
                .pointer_button(BTN_LEFT, ButtonState::Released, 3);
        }

        /// Waits for a configure of size `w` × `h` and returns its states.
        fn wait_size(&mut self, what: &str, w: i32, h: i32) -> Vec<XdgState> {
            self.run_until(what, |c, _| {
                c.configures.last().is_some_and(|c| (c.0, c.1) == (w, h))
            });
            self.last_configure().2.clone()
        }

        /// Gesture key and one key, pressed then released.
        fn shortcut(&mut self, key: u32) {
            self.chord(&[KEY_SUPER], key);
        }

        /// The `mods` keys held during the press and release of `key`.
        fn chord(&mut self, mods: &[u32], key: u32) {
            let press = mods.iter().chain([&key]).map(|k| (*k, KeyState::Pressed));
            let release = [&key]
                .into_iter()
                .chain(mods)
                .map(|k| (*k, KeyState::Released));
            for (code, state) in press.chain(release).collect::<Vec<_>>() {
                self.state.keyboard_key(Keycode::new(code), state, 4);
            }
        }
    }

    fn location(s: &State) -> Option<Point<i32, Logical>> {
        let window = s.space.elements().next()?;
        s.space.element_location(window)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_client_creates_a_window_and_receives_its_configure() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "xdg_wm_base")
        });

        let compositor: WlCompositor = h.bind(&registry);
        let wm_base: XdgWmBase = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        toplevel.set_title("test".into());
        surface.commit();

        h.run_until("the first configure", |c, _| !c.configures.is_empty());
        assert_eq!(
            h.last_configure(),
            &(0, 0, vec![]),
            "the client chooses its size"
        );
        let windows: Vec<_> = h.state.space.elements().collect();
        assert_eq!(windows.len(), 1);
        assert_eq!(State::title(windows[0]).as_deref(), Some("test"));

        toplevel.set_maximized();
        h.run_until("maximizing", |c, _| c.configures.len() >= 2);
        let (w, h_, states) = h.last_configure().clone();
        assert_eq!(
            (w, h_),
            (800, 600),
            "maximized, the window takes the whole screen"
        );
        assert!(states.contains(&XdgState::Maximized));

        toplevel.set_fullscreen(None);
        h.run_until("fullscreen", |c, _| c.configures.len() >= 3);
        assert!(h.last_configure().2.contains(&XdgState::Fullscreen));

        toplevel.unset_fullscreen();
        toplevel.unset_maximized();
        h.run_until("back to normal", |c, _| {
            c.configures.last().is_some_and(|(_, _, s)| {
                !s.contains(&XdgState::Maximized) && !s.contains(&XdgState::Fullscreen)
            })
        });

        toplevel.destroy();
        h.run_until("the window closing", |_, s| s.space.elements().count() == 0);
    }

    #[test]
    fn a_client_moves_its_window_from_its_title_bar() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let Shown { seat, toplevel, .. } = h.show_window();

        // Press on the title bar, which the client draws itself.
        let serial = h.press(360.0, 270.0);

        // A serial that does not come from this press is refused.
        toplevel._move(&seat, serial + 1000);
        h.conn.display().sync(&qh, ());
        h.run_until("the sync answer", |c, _| c.synced);
        assert!(!h.state.gesture_active());

        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Move));
        h.state.pointer_motion((410.0, 300.0).into(), 3);
        assert_eq!(
            location(&h.state),
            Some((400, 290).into()),
            "the window follows the pointer"
        );
        assert_eq!(
            h.state.snap_preview(),
            None,
            "far from the edges, nothing to snap"
        );
        h.release();
        assert!(!h.state.gesture_active());
        h.state.pointer_motion((500.0, 400.0).into(), 5);
        assert_eq!(
            location(&h.state),
            Some((400, 290).into()),
            "the gesture is over"
        );
    }

    #[test]
    fn a_window_dropped_at_the_top_is_maximized_then_restored_by_moving_it() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();

        let serial = h.press(360.0, 270.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        h.state.pointer_motion((400.0, 2.0).into(), 3);
        assert_eq!(
            h.state.snap_preview(),
            Some(rect(0, 0, 800, 600)),
            "the preview shows the whole screen"
        );
        h.release();
        let states = h.wait_size("maximizing", 800, 600);
        assert!(states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((0, 0).into()));

        // When moved, it leaves maximized and gets its previous size back.
        let serial = h.press(50.0, 10.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        // Without motion, nothing changes: a mere click does not pull the window out.
        h.state.pointer_motion((51.0, 11.0).into(), 3);
        assert_eq!(h.last_configure().0, 800);
        h.state.pointer_motion((300.0, 300.0).into(), 3);
        let states = h.wait_size("the previous size", 100, 80);
        assert!(!states.contains(&XdgState::Maximized));
        h.release();
        assert!(!h.state.gesture_active());
    }

    #[test]
    fn a_window_dropped_in_a_corner_or_on_a_side_snaps() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();

        let serial = h.press(360.0, 270.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        h.state.pointer_motion((795.0, 590.0).into(), 3);
        // 8 from the edges of the screen, 8 from the window of the next quarter.
        assert_eq!(h.state.snap_preview(), Some(rect(404, 304, 388, 288)));
        h.release();
        let states = h.wait_size("the bottom right quarter", 388, 288);
        assert!(!states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((404, 304).into()));

        // The content stays 100 × 80 as long as the client has not redrawn it.
        let serial = h.press(414.0, 314.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        h.state.pointer_motion((1.0, 300.0).into(), 3);
        assert_eq!(h.state.snap_preview(), Some(rect(8, 8, 388, 584)));
        h.release();
        h.wait_size("the left half", 388, 584);

        // Restored, it returns to its place from before the drag, not the one where it was dropped.
        h.shortcut(KEY_DOWN);
        h.wait_size("the previous size", 100, 80);
        assert_eq!(location(&h.state), Some((404, 304).into()));
    }

    #[test]
    fn the_band_around_the_window_resizes_it() {
        let mut h = Harness::start();
        let _shown = h.show_window();

        // 5 pixels to the right of the window, which spans x = 350 to x = 450.
        h.state.pointer_motion((455.0, 300.0).into(), 1);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::EResize));
        h.state.pointer_motion((455.0, 345.0).into(), 1);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::SeResize));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        assert!(h.state.gesture_active());
        h.state.pointer_motion((485.0, 365.0).into(), 3);
        let states = h.wait_size("the resize", 130, 100);
        assert!(states.contains(&XdgState::Resizing));
        // Another button does not end the gesture: only the release of the left one does.
        h.state.pointer_button(BTN_RIGHT, ButtonState::Pressed, 3);
        h.state.pointer_button(BTN_RIGHT, ButtonState::Released, 3);
        assert!(h.state.gesture_active());
        h.release();
        assert!(!h.state.gesture_active());
        h.run_until("the resize end", |c, _| {
            c.configures
                .last()
                .is_some_and(|c| !c.2.contains(&XdgState::Resizing))
        });
    }

    #[test]
    fn a_client_resizes_its_window_by_an_edge() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();

        // In the bottom right corner, inside: where the client draws its edge.
        let serial = h.press(448.0, 338.0);
        toplevel.resize(&seat, serial, ResizeEdge::BottomRight);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::SeResize));
        h.state.pointer_motion((478.0, 358.0).into(), 3);
        let states = h.wait_size("the resize", 130, 100);
        assert!(states.contains(&XdgState::Resizing));
        h.release();
        assert!(!h.state.gesture_active());
    }

    #[test]
    fn a_fullscreen_window_does_not_move() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let Shown { seat, toplevel, .. } = h.show_window();
        toplevel.set_fullscreen(None);
        let states = h.wait_size("fullscreen", 800, 600);
        assert!(states.contains(&XdgState::Fullscreen));

        // The content stays 100 × 80, placed at (0, 0).
        let serial = h.press(50.0, 40.0);
        toplevel._move(&seat, serial);
        toplevel.resize(&seat, serial, ResizeEdge::BottomRight);
        h.conn.display().sync(&qh, ());
        h.run_until("the sync answer", |c, _| c.synced);
        assert!(!h.state.gesture_active());
        // No grab band either.
        h.release();
        h.state.pointer_motion((103.0, 40.0).into(), 3);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Default));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 4);
        assert!(!h.state.gesture_active());
    }

    #[test]
    fn the_gesture_key_and_the_arrows_snap_the_window() {
        let mut h = Harness::start();
        let _shown = h.show_window();

        h.shortcut(KEY_LEFT);
        let states = h.wait_size("the left half", 388, 584);
        assert!(!states.contains(&XdgState::Maximized));
        h.shortcut(KEY_UP);
        let states = h.wait_size("maximizing", 800, 600);
        assert!(states.contains(&XdgState::Maximized));
        h.shortcut(KEY_DOWN);
        let states = h.wait_size("the previous size", 100, 80);
        assert!(!states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((350, 260).into()));
        h.shortcut(KEY_UP);
        h.wait_size("maximizing", 800, 600);
        h.shortcut(KEY_UP);
        h.wait_size("the restore", 100, 80);

        // The client sees Super, never the intercepted arrows, neither their press nor their
        // release.
        h.conn.display().sync(&h.queue.handle(), ());
        h.run_until("the sync answer", |c, _| c.synced);
        let evdev = |xkb: u32| xkb - 8;
        assert!(h.client.keys.contains(&(evdev(KEY_SUPER), true)));
        let arrows = [KEY_UP, KEY_LEFT, KEY_DOWN].map(evdev);
        assert!(!h.client.keys.iter().any(|(k, _)| arrows.contains(k)));

        // With another key on top of the gesture key, the arrow goes to the client.
        h.chord(&[KEY_SUPER, KEY_SHIFT], KEY_LEFT);
        h.run_until("the arrow", |c, _| {
            c.keys.contains(&(evdev(KEY_LEFT), false))
        });
        assert!(h.client.keys.contains(&(evdev(KEY_LEFT), true)));
        assert_eq!(h.last_configure().0, 100, "the window is not snapped");
    }

    #[test]
    fn a_window_maximized_before_its_content_moves_at_the_client_size() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window_with(true);
        let states = h.wait_size("maximizing", 800, 600);
        assert!(states.contains(&XdgState::Maximized));

        // Its previous place is empty: the client chooses its size, 0 × 0.
        let serial = h.press(50.0, 10.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        h.state.pointer_motion((300.0, 300.0).into(), 3);
        let states = h.wait_size("the client size", 0, 0);
        assert!(!states.contains(&XdgState::Maximized));

        // Snapped then restored, it does not keep the screen size as its previous size.
        h.state.pointer_motion((1.0, 300.0).into(), 4);
        h.release();
        h.wait_size("the left half", 388, 584);
        h.shortcut(KEY_DOWN);
        let states = h.wait_size("the client size", 0, 0);
        assert!(!states.contains(&XdgState::Maximized));
    }

    #[test]
    fn an_interrupted_snap_does_not_make_the_next_drag_animated() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();
        h.drawn(0);
        h.assert_follows(1000, "opened");

        // Maximized, then restored before the end of the animation.
        h.shortcut(KEY_UP);
        let (moving, real) = h.drawn(1010);
        assert_ne!(moving, real, "the snap is animated");
        h.shortcut(KEY_DOWN);
        h.drawn(1050);
        h.assert_follows(3000, "restored");

        // The next drag follows the pointer, on every frame.
        let serial = h.press(360.0, 270.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        h.state.pointer_motion((330.0, 280.0).into(), 3);
        h.assert_follows(3010, "dragged");
        h.state.pointer_motion((300.0, 290.0).into(), 3);
        h.assert_follows(3020, "dragged further");
        h.release();
    }

    #[test]
    fn a_snap_without_effect_does_not_make_the_next_resize_animated() {
        let mut h = Harness::start();
        let shown = h.show_window();
        h.drawn(0);
        h.shortcut(KEY_LEFT);
        h.wait_size("the left half", 388, 584);
        h.drawn(1000);
        let _left = h.redraw(&shown, 388, 584);
        h.assert_follows(3000, "snapped");

        // Already on the left: the snap changes nothing.
        h.shortcut(KEY_LEFT);
        h.assert_follows(3010, "snapped again");

        // Resized by the right band, it is drawn at its new size.
        h.state.pointer_motion((400.0, 40.0).into(), 1);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        assert!(h.state.gesture_active());
        h.state.pointer_motion((430.0, 40.0).into(), 3);
        h.wait_size("the resize", 418, 584);
        let _file = h.redraw(&shown, 418, 584);
        h.assert_follows(3020, "resized");
        assert_eq!(
            h.drawn(3030).0,
            Rectangle::new((8.0, 8.0).into(), (418.0, 584.0).into())
        );
        h.release();
    }

    #[test]
    fn a_snapped_window_pulled_out_follows_the_pointer() {
        let mut h = Harness::start();
        let shown = h.show_window();
        h.drawn(0);
        h.shortcut(KEY_LEFT);
        h.wait_size("the left half", 388, 584);
        h.drawn(1000);
        let _left = h.redraw(&shown, 388, 584);
        h.assert_follows(3000, "snapped");
        let Shown { seat, toplevel, .. } = shown;

        // Pulled out by dragging it, then dragged fast: it stays under the pointer.
        let serial = h.press(10.0, 10.0);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        for (i, x) in [100.0, 200.0, 300.0, 400.0].into_iter().enumerate() {
            h.state.pointer_motion((x, 300.0).into(), 3);
            h.assert_follows(3001 + i as u64, "pulled out");
        }
        h.release();
    }

    /// What the client asks for an anchored surface.
    #[derive(Clone, Copy)]
    struct Spec {
        layer: Layer,
        anchor: Anchor,
        size: (u32, u32),
        zone: i32,
        /// Top, right, bottom, left.
        margin: (i32, i32, i32, i32),
        keyboard: KeyboardInteractivity,
    }

    /// The bar: anchored at the top and both sides, 36 high with margins of 10 at the top and
    /// 12 on the sides, an exclusive zone of 36, the keyboard on demand.
    const BAR: Spec = Spec {
        layer: Layer::Top,
        anchor: Anchor::Top.union(Anchor::Left).union(Anchor::Right),
        size: (0, 36),
        zone: 36,
        margin: (10, 12, 0, 12),
        keyboard: KeyboardInteractivity::OnDemand,
    };

    /// An anchored surface of the client, with what it needs to draw again.
    struct Anchored {
        surface: WlSurface,
        layer: ZwlrLayerSurfaceV1,
        shm: WlShm,
        _file: Option<File>,
    }

    impl Anchored {
        /// Draws at the size of the last configure.
        fn draw(&mut self, h: &mut Harness) {
            let qh = h.queue.handle();
            let (w, ht) = *h.client.layer_configures.last().unwrap();
            let (w, ht) = (w as i32, ht as i32);
            let file = shm_file((w * ht * 4) as u64);
            let pool = self.shm.create_pool(file.as_fd(), w * ht * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, ht, w * 4, Format::Argb8888, &qh, ());
            self.surface.attach(Some(&buffer), 0, 0);
            self.surface.commit();
            self._file = Some(file);
            settle(h);
        }
    }

    /// A few turns of the loop, for everything sent to be handled.
    fn settle(h: &mut Harness) {
        for _ in 0..5 {
            h.run_until("one turn", |_, _| true);
        }
    }

    /// The client creates an anchored surface and commits it. Once configured, it draws it;
    /// a surface closed by the compositor stays without content.
    fn anchor(h: &mut Harness, spec: Spec) -> Anchored {
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "zwlr_layer_shell_v1")
        });
        let compositor: WlCompositor = h.bind(&registry);
        let layer_shell: ZwlrLayerShellV1 = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let layer =
            layer_shell.get_layer_surface(&surface, None, spec.layer, "test".into(), &qh, ());
        layer.set_size(spec.size.0, spec.size.1);
        layer.set_anchor(spec.anchor);
        layer.set_exclusive_zone(spec.zone);
        let (top, right, bottom, left) = spec.margin;
        layer.set_margin(top, right, bottom, left);
        layer.set_keyboard_interactivity(spec.keyboard);
        surface.commit();
        let (configured, closed) = (h.client.layer_configures.len(), h.client.layer_closed);
        h.run_until("the answer to the anchored surface", |c, _| {
            c.layer_configures.len() > configured || c.layer_closed > closed
        });
        let mut anchored = Anchored {
            surface,
            layer,
            shm: h.bind(&registry),
            _file: None,
        };
        if h.client.layer_closed == closed {
            anchored.draw(h);
        }
        anchored
    }

    /// The client creates the bar and draws it. Returns its surface.
    fn show_bar(h: &mut Harness) -> Anchored {
        let bar = anchor(h, BAR);
        assert_eq!(
            h.client.layer_configures,
            vec![(776, 36)],
            "the whole width, minus the margins"
        );
        h.run_until("the exclusive zone", |_, s| {
            s.output_area() != Some(rect(0, 0, 800, 600))
        });
        bar
    }

    #[test]
    fn a_bar_anchored_at_the_top_reduces_the_maximize_area() {
        let mut h = Harness::start();
        h.show_window();
        h.shortcut(KEY_UP);
        let states = h.wait_size("maximizing", 800, 600);
        assert!(states.contains(&XdgState::Maximized));

        let bar = show_bar(&mut h);
        // 10 of margin and 36 of bar at the top.
        assert_eq!(h.state.output_area(), Some(rect(0, 46, 800, 554)));
        let states = h.wait_size("maximized below the bar", 800, 554);
        assert!(states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((0, 46).into()));

        // Back to its size, then snapped to the left: the bar stays uncovered.
        h.shortcut(KEY_DOWN);
        h.wait_size("the previous size", 100, 80);
        h.shortcut(KEY_LEFT);
        h.wait_size("the left half", 388, 538);
        assert_eq!(location(&h.state), Some((8, 54).into()));

        // A click on the bar gives it the keyboard: it asked for it on demand.
        h.press(100.0, 20.0);
        h.release();
        let focus = h.state.keyboard_focus().expect("the keyboard on the bar");
        assert_eq!(focus.id().protocol_id(), bar.surface.id().protocol_id());
    }

    #[test]
    fn with_two_screens_a_window_snaps_on_the_second() {
        let mut h = Harness::start();
        h.state.add_output((800, 600).into(), 1.0);
        h.show_window();
        // The pointer on the second screen: its work area receives new windows.
        h.state.pointer_motion((1200.0, 300.0).into(), 1);
        assert_eq!(h.state.output_area(), Some(rect(800, 0, 800, 600)));

        // Gesture key + drag the window to the right edge of the second screen.
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Pressed, 2);
        h.state.pointer_motion((400.0, 300.0).into(), 3);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 4);
        h.state.pointer_motion((1597.0, 300.0).into(), 5);
        assert_eq!(h.state.snap_preview(), Some(rect(1204, 8, 388, 584)));
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 6);
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Released, 7);
        h.wait_size("the right half of the second screen", 388, 584);
        assert_eq!(location(&h.state), Some((1204, 8).into()));

        // Maximized by the shortcut, it stays on the second screen.
        h.shortcut(KEY_UP);
        h.wait_size("maximizing", 800, 600);
        assert_eq!(location(&h.state), Some((800, 0).into()));
    }

    #[test]
    fn a_screen_unplugged_hands_its_windows_to_the_others() {
        let mut h = Harness::start();
        let second = h.state.add_output((1024, 768).into(), 1.0);
        h.show_window();
        // Gesture key + drag the window onto the second screen, away from its edges.
        let drag = |h: &mut Harness, to: (f64, f64)| {
            h.state
                .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Pressed, 2);
            h.state.pointer_motion((400.0, 300.0).into(), 3);
            h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 4);
            h.state.pointer_motion(to.into(), 5);
            h.state.pointer_button(BTN_LEFT, ButtonState::Released, 6);
            h.state
                .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Released, 7);
        };
        drag(&mut h, (1200.0, 300.0));
        assert_eq!(location(&h.state), Some((1150, 260).into()));

        // The second screen is unplugged: the pointer comes back to the first screen, and the
        // window is centered there.
        h.state.remove_screen(&second);
        assert_eq!(h.state.outputs().len(), 1);
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
        assert_eq!(location(&h.state), Some((350, 260).into()));

        // Plugged in again, it comes back to the right of the first one. The window, dropped
        // against its top edge, is maximized on it.
        let again = h.state.add_output((1024, 768).into(), 1.0);
        assert_eq!(
            h.state.space.output_geometry(&again),
            Some(rect(800, 0, 1024, 768))
        );
        drag(&mut h, (1200.0, 2.0));
        h.wait_size("maximizing on the second screen", 1024, 768);
        assert_eq!(location(&h.state), Some((800, 0).into()));

        // Unplugged again: maximized on the first screen.
        h.state.remove_screen(&again);
        h.wait_size("maximizing on the first screen", 800, 600);
        assert_eq!(location(&h.state), Some((0, 0).into()));
    }

    #[test]
    fn the_fractional_scale_is_advertised() {
        // 800 × 600 pixels at 1.5: 533 × 400 points.
        let mut h = Harness::start_scaled(1.5);
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals
                .iter()
                .any(|g| g.1 == "wp_fractional_scale_manager_v1")
        });
        for global in ["wp_viewporter", "zxdg_output_manager_v1"] {
            assert!(
                h.client.globals.iter().any(|g| g.1 == global),
                "{global} advertised"
            );
        }
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 533, 400)));
        let compositor: WlCompositor = h.bind(&registry);
        let wm_base: XdgWmBase = h.bind(&registry);
        let manager: WpFractionalScaleManagerV1 = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let _scale = manager.get_fractional_scale(&surface, &qh, ());
        let _toplevel = wm_base
            .get_xdg_surface(&surface, &qh, ())
            .get_toplevel(&qh, ());
        surface.commit();
        h.run_until("the screen scale", |c, _| c.preferred_scale.is_some());
        assert_eq!(h.client.preferred_scale, Some(180), "1.5 in 120ths");
    }

    /// The window is maximized, and its last configure is `w` × `h`.
    fn assert_maximized(h: &mut Harness, what: &str, w: i32, ht: i32) {
        let states = h.wait_size(what, w, ht);
        assert!(states.contains(&XdgState::Maximized), "{what}");
    }

    /// The protocol id of the surface that has the keyboard.
    fn keyboard_id(h: &Harness) -> Option<u32> {
        h.state.keyboard_focus().map(|f| f.id().protocol_id())
    }

    #[test]
    fn exclusive_zones_stack_then_are_given_back() {
        let mut h = Harness::start();
        h.show_window();
        h.shortcut(KEY_UP);
        assert_maximized(&mut h, "maximizing", 800, 600);
        let first = anchor(&mut h, BAR);
        let second = anchor(&mut h, BAR);
        let dock = Spec {
            layer: Layer::Bottom,
            anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
            size: (0, 76),
            zone: 76,
            margin: (0, 0, 0, 0),
            keyboard: KeyboardInteractivity::None,
        };
        let dock = anchor(&mut h, dock);
        // Two bars of 10 + 36 at the top, a dock of 76 at the bottom.
        assert_eq!(h.state.output_area(), Some(rect(0, 92, 800, 432)));
        assert_maximized(&mut h, "below the two bars and above the dock", 800, 432);
        // A zone of -1 does not move the others (the surface ignores them).
        let ignoring = anchor(&mut h, Spec { zone: -1, ..BAR });
        assert_eq!(h.state.output_area(), Some(rect(0, 92, 800, 432)));
        second.layer.destroy();
        assert_maximized(&mut h, "the second bar closed", 800, 478);
        assert_eq!(h.state.output_area(), Some(rect(0, 46, 800, 478)));
        for anchored in [first, dock, ignoring] {
            anchored.layer.destroy();
        }
        assert_maximized(&mut h, "everything closed", 800, 600);
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
    }

    #[test]
    fn an_exclusive_zone_too_large_is_refused() {
        let mut h = Harness::start();
        h.show_window();
        h.shortcut(KEY_UP);
        assert_maximized(&mut h, "maximizing", 800, 600);
        let refused = [
            // Larger than the screen.
            Spec {
                zone: 100_000,
                margin: (0, 0, 0, 0),
                ..BAR
            },
            // i32::MAX with a margin: smithay would overflow.
            Spec {
                zone: i32::MAX,
                ..BAR
            },
            // A margin far larger than the screen.
            Spec {
                margin: (i32::MAX, 0, 0, i32::MIN),
                ..BAR
            },
        ];
        for (i, spec) in refused.into_iter().enumerate() {
            anchor(&mut h, spec);
            assert_eq!(h.client.layer_closed, i + 1, "surface {i} closed");
            assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
        }
        assert_eq!(
            h.last_configure().1,
            600,
            "the maximized window has not moved"
        );
        // The compositor still works: snapped left, the window takes the whole height.
        h.shortcut(KEY_LEFT);
        h.wait_size("the left half", 388, 584);
    }

    #[test]
    fn zones_that_cover_the_screen_leave_a_minimum_area() {
        let mut h = Harness::start();
        h.show_window();
        let top = Spec {
            size: (0, 300),
            zone: 300,
            margin: (0, 0, 0, 0),
            ..BAR
        };
        let bottom = Spec {
            anchor: Anchor::Bottom | Anchor::Left | Anchor::Right,
            ..top
        };
        anchor(&mut h, top);
        anchor(&mut h, bottom);
        // The zones take the whole height: the work area keeps the minimum of sherlock-ui,
        // inside the screen.
        let (w, ht) = sherlock_ui::window::MIN_WORK_AREA;
        assert_eq!(h.state.output_area(), Some(rect(0, 300, 800, ht)));
        assert!(w <= 800);
        // One more pixel of exclusive zone does not fit: refused.
        anchor(
            &mut h,
            Spec {
                size: (0, 1),
                zone: 1,
                ..top
            },
        );
        assert_eq!(h.client.layer_closed, 1);
        h.shortcut(KEY_UP);
        assert_maximized(&mut h, "maximized in the minimum area", 800, ht);
        h.shortcut(KEY_LEFT);
        h.wait_size("the left half", 388, ht - 16);
    }

    #[test]
    fn an_unmapped_bar_gives_back_its_zone_and_the_keyboard() {
        let mut h = Harness::start();
        let shown = h.show_window();
        h.shortcut(KEY_UP);
        assert_maximized(&mut h, "maximizing", 800, 600);
        let mut bar = show_bar(&mut h);
        assert_maximized(&mut h, "below the bar", 800, 554);
        h.press(100.0, 20.0);
        h.release();
        assert_eq!(keyboard_id(&h), Some(bar.surface.id().protocol_id()));

        // A null buffer unmaps the bar: its zone and the keyboard come back.
        bar.surface.attach(None, 0, 0);
        bar.surface.commit();
        assert_maximized(&mut h, "the bar unmapped", 800, 600);
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
        assert_eq!(keyboard_id(&h), Some(shown.surface.id().protocol_id()));

        // Committed again, it gets a new configure, then shows again.
        bar.surface.commit();
        h.run_until("the new configure", |c, _| c.layer_configures.len() == 2);
        bar.draw(&mut h);
        assert_maximized(&mut h, "below the bar again", 800, 554);
    }

    #[test]
    fn a_surface_with_an_exclusive_keyboard_keeps_it() {
        let mut h = Harness::start();
        h.show_window();
        let panel = Spec {
            layer: Layer::Overlay,
            anchor: Anchor::Top | Anchor::Left,
            size: (1, 1),
            zone: 0,
            margin: (0, 0, 0, 0),
            keyboard: KeyboardInteractivity::Exclusive,
        };
        let panel = anchor(&mut h, panel);
        let id = Some(panel.surface.id().protocol_id());
        assert_eq!(keyboard_id(&h), id, "from its first content");
        h.press(400.0, 300.0);
        h.release();
        assert_eq!(keyboard_id(&h), id, "even after a click on the window");
        panel.layer.destroy();
        settle(&mut h);
        assert_ne!(keyboard_id(&h), id, "given back on leaving");
    }

    #[test]
    fn the_bar_client_leaves_and_gives_back_its_zone() {
        let mut h = Harness::start();
        h.show_window();
        h.shortcut(KEY_UP);
        assert_maximized(&mut h, "maximizing", 800, 600);
        // A second client, the bar, driven by hand.
        let (server_side, client_side) = UnixStream::pair().unwrap();
        h.state.insert_client(server_side);
        let conn = Connection::from_socket(client_side).unwrap();
        let mut queue: EventQueue<Client> = conn.new_event_queue();
        let mut bar_client = Client::default();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut step = |h: &mut Harness, client: &mut Client| {
            for _ in 0..10 {
                conn.flush().unwrap();
                h.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut h.state)
                    .unwrap();
                h.state.refresh();
                if let Some(guard) = queue.prepare_read() {
                    let _ = guard.read();
                }
                queue.dispatch_pending(client).unwrap();
            }
        };
        step(&mut h, &mut bar_client);
        let global = |name: &str| {
            let g = bar_client.globals.iter().find(|g| g.1 == name).unwrap();
            (g.0, g.2)
        };
        let ((c, cv), (m, mv), (l, lv)) = (
            global("wl_compositor"),
            global("wl_shm"),
            global("zwlr_layer_shell_v1"),
        );
        let compositor: WlCompositor = registry.bind(c, cv.min(5), &qh, ());
        let shm: WlShm = registry.bind(m, mv.min(1), &qh, ());
        let layer_shell: ZwlrLayerShellV1 = registry.bind(l, lv.min(4), &qh, ());
        let surface = compositor.create_surface(&qh, ());
        let bar = layer_shell.get_layer_surface(&surface, None, Layer::Top, "b".into(), &qh, ());
        bar.set_size(0, 36);
        bar.set_anchor(Anchor::Top | Anchor::Left | Anchor::Right);
        bar.set_exclusive_zone(36);
        surface.commit();
        step(&mut h, &mut bar_client);
        let file = shm_file(800 * 36 * 4);
        let pool = shm.create_pool(file.as_fd(), 800 * 36 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 800, 36, 800 * 4, Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        step(&mut h, &mut bar_client);
        assert_maximized(&mut h, "below the bar of the other client", 800, 564);
        // The bar's client goes away without a word.
        drop(step);
        drop((
            queue,
            registry,
            compositor,
            shm,
            layer_shell,
            surface,
            bar,
            pool,
            buffer,
        ));
        drop(conn);
        assert_maximized(&mut h, "the bar client gone", 800, 600);
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
    }

    #[test]
    fn two_screens_at_different_scales_receive_the_pointer() {
        let event_loop = EventLoop::<State>::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((720, 900).into(), 1.0);
        state.add_output((720, 900).into(), 2.0);
        state.resize_frame((1440, 900).into());
        let at = |x: f64, y: f64| state.frame_to_space((x, y).into());
        assert_eq!(at(719.0, 10.0), Some((719.0, 10.0).into()));
        // The second screen starts at 720 in the space, at half the pixels.
        assert_eq!(at(720.0, 10.0), Some((720.0, 5.0).into()));
        assert_eq!(at(1439.0, 899.0), Some((1079.5, 449.5).into()));
    }

    #[test]
    fn a_size_beyond_2_31_is_a_client_error() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "zwlr_layer_shell_v1")
        });
        let compositor: WlCompositor = h.bind(&registry);
        let layer_shell: ZwlrLayerShellV1 = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let layer = layer_shell.get_layer_surface(&surface, None, Layer::Top, "x".into(), &qh, ());
        // smithay would read it as a negative height, and assert against it.
        layer.set_size(100, u32::MAX);
        layer.set_anchor(Anchor::Top);
        surface.commit();
        // The client ends in a protocol error; the compositor goes on.
        for _ in 0..10 {
            let _ = h.conn.flush();
            h.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut h.state)
                .unwrap();
            h.state.refresh();
            if let Some(guard) = h.queue.prepare_read() {
                let _ = guard.read();
            }
            let _ = h.queue.dispatch_pending(&mut h.client);
        }
        let error = h.conn.protocol_error().expect("a protocol error");
        assert_eq!(error.object_interface, "zwlr_layer_surface_v1");
        assert_eq!(error.code, 1, "invalid_size");
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
    }

    /// A client maximizes its window before its first content, then the bar arrives on the
    /// screen under the pointer, then the client draws: the window ends up under the bar, at
    /// (`x`, 46), whatever the order.
    fn maximized_before_the_bar(h: &mut Harness, x: i32) {
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "xdg_wm_base")
        });
        let compositor: WlCompositor = h.bind(&registry);
        let wm_base: XdgWmBase = h.bind(&registry);
        let shm: WlShm = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let toplevel = wm_base
            .get_xdg_surface(&surface, &qh, ())
            .get_toplevel(&qh, ());
        toplevel.set_maximized();
        surface.commit();
        assert_maximized(h, "maximized before any content", 800, 600);
        anchor(h, BAR);
        assert_maximized(h, "below the bar, still without content", 800, 554);
        let file = shm_file(800 * 554 * 4);
        let pool = shm.create_pool(file.as_fd(), 800 * 554 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 800, 554, 800 * 4, Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        h.run_until("the first content below the bar", |_, s| {
            let window = s.space.elements().next();
            window.and_then(|w| s.space.element_geometry(w)) == Some(rect(x, 46, 800, 554))
        });
    }

    #[test]
    fn a_window_maximized_before_the_bar_goes_below_it() {
        let mut h = Harness::start();
        maximized_before_the_bar(&mut h, 0);
    }

    #[test]
    fn a_window_maximized_before_the_bar_goes_below_it_on_the_second_screen() {
        let mut h = Harness::start();
        h.state.add_output((800, 600).into(), 1.0);
        h.state.pointer_motion((1200.0, 300.0).into(), 1);
        maximized_before_the_bar(&mut h, 800);
        // The first screen has no bar: its work area is whole.
        h.state.pointer_motion((400.0, 300.0).into(), 2);
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
    }

    /// The dock at the bottom: 76 high, with an exclusive zone of 76, the keyboard on demand.
    const DOCK: Spec = Spec {
        layer: Layer::Top,
        anchor: Anchor::Bottom.union(Anchor::Left).union(Anchor::Right),
        size: (0, 76),
        zone: 76,
        margin: (0, 0, 0, 0),
        keyboard: KeyboardInteractivity::OnDemand,
    };

    /// Lot D-18d: a maximized or snapped window pulled by its title bar comes off under the
    /// pointer at its previous size and stays where it is dropped, also when the dock comes
    /// back and the work area shrinks.
    #[test]
    fn a_window_pulled_out_of_its_zone_stays_where_it_is_dropped() {
        // Shortcut, size in the work area above the dock, size once the dock hid, a point on
        // the title bar.
        let cases = [
            (KEY_UP, (800, 524), (800, 600), (400.0, 10.0)),
            (KEY_LEFT, (388, 508), (388, 584), (100.0, 20.0)),
        ];
        for (key, above_dock, whole, title) in cases {
            let mut h = Harness::start();
            let shown = h.show_window();
            // Frames are drawn all along, as on a screen: the motion of the window follows.
            h.drawn(0);
            let dock = anchor(&mut h, DOCK);
            h.shortcut(key);
            h.drawn(1_000);
            h.wait_size("in its zone, above the dock", above_dock.0, above_dock.1);
            let _a = h.redraw(&shown, above_dock.0, above_dock.1);
            // The dock hides: it gives up its zone, the window takes the whole height.
            dock.layer.set_exclusive_zone(-1);
            dock.surface.commit();
            h.wait_size("the whole height", whole.0, whole.1);
            h.drawn(2_000);
            let _b = h.redraw(&shown, whole.0, whole.1);
            h.drawn(3_000);

            // Pulled by its title bar: it comes off at its previous size, under the pointer.
            let serial = h.press(title.0, title.1);
            shown.toplevel._move(&shown.seat, serial);
            h.run_until("the gesture start", |_, s| s.gesture_active());
            h.state.pointer_motion((300.0, 300.0).into(), 3);
            let states = h.wait_size("the previous size", 100, 80);
            assert!(!states.contains(&XdgState::Maximized));
            h.drawn(3_010);
            let _c = h.redraw(&shown, 100, 80);
            h.drawn(3_020);
            h.state.pointer_motion((320.0, 310.0).into(), 4);
            h.drawn(3_030);
            let dropped = location(&h.state).unwrap();
            h.release();
            settle(&mut h);
            assert_eq!(location(&h.state), Some(dropped), "moved by the release");
            h.drawn(3_040);

            // The dock comes back with its zone: the floating window stays.
            dock.layer.set_exclusive_zone(76);
            dock.surface.commit();
            settle(&mut h);
            assert_eq!(location(&h.state), Some(dropped), "moved by the dock");
            assert_eq!(h.last_configure().0, 100, "resized by the dock");
            // Drawn where it is, at its size, all along.
            for t in [3_050, 3_100, 3_300, 5_000] {
                let (drawn, real) = h.drawn(t);
                assert_eq!(drawn, real, "drawn away from its place at {t} ms");
            }
            let (drawn, real) = h.drawn(60_000);
            assert_eq!(drawn, real);
            assert_eq!(real, rect(dropped.x, dropped.y, 100, 80).to_f64());
        }
    }
    /// Lot D-18d: some clients leave the maximized state themselves when their title bar is
    /// dragged, before or after asking for the move. The window still comes off under the
    /// pointer, never back at its place from before it was maximized.
    #[test]
    fn a_client_that_unmaximizes_itself_while_dragged_comes_off_under_the_pointer() {
        for unmaximize_first in [true, false] {
            let mut h = Harness::start();
            let shown = h.show_window();
            h.drawn(0);
            h.shortcut(KEY_UP);
            h.wait_size("maximized", 800, 600);
            let _a = h.redraw(&shown, 800, 600);
            h.drawn(1_000);

            // Pressed on the title bar, then dragged 14 px: past the threshold of 4.
            let serial = h.press(400.0, 10.0);
            if unmaximize_first {
                h.state.pointer_motion((410.0, 20.0).into(), 3);
                shown.toplevel.unset_maximized();
                shown.toplevel._move(&shown.seat, serial);
            } else {
                shown.toplevel._move(&shown.seat, serial);
                h.run_until("the gesture start", |_, s| s.gesture_active());
                h.state.pointer_motion((410.0, 20.0).into(), 3);
                shown.toplevel.unset_maximized();
            }
            h.run_until("the gesture start", |_, s| s.gesture_active());
            let states = h.wait_size("the previous size", 100, 80);
            assert!(!states.contains(&XdgState::Maximized));
            // Grabbed in the middle of its width, as it was pressed: 400 of 800, 50 of 100;
            // never at (350, 260), its place before.
            let _b = h.redraw(&shown, 100, 80);
            h.state.pointer_motion((420.0, 200.0).into(), 4);
            let place = Some((370, 190).into());
            assert_eq!(location(&h.state), place, "{unmaximize_first}");
            h.release();
            settle(&mut h);
            assert_eq!(location(&h.state), place, "{unmaximize_first}");
            for t in [1_010, 1_100, 1_500, 5_000] {
                let (drawn, real) = h.drawn(t);
                assert_eq!(drawn, real, "{unmaximize_first}: drawn away at {t} ms");
            }
        }
    }

    /// Lot D-18d: a double click on a title bar that the client draws. GTK 4 unmaximizes at
    /// the second press, the button held and the pointer still: the window goes back to its
    /// place, it does not come off under the pointer.
    #[test]
    fn a_double_click_on_a_client_title_bar_restores_the_window_to_its_place() {
        let mut h = Harness::start();
        let shown = h.show_window();
        h.shortcut(KEY_UP);
        h.wait_size("maximized", 800, 600);
        let _a = h.redraw(&shown, 800, 600);
        h.press(400.0, 10.0);
        h.release();
        h.press(400.0, 10.0);
        shown.toplevel.unset_maximized();
        let states = h.wait_size("the previous size", 100, 80);
        assert!(!states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((350, 260).into()));
        assert!(!h.state.gesture_active());
        h.release();
        settle(&mut h);
        assert_eq!(location(&h.state), Some((350, 260).into()));
    }

    #[test]
    fn a_client_that_asks_gets_its_decoration_from_the_compositor() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let (shown, decoration) = h.show_decorated();
        assert_eq!(h.client.decorations, [DecorationMode::ServerSide]);
        let window = h.state.space.elements().next().unwrap().clone();
        assert!(compositor::decorations::server_side(&window));

        // A client that wants to draw its own keeps it, and the bar goes away.
        decoration.set_mode(DecorationMode::ClientSide);
        h.run_until("client side", |c, _| c.decorations.len() == 2);
        assert_eq!(h.client.decorations[1], DecorationMode::ClientSide);
        assert!(!compositor::decorations::server_side(&window));
        // Without a preference, the compositor draws it again.
        decoration.unset_mode();
        h.run_until("server side again", |c, _| c.decorations.len() == 3);
        assert_eq!(h.client.decorations[2], DecorationMode::ServerSide);

        // KDE's older protocol, which GTK 3 and the Qt 5 of KDE use: server by default.
        let registry = h.conn.display().get_registry(&qh, ());
        let manager: OrgKdeKwinServerDecorationManager = h.bind(&registry);
        h.run_until("the default mode", |c, _| c.kde_default.is_some());
        let server = u32::from(kde_manager::Mode::Server);
        assert_eq!(h.client.kde_default, Some(server));
        let compositor: WlCompositor = h.bind(&registry);
        let other = compositor.create_surface(&qh, ());
        let kde = manager.create(&other, &qh, ());
        h.run_until("its mode", |c, _| !c.kde_modes.is_empty());
        assert_eq!(h.client.kde_modes, [server]);
        kde.request_mode(kde::Mode::Client);
        h.run_until("the client mode", |c, _| c.kde_modes.len() == 2);
        assert_eq!(h.client.kde_modes[1], u32::from(kde::Mode::Client));
        drop(shown);
    }

    #[test]
    fn the_title_bar_moves_and_maximizes_the_window_and_its_buttons_act() {
        let mut h = Harness::start();
        let (shown, _decoration) = h.show_decorated();
        let window = h.state.space.elements().next().unwrap().clone();

        // A drag of the bar moves the window; the client does not see the press.
        h.state.pointer_motion((300.0, 150.0).into(), 1);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        assert!(h.state.gesture_active());
        h.state.pointer_motion((320.0, 170.0).into(), 3);
        h.release();
        assert_eq!(location(&h.state), Some((220, 191).into()));
        assert_eq!(h.client.button_serial, None);

        // A double click maximizes it under its bar: the content takes the screen but 42.
        h.click(300.0, 170.0, 1000);
        h.click(301.0, 170.0, 1200);
        let states = h.wait_size("maximized", 800, 558);
        assert!(states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((0, 42).into()));
        let _file = h.redraw(&shown, 800, 558);

        // Its maximize button, now the restore one, gives it back its size and place.
        h.click(745.0, 21.0, 2000);
        let states = h.wait_size("restored", 400, 300);
        assert!(!states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((220, 191).into()));
        let _file = h.redraw(&shown, 400, 300);

        // Dropped at the top of the screen, it is maximized, like any moved window.
        h.state.pointer_motion((300.0, 170.0).into(), 3000);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 3000);
        h.state.pointer_motion((300.0, 2.0).into(), 3001);
        h.release();
        h.wait_size("maximized by the drop", 800, 558);
        let _file = h.redraw(&shown, 800, 558);
        h.click(745.0, 21.0, 4000);
        h.wait_size("restored again", 400, 300);
        let _file = h.redraw(&shown, 400, 300);

        // A press that leaves its button does nothing.
        let (x, y) = (220.0 + 400.0 - 101.0 + 16.0, 191.0 - 42.0 + 21.0);
        h.state.pointer_motion((x, y).into(), 5000);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 5000);
        h.state.pointer_motion((x - 100.0, y).into(), 5001);
        h.state
            .pointer_button(BTN_LEFT, ButtonState::Released, 5002);
        assert!(!h.state.is_minimized(&window));
        // Minimize, then close.
        h.click(x, y, 6000);
        assert!(h.state.is_minimized(&window));
        h.state.activate(&window);
        // The pointer reaches it again once it is back from its icon (lot D-19).
        h.run_until("the window back", |_, s| {
            s.minimize_progress(&window, s.now()).is_none()
        });
        h.click(x + 60.0, y, 7000);
        h.run_until("the close request", |c, _| c.closed);
        drop(shown);
    }

    /// Captures of a decorated window, dark then light with the pointer on the maximize
    /// button: the scene of a screen, as the real session draws it, but into a texture with
    /// Mesa's software renderer. They go to `screenshots/`, to compare with the Main board.
    #[test]
    #[ignore = "needs Mesa's EGL; writes the captures: run it with --ignored"]
    fn captures_of_a_decorated_window() {
        use compositor::{damage::Ids, render::Scene, wallpaper};
        use sherlock_ui::color::{self, Theme};
        use smithay::backend::{
            allocator::Fourcc,
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::{
                Bind as _, ExportMem as _, Offscreen as _, TextureMapping as _,
                damage::OutputDamageTracker,
                gles::{GlesRenderer, GlesTexture},
            },
        };
        use smithay::utils::Transform;

        let mut device = EGLDevice::enumerate().unwrap();
        let device = device
            .find(EGLDevice::is_software)
            .expect("no software EGL device");
        // SAFETY: the display lives as long as the renderer, in this test only.
        let display = unsafe { EGLDisplay::new(device) }.unwrap();
        let context = EGLContext::new(&display).unwrap();
        // SAFETY: the context was just made and is used by this renderer alone.
        let mut renderer = unsafe { GlesRenderer::new(context) }.unwrap();
        let scene = Scene::new(&mut renderer).unwrap();
        for (theme, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
            color::set_active(theme);
            let mut h = Harness::start();
            let (_shown, _decoration) = h.show_decorated_in(theme.palette().window);
            if theme == Theme::Light {
                h.state.pointer_motion((545.0, 150.0).into(), 1);
            }
            let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());
            let views = h.state.views();
            let now = h.state.now();
            // The second frame, once the opening is over.
            let mut elements = Vec::new();
            for at in [now, now + Duration::from_secs(2)] {
                let state = &mut h.state;
                let drawn = scene.screens(
                    &mut renderer,
                    &mut ids,
                    state,
                    &mut textures,
                    &views,
                    None,
                    at,
                );
                elements = drawn.0.into_iter().next().unwrap();
            }
            let (w, h) = (800, 600);
            let mut texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, (w, h).into())
                .unwrap();
            let mut target = renderer.bind(&mut texture).unwrap();
            let mut tracker = OutputDamageTracker::new((w, h), 1.0, Transform::Normal);
            let background = color::palette().desktop.premultiplied();
            tracker
                .render_output(&mut renderer, &mut target, 0, &elements, background)
                .unwrap();
            let region = smithay::utils::Rectangle::from_size((w, h).into());
            let mapping = renderer
                .copy_framebuffer(&target, region, Fourcc::Abgr8888)
                .unwrap();
            let mut pixels = renderer.map_texture(&mapping).unwrap().to_vec();
            // GL reads the rows from the bottom up.
            if !mapping.flipped() {
                pixels = pixels
                    .chunks(w as usize * 4)
                    .rev()
                    .flatten()
                    .copied()
                    .collect();
            }
            let path = format!(
                "{}/screenshots/decorations-{name}.png",
                env!("CARGO_MANIFEST_DIR")
            );
            image::save_buffer(&path, &pixels, w as u32, h as u32, image::ColorType::Rgba8)
                .unwrap();
            // The bar has the color of its theme, under the title and beside the buttons.
            let at = |x: usize, y: usize| &pixels[(y * w as usize + x) * 4..][..3];
            let bar = color::palette().title_bar;
            assert_eq!(at(400, 140), [bar.r, bar.g, bar.b], "{name}");
        }
        color::set_active(Theme::Dark);
    }
}
