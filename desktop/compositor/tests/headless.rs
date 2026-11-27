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

    use compositor::{State, render::Item};
    use sherlock_ui::color::{Color, Palette};
    use smithay::reexports::wayland_server::Resource as _;
    use smithay::{
        backend::input::{ButtonState, KeyState},
        desktop::Window,
        input::{keyboard::Keycode, pointer::CursorIcon},
        reexports::calloop::EventLoop,
        utils::{Logical, Physical, Point, Rectangle},
    };
    use smithay::{
        desktop::layer_map_for_output,
        reexports::wayland_server::protocol::wl_surface::WlSurface as ServerSurface,
        wayland::{seat::WaylandFocus as _, shell::wlr_layer::Layer as ServerLayer},
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
    use wayland_protocols::wp::cursor_shape::v1::client::{
        wp_cursor_shape_device_v1::{Shape as CursorShape, WpCursorShapeDeviceV1},
        wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
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
        xdg_popup::{self, XdgPopup},
        xdg_positioner::{self, XdgPositioner},
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
        /// Serial of the last `wl_pointer.enter`: what `wp_cursor_shape_device_v1.set_shape`
        /// must give.
        enter_serial: Option<u32>,
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
        /// Popups the compositor closed (`popup_done`), by protocol id.
        popups_done: Vec<u32>,
    }

    impl Dispatch<XdgPopup, ()> for Client {
        fn event(
            client: &mut Self,
            popup: &XdgPopup,
            event: xdg_popup::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_popup::Event::PopupDone = event {
                client.popups_done.push(popup.id().protocol_id());
            }
        }
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
            match event {
                wl_pointer::Event::Button { serial, .. } => client.button_serial = Some(serial),
                wl_pointer::Event::Enter { serial, .. } => client.enter_serial = Some(serial),
                _ => {}
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
    delegate_noop!(Client: WpCursorShapeManagerV1);
    delegate_noop!(Client: WpCursorShapeDeviceV1);
    delegate_noop!(Client: XdgPositioner);

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
    const KEY_SPACE: u32 = 65;

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
        // Held by its title bar: the closed hand (lot C-01).
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Grabbing));
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

    /// What the bar needs to open its panels.
    struct Panels {
        compositor: WlCompositor,
        wm_base: XdgWmBase,
        seat: WlSeat,
    }

    fn panels(h: &mut Harness) -> Panels {
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "xdg_wm_base")
        });
        Panels {
            compositor: h.bind(&registry),
            wm_base: h.bind(&registry),
            seat: h.bind(&registry),
        }
    }

    /// The bar opens a panel: a popup of its surface, under its right end, with a grab. Sent
    /// only, as the bar sends it.
    fn open_panel(h: &Harness, bar: &Anchored, with: &Panels) -> XdgPopup {
        let qh = h.queue.handle();
        let positioner = with.wm_base.create_positioner(&qh, ());
        positioner.set_size(200, 150);
        positioner.set_anchor_rect(560, 0, 2, 36);
        let surface = with.compositor.create_surface(&qh, ());
        let popup =
            with.wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_popup(None, &positioner, &qh, ());
        bar.layer.get_popup(&popup);
        popup.grab(&with.seat, 0);
        surface.commit();
        popup
    }

    #[test]
    fn a_panel_opened_again_still_closes_on_a_click_elsewhere() {
        let mut h = Harness::start();
        let bar = show_bar(&mut h);
        let with = panels(&mut h);
        let first = open_panel(&h, &bar, &with);
        settle(&mut h);
        // The bar closes its panel and opens another one in the same batch, as when it
        // switches from the calendar to the control centre.
        first.destroy();
        let second = open_panel(&h, &bar, &with);
        settle(&mut h);
        h.click(400.0, 500.0, 10);
        let id = second.id().protocol_id();
        h.run_until("the panel closed by a click elsewhere", |c, _| {
            c.popups_done.contains(&id)
        });
        // Closed, then opened again in another batch: the same.
        second.destroy();
        settle(&mut h);
        let third = open_panel(&h, &bar, &with);
        settle(&mut h);
        h.click(400.0, 500.0, 20);
        let id = third.id().protocol_id();
        h.run_until(
            "the panel opened again, closed by a click elsewhere",
            |c, _| c.popups_done.contains(&id),
        );
    }

    /// A panel of the bar closed by a click elsewhere fades out from its last image, drawn
    /// for real with Mesa's software renderer (lot UI-06): the panel's color where it was at
    /// the click, mixed with what is behind it halfway, gone after `motion::POPUP_CLOSE`.
    #[test]
    fn a_panel_closed_by_a_click_elsewhere_fades_out() {
        use compositor::{damage::Ids, render::Scene, wallpaper};
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
        use std::io::Write as _;
        use std::{cell::RefCell, rc::Rc};

        let software = EGLDevice::enumerate()
            .ok()
            .and_then(|mut d| d.find(EGLDevice::is_software));
        let Some(device) = software else {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI has Mesa's EGL (libegl-mesa0): its software device must be found"
            );
            eprintln!("no software EGL device (Mesa's llvmpipe): nothing is drawn");
            return;
        };
        // SAFETY: the display lives as long as the renderer, in this test only.
        let display = unsafe { EGLDisplay::new(device) }.unwrap();
        let context = EGLContext::new(&display).unwrap();
        // SAFETY: the context was just made and is used by this renderer alone.
        let renderer = unsafe { GlesRenderer::new(context) }.unwrap();
        let gpu = Rc::new(RefCell::new(renderer));
        let scene = Rc::new(Scene::new(&mut gpu.borrow_mut()).unwrap());
        let (w, ht) = (800usize, 600usize);
        let shoot = |state: &mut State, at: Duration| {
            let mut renderer = gpu.borrow_mut();
            let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());
            let views = state.views();
            let drawn = scene.screens(
                &mut renderer,
                &mut ids,
                state,
                &mut textures,
                &views,
                None,
                at,
            );
            let elements = drawn.0.into_iter().next().unwrap();
            let size = (w as i32, ht as i32);
            let mut texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, size.into())
                .unwrap();
            let mut target = renderer.bind(&mut texture).unwrap();
            let mut tracker = OutputDamageTracker::new(size, 1.0, Transform::Normal);
            tracker
                .render_output(
                    &mut *renderer,
                    &mut target,
                    0,
                    &elements,
                    [0.0, 0.0, 0.0, 1.0],
                )
                .unwrap();
            let region = Rectangle::from_size(size.into());
            let mapping = renderer
                .copy_framebuffer(&target, region, Fourcc::Abgr8888)
                .unwrap();
            let pixels = renderer.map_texture(&mapping).unwrap().to_vec();
            match mapping.flipped() {
                true => pixels,
                false => pixels.chunks(w * 4).rev().flatten().copied().collect(),
            }
        };
        let at = |pixels: &[u8], x: usize, y: usize| pixels[(y * w + x) * 4..][..3].to_vec();

        let mut h = Harness::start();
        let (gpu_, scene_) = (Rc::clone(&gpu), Rc::clone(&scene));
        h.state
            .set_popup_snapshotter(Box::new(move |state, surface, size, scale| {
                let mut renderer = gpu_.try_borrow_mut().ok()?;
                compositor::popup_anim::snapshot(
                    &scene_,
                    &mut renderer,
                    state,
                    surface,
                    size,
                    scale,
                )
            }));
        let bar = show_bar(&mut h);
        let with = panels(&mut h);
        // What is behind the panel, before it opens.
        let now = h.state.now();
        let bare = shoot(&mut h.state, now);
        let qh = h.queue.handle();
        let positioner = with.wm_base.create_positioner(&qh, ());
        positioner.set_size(200, 150);
        positioner.set_anchor_rect(500, 36, 2, 2);
        positioner.set_anchor(xdg_positioner::Anchor::Bottom);
        positioner.set_gravity(xdg_positioner::Gravity::Bottom);
        let surface = with.compositor.create_surface(&qh, ());
        let popup =
            with.wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_popup(None, &positioner, &qh, ());
        bar.layer.get_popup(&popup);
        popup.grab(&with.seat, 0);
        surface.commit();
        settle(&mut h);
        // An opaque magenta panel (ARGB8888: blue, green, red, alpha in memory).
        let mut file = shm_file(200 * 150 * 4);
        file.write_all(&[255, 0, 255, 255].repeat(200 * 150))
            .unwrap();
        let pool = bar.shm.create_pool(file.as_fd(), 200 * 150 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 200, 150, 800, Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.damage(0, 0, 200, 150);
        surface.commit();
        settle(&mut h);
        let magenta = vec![255u8, 0, 255];
        let now = h.state.now();
        let open = shoot(&mut h.state, now);
        let (x, y) = (0..w * ht)
            .map(|i| (i % w, i / w))
            .find(|(x, y)| at(&open, *x, *y) == magenta)
            .expect("the panel is drawn");
        // Its middle, well inside it whatever its exact place.
        let (x, y) = (x + 100, y + 75);
        assert_eq!(at(&open, x, y), magenta);
        h.click(100.0, 500.0, 10);
        settle(&mut h);
        assert_eq!(h.state.closing_popups(), 1);
        let closed = h.state.now();
        let first = shoot(&mut h.state, closed);
        let half = shoot(&mut h.state, closed + sherlock_ui::motion::POPUP_CLOSE / 2);
        let after = shoot(&mut h.state, closed + sherlock_ui::motion::POPUP_CLOSE * 2);
        // Its last image, nearly opaque at the click (the loop turned since the fade began).
        let near = |a: &[u8], b: &[u8]| a.iter().zip(b).all(|(a, b)| a.abs_diff(*b) <= 16);
        let first = at(&first, x, y);
        assert!(near(&first, &magenta), "{first:?}");
        let mid = at(&half, x, y);
        assert!(
            mid != magenta && mid != at(&bare, x, y),
            "mixed halfway: {mid:?}"
        );
        assert_eq!(at(&after, x, y), at(&bare, x, y), "gone after its fade");
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

    /// Lot D-21g: as on macOS, a dock that hides by itself reserves no room. A maximized window,
    /// then the dock hidden as the real one is (no zone, all but 2 of its 76 below the edge),
    /// brought back by the pointer (its margin back to 0) and hidden again: the window gets no
    /// new configure, and keeps its size.
    #[test]
    fn a_dock_revealed_then_hidden_does_not_resize_a_maximized_window() {
        let mut h = Harness::start();
        let _shown = h.show_window_with(true);
        h.wait_size("maximized", 800, 600);
        let hidden = Spec {
            zone: -1,
            margin: (0, 0, 2 - 76, 0),
            ..DOCK
        };
        let dock = anchor(&mut h, hidden);
        settle(&mut h);
        let window = h.state.space.elements().next().unwrap().clone();
        let size = h.state.space.element_geometry(&window).map(|g| g.size);
        let configures = h.client.configures.len();
        // Revealed, then hidden again, a few steps of its slide each way.
        for bottom in [-50, -20, 0, -20, -50, 2 - 76] {
            dock.layer.set_margin(0, 0, bottom, 0);
            dock.surface.commit();
            settle(&mut h);
            assert_eq!(
                h.state.output_area(),
                Some(rect(0, 0, 800, 600)),
                "{bottom}"
            );
        }
        assert_eq!(h.client.configures.len(), configures, "no new configure");
        assert_eq!(
            h.state.space.element_geometry(&window).map(|g| g.size),
            size
        );
    }

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

    /// Lot C-01: dragged up, a window stops with its title bar under the top bar, where it
    /// can still be grabbed; dropped there, it stays.
    #[test]
    fn a_window_dragged_up_keeps_its_title_bar_under_the_top_bar() {
        let mut h = Harness::start();
        let (_shown, _decoration) = h.show_decorated();
        let _bar = show_bar(&mut h);
        // The whole window, 342 high, at (200, 129): its bar from 129 to 171, below the top bar.
        assert_eq!(location(&h.state), Some((200, 171).into()));
        h.state.pointer_motion((300.0, 150.0).into(), 1);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        assert!(h.state.gesture_active());
        // Up to the middle of the top bar, short of the edge that maximizes.
        h.state.pointer_motion((300.0, 20.0).into(), 3);
        assert_eq!(location(&h.state), Some((200, 46 + 42).into()), "stopped");
        h.state.pointer_motion((350.0, 12.0).into(), 4);
        assert_eq!(
            location(&h.state),
            Some((250, 88).into()),
            "it still slides"
        );
        h.release();
        assert_eq!(location(&h.state), Some((250, 88).into()), "dropped there");
        // Its title bar is in reach: a press on it moves it again.
        h.state.pointer_motion((350.0, 67.0).into(), 5);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 6);
        assert!(h.state.gesture_active(), "the title bar is grabbed");
        h.state.pointer_motion((350.0, 167.0).into(), 7);
        h.release();
        assert_eq!(location(&h.state), Some((250, 188).into()));
    }

    /// A client that draws its own title bar stops at the top of the work area, its top edge
    /// under the top bar.
    #[test]
    fn a_window_without_our_bar_stops_under_the_top_bar() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();
        let _bar = show_bar(&mut h);
        let start = location(&h.state).unwrap();
        let (x, y) = (f64::from(start.x) + 10.0, f64::from(start.y) + 10.0);
        let serial = h.press(x, y);
        toplevel._move(&seat, serial);
        h.run_until("the gesture start", |_, s| s.gesture_active());
        h.state.pointer_motion((x, 15.0).into(), 3);
        assert_eq!(location(&h.state), Some((start.x, 46).into()));
        h.release();
        assert_eq!(location(&h.state), Some((start.x, 46).into()));
    }

    /// A window already under an anchored surface that leaves it no room (no exclusive zone)
    /// is taken back with the gesture key, through that surface.
    #[test]
    fn the_gesture_key_takes_back_a_window_under_a_bar() {
        let mut h = Harness::start();
        let _overlay = anchor(&mut h, Spec { zone: 0, ..BAR });
        assert_eq!(h.state.output_area(), Some(rect(0, 0, 800, 600)));
        let _shown = h.show_window();
        // Moved to the top of the screen: the bar covers its top 36 pixels.
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Pressed, 1);
        h.state.pointer_motion((400.0, 300.0).into(), 2);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 3);
        h.state.pointer_motion((400.0, 50.0).into(), 4);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 5);
        assert_eq!(location(&h.state), Some((350, 10).into()));
        // On the bar, with the gesture key: the window below moves.
        h.state.pointer_motion((400.0, 20.0).into(), 6);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 7);
        assert!(
            h.state.gesture_active(),
            "the window under the bar is grabbed"
        );
        h.state.pointer_motion((400.0, 220.0).into(), 8);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 9);
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Released, 10);
        assert_eq!(location(&h.state), Some((350, 210).into()));
        // Without it, the click goes to the bar.
        h.state.pointer_motion((400.0, 20.0).into(), 11);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 12);
        assert!(!h.state.gesture_active());
        h.release();
    }

    /// A window whose first content is larger than the work area (a size its application kept
    /// from a larger screen) is asked to fit, and shows once it has: its corners in reach.
    #[test]
    fn a_window_larger_than_the_screen_fits_before_it_shows() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "wl_shm")
        });
        let compositor: WlCompositor = h.bind(&registry);
        let wm_base: XdgWmBase = h.bind(&registry);
        let shm: WlShm = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let _toplevel = wm_base
            .get_xdg_surface(&surface, &qh, ())
            .get_toplevel(&qh, ());
        surface.commit();
        h.run_until("the first configure", |c, _| !c.configures.is_empty());
        let file = shm_file(1000 * 700 * 4);
        let pool = shm.create_pool(file.as_fd(), 1000 * 700 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 1000, 700, 4000, Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        h.wait_size("the size of the screen", 800, 600);
        let window = h.state.space.elements().next().unwrap().clone();
        assert!(h.state.window_visual(&window, h.state.now()).is_none());
        let shown = Shown {
            seat: h.bind(&registry),
            toplevel: _toplevel,
            surface,
            shm,
            _file: file,
        };
        let _file = h.redraw(&shown, 800, 600);
        h.run_until("the placement", |_, s| location(s) == Some((0, 0).into()));
        assert!(h.state.window_visual(&window, h.state.now()).is_some());
    }

    /// A terminal: a minimum size, and a content that takes whole cells (8 × 16) of the size
    /// it is given. Its corners resize it; the opposite corner stays in place.
    #[test]
    fn a_terminal_resizes_by_its_corners() {
        let mut h = Harness::start();
        let shown = h.show_window();
        shown.toplevel.set_min_size(96, 64);
        shown.surface.commit();
        settle(&mut h);
        // Bottom right, just outside the corner of the window (350, 260, 100 × 80).
        h.state.pointer_motion((452.0, 342.0).into(), 1);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::SeResize));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        h.state.pointer_motion((552.0, 402.0).into(), 3);
        h.wait_size("larger", 200, 140);
        h.release();
        let cells = |w: i32, ht: i32| (w / 8 * 8, ht / 16 * 16);
        let (w, ht) = cells(200, 140);
        let _file = h.redraw(&shown, w, ht);
        assert_eq!(location(&h.state), Some((350, 260).into()));
        // Top left: the bottom right corner stays where it is.
        h.state.pointer_motion((348.0, 258.0).into(), 4);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::NwResize));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 5);
        h.state.pointer_motion((298.0, 198.0).into(), 6);
        h.wait_size("larger again", 250, 188);
        h.release();
        let (w2, ht2) = cells(250, 188);
        let _file = h.redraw(&shown, w2, ht2);
        h.run_until("the realigned window", |_, s| {
            location(s) == Some((350 + w - w2, 260 + ht - ht2).into())
        });
        // Never below its minimum.
        let at = location(&h.state).unwrap();
        let corner = (f64::from(at.x + w2) + 2.0, f64::from(at.y + ht2) + 2.0);
        h.state.pointer_motion(corner.into(), 7);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 8);
        h.state
            .pointer_motion((corner.0 - 400.0, corner.1 - 400.0).into(), 9);
        h.wait_size("the minimum", 96, 64);
        h.release();
    }

    /// The cursor during the gestures (lot C-01): the closed hand on a title bar, the Sherlock
    /// cursor while the gesture key and the button are held without moving, the four arrows
    /// once the window moves with the gesture key.
    #[test]
    fn the_cursor_follows_the_gesture() {
        use compositor::cursor::Shape;
        let mut h = Harness::start();
        let (_shown, _decoration) = h.show_decorated();
        h.state.pointer_motion((300.0, 150.0).into(), 1);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Default));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        h.state.pointer_motion((310.0, 160.0).into(), 3);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Grabbing));
        h.release();
        // Gesture key and press on the window: Sherlock's cursor until it moves.
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Pressed, 4);
        h.state.pointer_motion((400.0, 300.0).into(), 5);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 6);
        assert_eq!(h.state.cursor_shape(), Some(Shape::Sherlock));
        h.state.pointer_motion((401.0, 301.0).into(), 7);
        assert_eq!(
            h.state.cursor_shape(),
            Some(Shape::Sherlock),
            "a click, not a drag"
        );
        h.state.pointer_motion((450.0, 320.0).into(), 8);
        assert_eq!(h.state.cursor_shape(), Some(Shape::Icon(CursorIcon::Move)));
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 9);
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Released, 10);
        assert_eq!(
            h.state.cursor_shape(),
            Some(Shape::Icon(CursorIcon::Default))
        );
    }

    /// Show the clicks (lot C-01): off by default; on, a ring leaves each press for 460 ms and
    /// the pointer sinks, then nothing is drawn any more.
    #[test]
    fn a_click_shown_leaves_a_ring() {
        let mut h = Harness::start();
        // The window appears at once: only the ring animates.
        h.state.set_window_animations(false);
        let _shown = h.show_window();
        settle(&mut h);
        assert!(!h.state.show_clicks());
        h.click(100.0, 100.0, 1);
        assert!(
            !animating(&h, h.state.now()),
            "nothing while the option is off"
        );
        h.state.set_show_clicks(true);
        h.click(100.0, 100.0, 2);
        let now = h.state.now();
        assert!(animating(&h, now), "the ring grows");
        let rings = |at: Duration| {
            let output = h.state.space.outputs().next().unwrap().clone();
            let view = h.state.views()[0].1;
            let items = compositor::render::items(&h.state, (&output, &view), None, false, at).0;
            items
                .iter()
                .filter(|i| {
                    matches!(i, compositor::render::Item::Shape { key, .. }
                    if matches!(key.part, compositor::damage::Part::Click(_)))
                })
                .count()
        };
        assert_eq!(rings(now), 1);
        let (press, sinking) = h.state.click_sink(now + Duration::from_millis(54));
        assert!(sinking && (press - 0.86).abs() < 1e-3);
        assert_eq!(rings(now + Duration::from_millis(470)), 0);
        std::thread::sleep(Duration::from_millis(470));
        h.state.refresh();
        assert!(!animating(&h, h.state.now()));
    }

    /// A menu of a window appears with a short fade, without moving (lot C-01).
    #[test]
    fn a_menu_of_a_window_appears_with_a_fade() {
        let mut h = Harness::start();
        // The window itself appears at once: only the menu fades.
        h.state.set_window_animations(false);
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals.iter().any(|g| g.1 == "wl_shm")
        });
        let compositor: WlCompositor = h.bind(&registry);
        let wm_base: XdgWmBase = h.bind(&registry);
        let shm: WlShm = h.bind(&registry);
        let buffer = |w: i32, ht: i32| {
            let file = shm_file((w * ht * 4) as u64);
            let pool = shm.create_pool(file.as_fd(), w * ht * 4, &qh, ());
            (
                pool.create_buffer(0, w, ht, w * 4, Format::Argb8888, &qh, ()),
                file,
            )
        };
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let _toplevel = xdg_surface.get_toplevel(&qh, ());
        surface.commit();
        h.run_until("the first configure", |c, _| !c.configures.is_empty());
        let (window_buffer, _window_file) = buffer(100, 80);
        surface.attach(Some(&window_buffer), 0, 0);
        surface.commit();
        h.run_until("the placement", |_, s| {
            location(s) == Some((350, 260).into())
        });

        let positioner = wm_base.create_positioner(&qh, ());
        positioner.set_size(60, 40);
        positioner.set_anchor_rect(10, 10, 1, 1);
        let menu = compositor.create_surface(&qh, ());
        let _popup = wm_base.get_xdg_surface(&menu, &qh, ()).get_popup(
            Some(&xdg_surface),
            &positioner,
            &qh,
            (),
        );
        menu.commit();
        settle(&mut h);
        let (menu_buffer, _menu_file) = buffer(60, 40);
        menu.attach(Some(&menu_buffer), 0, 0);
        menu.commit();
        settle(&mut h);
        let alpha = |h: &Harness| {
            let output = h.state.space.outputs().next().unwrap().clone();
            let view = h.state.views()[0].1;
            let now = h.state.now();
            let (items, animating) =
                compositor::render::items(&h.state, (&output, &view), None, false, now);
            let alpha = items.iter().find_map(|item| match item {
                compositor::render::Item::Surface { surface, alpha, .. }
                    if surface.id().protocol_id() == menu.id().protocol_id() =>
                {
                    Some(*alpha)
                }
                _ => None,
            });
            (alpha.expect("the menu is drawn"), animating)
        };
        let (first, animating) = alpha(&h);
        assert!(first < 1.0, "it fades in: {first}");
        assert!(animating);
        std::thread::sleep(sherlock_ui::motion::MENU_FADE);
        let (shown, animating) = alpha(&h);
        assert_eq!(shown, 1.0);
        assert!(!animating);
    }

    /// With the gesture key, a press on an open panel of the bar stays with the panel: only
    /// the bar itself lets it through to the window below (review of C-01a).
    #[test]
    fn the_gesture_key_does_not_go_through_a_panel_of_the_bar() {
        let mut h = Harness::start();
        let (_shown, _decoration) = h.show_decorated();
        let bar = show_bar(&mut h);
        let with = panels(&mut h);
        let qh = h.queue.handle();
        // A panel of 200 × 150 below the bar, from 473 to 673 and 46 to 196: over the window.
        let positioner = with.wm_base.create_positioner(&qh, ());
        positioner.set_size(200, 150);
        positioner.set_anchor_rect(560, 0, 2, 36);
        positioner.set_anchor(xdg_positioner::Anchor::Bottom);
        positioner.set_gravity(xdg_positioner::Gravity::Bottom);
        let surface = with.compositor.create_surface(&qh, ());
        let popup =
            with.wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_popup(None, &positioner, &qh, ());
        bar.layer.get_popup(&popup);
        surface.commit();
        settle(&mut h);
        let file = shm_file(200 * 150 * 4);
        let pool = bar.shm.create_pool(file.as_fd(), 200 * 150 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 200, 150, 800, Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        settle(&mut h);
        let window = h.state.space.elements().next().unwrap().clone();
        let before = h.state.space.element_geometry(&window);
        // Gesture key + drag on the panel: the window under it does not move.
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Pressed, 1);
        h.state.pointer_motion((580.0, 150.0).into(), 2);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 3);
        assert!(!h.state.gesture_active(), "the panel keeps the press");
        h.state.pointer_motion((500.0, 300.0).into(), 4);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 5);
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Released, 6);
        assert_eq!(h.state.space.element_geometry(&window), before);
        drop(file);
    }

    /// Lot D-21e: the front-to-back rank of what draws `surface` at `at`, and where: its
    /// origin, or the corner of its window.
    fn drawn_at(
        s: &State,
        surface: &ServerSurface,
        at: Duration,
    ) -> Option<(usize, Point<i32, Physical>)> {
        let output = s.space.outputs().next().unwrap().clone();
        let view = s.views()[0].1;
        let (items, _) = compositor::render::items(s, (&output, &view), None, false, at);
        items.iter().enumerate().find_map(|(i, item)| match item {
            Item::Surface {
                surface: drawn,
                origin,
                ..
            } if drawn == surface => Some((i, *origin)),
            Item::Window {
                surface: drawn,
                rect,
                ..
            } if drawn == surface => Some((i, rect.loc)),
            _ => None,
        })
    }

    /// `front` is drawn in front of `back` at `at`.
    fn in_front(s: &State, front: &ServerSurface, back: &ServerSurface, at: Duration) -> bool {
        match (drawn_at(s, front, at), drawn_at(s, back, at)) {
            (Some((front, _)), Some((back, _))) => front < back,
            _ => false,
        }
    }

    /// The surface that has the pointer at (`x`, `y`).
    fn pointed(h: &mut Harness, x: f64, y: f64, time: u32) -> Option<ServerSurface> {
        h.state.pointer_motion((x, y).into(), time);
        h.state.seat.get_pointer().unwrap().current_focus()
    }

    /// The bar, then a window made fullscreen and drawn at the size of the screen: the bar and
    /// the window as the compositor knows them.
    fn fullscreen_under_bar(h: &mut Harness) -> (Anchored, Shown, ServerSurface, ServerSurface) {
        let shown = h.show_window();
        let bar = show_bar(h);
        let output = h.state.space.outputs().next().unwrap().clone();
        let bar_surface = layer_map_for_output(&output)
            .layers()
            .next()
            .unwrap()
            .wl_surface()
            .clone();
        let window = h.state.space.elements().next().unwrap().clone();
        let window = window.wl_surface().unwrap().into_owned();
        // Not fullscreen yet: the bar has the pointer over its row, and is drawn in front.
        assert_eq!(pointed(h, 400.0, 20.0, 1), Some(bar_surface.clone()));
        assert!(in_front(&h.state, &bar_surface, &window, h.state.now()));
        shown.toplevel.set_fullscreen(None);
        h.wait_size("fullscreen", 800, 600);
        h.redraw(&shown, 800, 600);
        (bar, shown, bar_surface, window)
    }

    /// Lot D-21e: a fullscreen window covers the bar, which is on the top layer, as on macOS;
    /// the overlay layer (on-screen displays, the lock screen) stays in front of it.
    #[test]
    fn a_fullscreen_window_covers_the_bar_but_not_the_overlay() {
        let mut h = Harness::start();
        let (_bar, _shown, bar, window) = fullscreen_under_bar(&mut h);
        let _osd = anchor(
            &mut h,
            Spec {
                layer: Layer::Overlay,
                anchor: Anchor::Bottom,
                size: (200, 60),
                zone: 0,
                margin: (0, 0, 40, 0),
                keyboard: KeyboardInteractivity::None,
            },
        );
        let output = h.state.space.outputs().next().unwrap().clone();
        let osd = layer_map_for_output(&output)
            .layers_on(ServerLayer::Overlay)
            .next()
            .unwrap()
            .wl_surface()
            .clone();
        let now = h.state.now();
        assert!(
            in_front(&h.state, &window, &bar, now),
            "the window over the bar"
        );
        assert!(
            in_front(&h.state, &osd, &window, now),
            "the overlay over the window"
        );
        // The row of the bar belongs to the window, the pointer passing by it too.
        assert_eq!(pointed(&mut h, 400.0, 20.0, 2), Some(window.clone()));
        assert_eq!(pointed(&mut h, 400.0, 0.0, 3), Some(window.clone()));
        settle(&mut h);
        assert!(in_front(&h.state, &window, &bar, h.state.now()));
        // Out of fullscreen, the bar is in front again.
        let shown = &_shown;
        shown.toplevel.unset_fullscreen();
        h.run_until("out of fullscreen", |c, _| {
            !c.configures
                .last()
                .unwrap()
                .2
                .contains(&XdgState::Fullscreen)
        });
        assert!(in_front(&h.state, &bar, &window, h.state.now()));
        assert_eq!(pointed(&mut h, 400.0, 20.0, 4), Some(bar));
    }

    /// Lot D-21e: the pointer resting against the top edge for 150 ms brings the bar down over
    /// the fullscreen window, sliding in from above; it then has the pointer.
    #[test]
    fn the_bar_comes_over_a_fullscreen_window_after_a_rest_at_the_top() {
        let mut h = Harness::start();
        let (_bar, _shown, bar, window) = fullscreen_under_bar(&mut h);
        let rest = h.state.now();
        assert_eq!(pointed(&mut h, 400.0, 0.0, 2), Some(window.clone()));
        settle(&mut h);
        assert!(
            in_front(&h.state, &window, &bar, h.state.now()),
            "not at once"
        );
        h.run_until("the bar revealed", |_, s| {
            in_front(s, &bar, &window, s.now())
        });
        assert!(h.state.now() >= rest + Duration::from_millis(150));
        // It slides down from above the screen to its place, 10 below the edge.
        let now = h.state.now();
        let (_, start) = drawn_at(&h.state, &bar, now).unwrap();
        assert!(start.y < 10, "{start:?}");
        assert!(animating(&h, now));
        let (_, end) = drawn_at(&h.state, &bar, now + Duration::from_millis(300)).unwrap();
        assert_eq!((end.x, end.y), (12, 10));
        // Down from the edge, the pointer reaches it, and it stays.
        assert_eq!(pointed(&mut h, 400.0, 20.0, 3), Some(bar.clone()));
        settle(&mut h);
        assert!(in_front(&h.state, &bar, &window, h.state.now()));
    }

    /// Lot D-21e: the bar goes back up once the pointer leaves it, but not while one of its
    /// panels is open: that panel takes the pointer, over the fullscreen window.
    #[test]
    fn the_bar_goes_back_behind_once_the_pointer_leaves_it_and_its_panels() {
        let mut h = Harness::start();
        let (bar_client, _shown, bar, window) = fullscreen_under_bar(&mut h);
        let reveal = |h: &mut Harness, time: u32| {
            pointed(h, 400.0, 0.0, time);
            h.run_until("the bar revealed", |_, s| {
                in_front(s, &bar, &window, s.now())
            });
        };
        reveal(&mut h, 2);
        // Below the bar: it slides back up, then is behind the window again.
        assert_eq!(pointed(&mut h, 400.0, 300.0, 3), Some(window.clone()));
        settle(&mut h);
        let now = h.state.now();
        assert!(animating(&h, now), "it slides out");
        h.run_until("the bar behind the window", |_, s| {
            in_front(s, &window, &bar, s.now())
        });
        assert_eq!(pointed(&mut h, 400.0, 20.0, 4), Some(window.clone()));

        // Revealed again, it opens a panel of 200 × 150 below its right end.
        reveal(&mut h, 5);
        let with = panels(&mut h);
        let qh = h.queue.handle();
        let positioner = with.wm_base.create_positioner(&qh, ());
        positioner.set_size(200, 150);
        positioner.set_anchor_rect(560, 0, 2, 36);
        positioner.set_anchor(xdg_positioner::Anchor::Bottom);
        positioner.set_gravity(xdg_positioner::Gravity::Bottom);
        let surface = with.compositor.create_surface(&qh, ());
        let popup =
            with.wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_popup(None, &positioner, &qh, ());
        bar_client.layer.get_popup(&popup);
        surface.commit();
        settle(&mut h);
        let file = shm_file(200 * 150 * 4);
        let pool = bar_client
            .shm
            .create_pool(file.as_fd(), 200 * 150 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 200, 150, 800, Format::Argb8888, &qh, ());
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        settle(&mut h);
        // The pointer on the panel, then away from both: the bar stays.
        let panel = pointed(&mut h, 580.0, 150.0, 6);
        assert!(panel.is_some() && panel != Some(window.clone()) && panel != Some(bar.clone()));
        pointed(&mut h, 300.0, 400.0, 7);
        settle(&mut h);
        assert!(
            in_front(&h.state, &bar, &window, h.state.now()),
            "a panel is open"
        );
        // The panel closed, the bar goes.
        popup.destroy();
        surface.destroy();
        settle(&mut h);
        h.run_until("the bar behind the window", |_, s| {
            in_front(s, &window, &bar, s.now())
        });
        drop(file);
    }

    /// A window whose client took its content away is not drawn: it takes no click either
    /// (review of C-01a: an X11 window before its first buffer).
    #[test]
    fn a_window_without_content_takes_no_click() {
        let mut h = Harness::start();
        let shown = h.show_window();
        shown.surface.attach(None, 0, 0);
        shown.surface.commit();
        settle(&mut h);
        h.client.button_serial = None;
        h.state.pointer_motion((400.0, 300.0).into(), 1);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        h.release();
        settle(&mut h);
        assert_eq!(h.client.button_serial, None, "the click reaches nothing");
        assert!(!h.state.gesture_active());
    }

    /// The gesture key and Space start the launcher, again to close it (a second launcher
    /// closes the first), and do not open Mission Control when the gesture key is released
    /// (lot C-01).
    #[test]
    fn the_gesture_key_and_space_start_the_launcher() {
        use std::os::unix::fs::PermissionsExt as _;
        let mut h = Harness::start();
        let dir = std::env::temp_dir().join(format!("launcher-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("started");
        let program = dir.join("launcher");
        let script = format!("#!/bin/sh\necho started >> '{}'\n", marker.display());
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        h.state.set_launcher_program(program);
        let started = |n: usize| {
            for _ in 0..500 {
                let lines = std::fs::read_to_string(&marker).map_or(0, |t| t.lines().count());
                if lines >= n {
                    return lines;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            0
        };
        h.shortcut(KEY_SPACE);
        assert_eq!(started(1), 1, "the launcher is started");
        assert!(!h.state.overview_open(), "the gesture key was not alone");
        h.shortcut(KEY_SPACE);
        assert_eq!(started(2), 2, "started again, it closes the open one");
        // Space alone goes to the client.
        h.state
            .keyboard_key(Keycode::new(KEY_SPACE), KeyState::Pressed, 9);
        h.state
            .keyboard_key(Keycode::new(KEY_SPACE), KeyState::Released, 10);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(started(2), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The xkb code of the key that types a comma in the layout of the harness, the machine's:
    /// its own key in US (59), the key right of N in AZERTY (58).
    fn comma_key() -> u32 {
        use smithay::input::keyboard::xkb;
        let layout = compositor::keyboard::Layout::system();
        let names = layout.xkb();
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            names.rules,
            names.model,
            names.layout,
            names.variant,
            names.options.clone(),
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("the keymap of the machine");
        (8..256)
            .find(|&code| {
                keymap
                    .key_get_syms_by_level(xkb::Keycode::new(code), 0, 0)
                    .contains(&xkb::Keysym::comma)
            })
            .expect("a key that types a comma")
    }

    /// The gesture key and comma start Control Room, as « Réglages » in the bar's Sherlock menu
    /// does; comma alone goes to the client (lot Q-31).
    #[test]
    fn the_gesture_key_and_comma_start_control_room() {
        use std::os::unix::fs::PermissionsExt as _;
        let mut h = Harness::start();
        let dir = std::env::temp_dir().join(format!("settings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("started");
        let program = dir.join("control-room");
        let script = format!("#!/bin/sh\necho started >> '{}'\n", marker.display());
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        h.state.set_settings_program(program);
        let started = |n: usize| {
            for _ in 0..500 {
                let lines = std::fs::read_to_string(&marker).map_or(0, |t| t.lines().count());
                if lines >= n {
                    return lines;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            0
        };
        let comma = comma_key();
        h.shortcut(comma);
        assert_eq!(started(1), 1, "Control Room is started");
        assert!(!h.state.overview_open(), "the gesture key was not alone");
        h.state
            .keyboard_key(Keycode::new(comma), KeyState::Pressed, 9);
        h.state
            .keyboard_key(Keycode::new(comma), KeyState::Released, 10);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(started(1), 1, "comma alone starts nothing");
        std::fs::remove_dir_all(&dir).unwrap();
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

    #[test]
    fn a_click_without_moving_reaches_a_window_back_from_its_icon() {
        let mut h = Harness::start();
        let _shown = h.show_window();
        let window = h.state.space.elements().next().unwrap().clone();
        h.state.pointer_motion((400.0, 300.0).into(), 1);
        h.state.minimize(&window);
        h.run_until("the window in its icon", |_, s| {
            s.minimize_progress(&window, s.now()).is_none()
        });
        // Restored under the still pointer: during its effect, the pointer does not reach it.
        h.state.activate(&window);
        assert!(h.state.minimize_progress(&window, h.state.now()).is_some());
        h.run_until("the window back", |_, s| {
            s.minimize_progress(&window, s.now()).is_none()
        });
        // The loop iteration after the last frame of the effect.
        h.state.refresh();
        h.client.button_serial = None;
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        h.run_until("the press on the window", |c, _| c.button_serial.is_some());
    }

    /// Whether the screen animates at `at`: a frame is due at the next blank.
    fn animating(h: &Harness, at: Duration) -> bool {
        let output = h.state.space.outputs().next().unwrap().clone();
        let view = h.state.views()[0].1;
        compositor::render::items(&h.state, (&output, &view), None, false, at).1
    }

    /// Two windows shown at the same place, at rest: the one below, then the one above.
    fn two_windows_at_rest(h: &mut Harness) -> (Shown, Shown, Window, Window) {
        // Both appear at once: only the raise animates below.
        h.state.set_window_animations(false);
        let below = h.show_window();
        let above = h.show_window();
        let centered = |s: &State| {
            let at = |w| s.space.element_location(w) == Some((350, 260).into());
            s.space.elements().filter(|w| at(w)).count() == 2
        };
        h.run_until("both windows", |_, s| centered(s));
        assert!(!animating(h, h.state.now()));
        h.state.set_window_animations(true);
        let windows: Vec<Window> = h.state.space.elements().cloned().collect();
        let (low, high) = (windows[0].clone(), windows[1].clone());
        (below, above, low, high)
    }

    /// Scale and shadow share of `window` drawn at `at`.
    fn lift(h: &Harness, window: &Window, at: Duration) -> (f64, f64) {
        let visual = h.state.window_visual(window, at).unwrap();
        (visual.scale, visual.shadow)
    }

    fn has_keyboard(h: &Harness, window: &Window) -> bool {
        use compositor::focus::KeyboardFocus;
        let focus = h.state.seat.get_keyboard().unwrap().current_focus();
        let surface = window.toplevel().unwrap().wl_surface().clone();
        focus == Some(KeyboardFocus::Wayland(surface))
    }

    /// Raise (lot D-19d). The window below, covered by the other one, is activated: it comes to
    /// the front at once, with the keyboard, and grows from `RAISE_SCALE` to 1, its shadow with
    /// it, while the other one steps back and comes back; then nothing is drawn at rest.
    #[test]
    fn a_window_raised_from_behind_grows_and_the_other_steps_back() {
        use sherlock_ui::motion;
        let mut h = Harness::start();
        let (_below, _above, low, high) = two_windows_at_rest(&mut h);
        let start = h.state.now();
        h.state.activate(&low);
        let at = |ms: u64| start + Duration::from_millis(ms);
        // In front, with the keyboard, at once.
        assert_eq!(h.state.space.elements().last(), Some(&low));
        assert!(has_keyboard(&h, &low));
        let (scale, shadow) = lift(&h, &low, at(0));
        assert!(scale < 0.975 && shadow < 0.7, "{scale} {shadow}");
        assert!(animating(&h, at(0)));
        // Halfway: the raised one grows, the other one is at its furthest step back.
        let (scale, shadow) = lift(&h, &low, at(100));
        assert!(scale > 0.975 && scale < 1.0 && shadow > 0.7 && shadow < 1.0);
        let (back, _) = lift(&h, &high, at(100));
        assert!((back - motion::RECEDE_SCALE).abs() < 1e-3, "{back}");
        // Then both at rest, and no frame.
        assert_eq!(lift(&h, &low, at(400)), (1.0, 1.0));
        assert_eq!(lift(&h, &high, at(400)), (1.0, 1.0));
        assert!(!animating(&h, at(400)));
    }

    /// No raise animation for a window that nothing covers, nor with window animations off.
    #[test]
    fn a_window_fully_visible_or_animations_off_comes_to_the_front_without_animation() {
        let mut h = Harness::start();
        let (_below, _above, low, high) = two_windows_at_rest(&mut h);
        // Side by side: the one below is fully visible.
        h.state.space.map_element(high.clone(), (0, 0), false);
        let now = h.state.now();
        h.state.activate(&low);
        assert_eq!(h.state.space.elements().last(), Some(&low));
        assert_eq!(lift(&h, &low, now), (1.0, 1.0));
        assert_eq!(lift(&h, &high, now), (1.0, 1.0));
        assert!(!animating(&h, now));
        // Covered again, but animations off.
        h.state.space.map_element(high.clone(), (350, 260), true);
        h.state.set_window_animations(false);
        h.state.activate(&low);
        assert_eq!(h.state.space.elements().last(), Some(&low));
        assert!(!animating(&h, h.state.now()));
    }

    /// Raised, then the other window raised back before the end: each one goes on from where
    /// it is drawn, without a jump, and both end at rest.
    #[test]
    fn a_raise_interrupted_halfway_goes_on_without_a_jump() {
        let mut h = Harness::start();
        let (_below, _above, low, high) = two_windows_at_rest(&mut h);
        let start = h.state.now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        h.state.activate(&low);
        let (low_before, high_before) = (lift(&h, &low, at(60)).0, lift(&h, &high, at(60)).0);
        // `activate` reads the compositor's clock: the turn happens a little after 60 ms.
        while h.state.now() < at(60) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let turn = h.state.now();
        h.state.activate(&high);
        assert_eq!(h.state.space.elements().last(), Some(&high));
        assert!(has_keyboard(&h, &high));
        let (low_after, high_after) = (lift(&h, &low, turn).0, lift(&h, &high, turn).0);
        // Within what a few milliseconds of motion give: no jump to `RAISE_SCALE`.
        assert!(
            (low_after - low_before).abs() < 0.01,
            "{low_before} {low_after}"
        );
        assert!(
            (high_after - high_before).abs() < 0.01,
            "{high_before} {high_after}"
        );
        assert!(animating(&h, turn));
        let end = turn + Duration::from_millis(400);
        assert_eq!(lift(&h, &low, end), (1.0, 1.0));
        assert_eq!(lift(&h, &high, end), (1.0, 1.0));
        assert!(!animating(&h, end));
    }

    /// Opening and closing (lot D-34). A window closes while another one is under it: its last
    /// image shrinks and fades for 150 ms, out of reach, then it is forgotten.
    #[test]
    fn a_closing_window_takes_no_click_and_is_freed_at_the_end() {
        let mut h = Harness::start();
        // Both windows appear at once: only the closing animates below.
        h.state.set_window_animations(false);
        let _below = h.show_window();
        let above = h.show_window();
        let centered = |s: &State| {
            let at = |w| s.space.element_location(w) == Some((350, 260).into());
            s.space.elements().filter(|w| at(w)).count() == 2
        };
        h.run_until("both windows", |_, s| centered(s));
        assert!(!animating(&h, h.state.now()));
        h.state.set_window_animations(true);
        assert!(h.state.window_animations());

        above.toplevel.destroy();
        h.run_until("the window gone", |_, s| s.space.elements().count() == 1);
        // No renderer here: the closing runs without a texture.
        assert_eq!(h.state.closing_windows(), (1, 0));
        let now = h.state.now();
        assert!(animating(&h, now), "a frame at each blank while it closes");
        // The click at its place reaches the window below.
        h.press(400.0, 300.0);
        h.release();
        // Over: no frame any more, and nothing is kept.
        assert!(!animating(&h, now + Duration::from_millis(190)));
        std::thread::sleep(Duration::from_millis(190));
        h.state.refresh();
        assert_eq!(h.state.closing_windows(), (0, 0));
    }

    #[test]
    fn a_fullscreen_window_opens_and_closes_at_once() {
        let mut h = Harness::start();
        let shown = h.show_window();
        shown.toplevel.set_fullscreen(None);
        h.wait_size("fullscreen", 800, 600);
        let _file = h.redraw(&shown, 800, 600);
        let window = h.state.space.elements().next().unwrap().clone();
        let first = h.state.window_visual(&window, h.state.now()).unwrap();
        assert_eq!(first.opacity, 1.0);
        assert_eq!(first.rect, rect(0, 0, 800, 600).to_f64());
        shown.toplevel.destroy();
        h.run_until("the window gone", |_, s| s.space.elements().count() == 0);
        assert_eq!(h.state.closing_windows(), (0, 0));
    }

    #[test]
    fn with_window_animations_off_windows_open_and_close_at_once() {
        let mut h = Harness::start();
        h.state.set_window_animations(false);
        assert!(!h.state.window_animations());
        let shown = h.show_window();
        let (drawn, real) = h.drawn(0);
        assert_eq!(drawn, real);
        let window = h.state.space.elements().next().unwrap().clone();
        assert_eq!(
            h.state
                .window_visual(&window, h.state.now())
                .unwrap()
                .opacity,
            1.0
        );
        assert!(!animating(&h, h.state.now()));
        shown.toplevel.destroy();
        h.run_until("the window gone", |_, s| s.space.elements().count() == 0);
        assert_eq!(h.state.closing_windows(), (0, 0));
        assert!(!animating(&h, h.state.now()));
    }

    /// The opening and the closing drawn for real, without a screen, by Mesa's software
    /// renderer (llvmpipe), as the genie is: a decorated window halfway through its opening,
    /// then its last image halfway through its closing, drawn from the texture kept when its
    /// client destroyed it. The images go to Cargo's temporary folder of the integration tests
    /// (`target/tmp`): `window-opening.png`, `window-closing.png`. Without Mesa's EGL the test
    /// has nothing to draw with: it says so, and fails on the CI, which installs it.
    #[test]
    fn the_opening_and_the_closing_are_drawn_halfway() {
        use compositor::{damage::Ids, render::Scene, wallpaper};
        use sherlock_ui::color;
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
        use std::{cell::RefCell, rc::Rc};

        let software = EGLDevice::enumerate()
            .ok()
            .and_then(|mut d| d.find(EGLDevice::is_software));
        let Some(device) = software else {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI has Mesa's EGL (libegl-mesa0): its software device must be found"
            );
            eprintln!("no software EGL device (Mesa's llvmpipe): nothing is drawn");
            return;
        };
        // SAFETY: the display lives as long as the renderer, in this test only.
        let display = unsafe { EGLDisplay::new(device) }.unwrap();
        let context = EGLContext::new(&display).unwrap();
        // SAFETY: the context was just made and is used by this renderer alone.
        let renderer = unsafe { GlesRenderer::new(context) }.unwrap();
        let gpu = Rc::new(RefCell::new(renderer));
        let scene = Rc::new(Scene::new(&mut gpu.borrow_mut()).unwrap());
        let (w, ht) = (800, 600);
        // The scene at `at`, read back as rows from the top.
        let shoot = |state: &mut State, at: Duration| {
            let mut renderer = gpu.borrow_mut();
            let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());
            let views = state.views();
            let drawn = scene.screens(
                &mut renderer,
                &mut ids,
                state,
                &mut textures,
                &views,
                None,
                at,
            );
            let elements = drawn.0.into_iter().next().unwrap();
            let mut texture: GlesTexture = renderer
                .create_buffer(Fourcc::Abgr8888, (w, ht).into())
                .unwrap();
            let mut target = renderer.bind(&mut texture).unwrap();
            let mut tracker = OutputDamageTracker::new((w, ht), 1.0, Transform::Normal);
            let background = color::palette().desktop.premultiplied();
            tracker
                .render_output(&mut *renderer, &mut target, 0, &elements, background)
                .unwrap();
            let region = Rectangle::from_size((w, ht).into());
            let mapping = renderer
                .copy_framebuffer(&target, region, Fourcc::Abgr8888)
                .unwrap();
            let pixels = renderer.map_texture(&mapping).unwrap().to_vec();
            match mapping.flipped() {
                true => pixels,
                false => pixels
                    .chunks(w as usize * 4)
                    .rev()
                    .flatten()
                    .copied()
                    .collect(),
            }
        };
        let save = |name: &str, pixels: &[u8]| {
            let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
            image::save_buffer(&path, pixels, w as u32, ht as u32, image::ColorType::Rgba8)
                .unwrap();
            eprintln!("{name}: {}", path.display());
        };
        let at =
            |pixels: &[u8], x: usize, y: usize| pixels[(y * w as usize + x) * 4..][..3].to_vec();

        let mut h = Harness::start();
        let (gpu_, scene_) = (Rc::clone(&gpu), Rc::clone(&scene));
        h.state.set_snapshotter(Box::new(move |state, window| {
            let mut renderer = gpu_.try_borrow_mut().ok()?;
            compositor::window_anim::snapshot(&scene_, &mut renderer, state, window)
        }));
        // Six steps side by side, cut around the window and its shadow: the boards of the
        // opening and of the closing (lot C-01).
        let (cx, cy, cw, ch, gap) = (160usize, 99usize, 480usize, 400usize, 8usize);
        let sheet = |name: &str, frames: &[Vec<u8>]| {
            let width = frames.len() * cw + (frames.len() - 1) * gap;
            let mut out = vec![0u8; width * ch * 4];
            for (i, frame) in frames.iter().enumerate() {
                for y in 0..ch {
                    let src = ((cy + y) * w as usize + cx) * 4;
                    let dst = (y * width + i * (cw + gap)) * 4;
                    out[dst..dst + cw * 4].copy_from_slice(&frame[src..src + cw * 4]);
                }
            }
            let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
            image::save_buffer(
                &path,
                &out,
                width as u32,
                ch as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            eprintln!("{name}: {}", path.display());
        };
        // The first row of the bar's color in column `x`, from the top: where the window
        // starts.
        let top = |pixels: &[u8], x: usize| {
            let bar = color::palette().title_bar;
            (0..ht as usize).find(|y| at(pixels, x, *y) == [bar.r, bar.g, bar.b])
        };
        let fill = color::palette().window;
        let window = [fill.r, fill.g, fill.b].to_vec();
        let (shown, _decoration) = h.show_decorated_in(fill);
        // The whole window is at (200, 129), 400 × 342 with its bar.
        let start = h.state.now();
        let opening: Vec<Vec<u8>> = [0, 50, 100, 150, 200, 250]
            .map(|t| shoot(&mut h.state, start + Duration::from_millis(t)))
            .to_vec();
        sheet("window-opening-steps.png", &opening);
        save("window-opening.png", &opening[1]);
        let open = shoot(&mut h.state, start + Duration::from_secs(1));
        assert_eq!(at(&open, 400, 400), window);
        // The bar comes down to its place with the window, and the window grows: never past
        // its place, without a bounce.
        let tops: Vec<usize> = opening[1..]
            .iter()
            .map(|f| top(f, 400).unwrap_or(ht as usize))
            .collect();
        assert!(tops.windows(2).all(|t| t[1] <= t[0]), "{tops:?}");
        // Under its border of 1 at 130 once open; 8 lower at 50 ms (scale 0.95).
        assert!(tops[0] > 130 + 5, "it starts smaller: {tops:?}");
        assert_eq!(tops[4], 130, "{tops:?}");
        // Halfway: smaller, its edge inside its place, and still translucent.
        assert_ne!(at(&opening[1], 203, 400), at(&open, 203, 400));
        assert_ne!(at(&opening[1], 400, 400), window);
        assert_eq!(at(&opening[5], 400, 400), window, "opaque at 250 ms");

        shown.toplevel.destroy();
        h.run_until("the window gone", |_, s| s.space.elements().count() == 0);
        assert_eq!(h.state.closing_windows(), (1, 1), "its last image is kept");
        let closed = h.state.now();
        let closing: Vec<Vec<u8>> = [0, 36, 72, 108, 144, 180]
            .map(|t| shoot(&mut h.state, closed + Duration::from_millis(t)))
            .to_vec();
        sheet("window-closing-steps.png", &closing);
        save("window-closing.png", &closing[2]);
        let gone = &closing[5];
        assert_eq!(at(&closing[0], 400, 400), window, "it starts as it was");
        assert_ne!(at(&closing[2], 400, 400), at(gone, 400, 400));
        assert_ne!(at(&closing[2], 203, 400), at(&open, 203, 400));
        // Shrunk, not cut: at 108 ms (scale 0.971) its bottom right corner is still rounded,
        // 3 pixels inside it is not the window.
        assert_ne!(at(&closing[3], 591, 463), at(&closing[3], 400, 400));
        // Gone at 180 ms; and the first frame of the opening showed nothing yet, as without
        // the window: no flash of the whole window.
        for (x, y) in [(400, 400), (203, 150), (400, 135)] {
            assert_eq!(at(gone, x, y), at(&opening[0], x, y), "({x}, {y})");
        }
        std::thread::sleep(Duration::from_millis(200));
        h.state.refresh();
        assert_eq!(h.state.closing_windows(), (0, 0), "its texture is freed");
    }

    /// Board of the raise (lot D-19d), dark: two decorated windows that overlap, the one below
    /// activated, drawn with Mesa's software renderer at six steps, 0 to 250 ms. In Cargo's
    /// temporary folder of the integration tests: `target/tmp/raise-board.png`, three by two.
    #[test]
    #[ignore = "needs Mesa's EGL; writes the board: run it with --ignored"]
    fn board_of_the_raise() {
        use compositor::{damage::Ids, render::Scene, wallpaper};
        use sherlock_ui::color::{Palette, Theme};
        use smithay::backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::gles::GlesRenderer,
        };

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
        let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());
        sherlock_ui::color::set_active(Theme::Dark);
        let mut h = Harness::start();
        // Both appear at once, then are placed across each other.
        h.state.set_window_animations(false);
        let (_low, _) = h.show_decorated_in(Palette::DARK.window);
        let (_high, _) = h.show_decorated_in(Palette::DARK.card);
        h.run_until("both windows", |_, s| s.space.elements().count() == 2);
        let windows: Vec<Window> = h.state.space.elements().cloned().collect();
        let (low, high) = (windows[0].clone(), windows[1].clone());
        h.state.space.map_element(low.clone(), (110, 120), false);
        h.state.space.map_element(high.clone(), (290, 230), true);
        let size = (800, 600);
        let pass = (&scene, &mut ids, &mut textures);
        software_frame(&mut renderer, pass, &mut h, size);
        h.state.set_window_animations(true);
        let start = h.state.now();
        h.state.activate(&low);
        let frames: Vec<Vec<u8>> = [0, 40, 80, 120, 170, 250]
            .iter()
            .map(|ms| {
                let at = start + Duration::from_millis(*ms);
                let pass = (&scene, &mut ids, &mut textures);
                software_frame_at(&mut renderer, pass, &mut h, size, at)
            })
            .collect();
        // The first image is smaller than the last around the raised window: at its title bar's
        // top, near its left end, the desktop shows first, the bar at the end.
        let at = |f: &[u8], x: usize, y: usize| f[(y * 800 + x) * 4..][..3].to_vec();
        assert_ne!(at(&frames[0], 130, 81), at(&frames[5], 130, 81));
        let (w, ht) = (800u32, 600u32);
        let mut board = image::RgbaImage::new(w * 3, ht * 2);
        for (i, frame) in frames.iter().enumerate() {
            let tile = image::RgbaImage::from_raw(w, ht, frame.clone()).unwrap();
            let (x, y) = ((i as u32 % 3) * w, (i as u32 / 3) * ht);
            image::imageops::replace(&mut board, &tile, i64::from(x), i64::from(y));
        }
        let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("raise-board.png");
        board.save(&path).unwrap();
        eprintln!("raise board: {}", path.display());
    }

    /// Captures of a decorated window, dark then light: at rest, then with the pointer on each
    /// button, once its hover has settled. The scene of a screen, as the real session draws
    /// it, but into a texture with Mesa's software renderer. They go to `screenshots/`
    /// (`decorations-<theme>.png`, `decorations-<theme>-<button>.png`), cut around the window.
    #[test]
    #[ignore = "needs Mesa's EGL; writes the captures: run it with --ignored"]
    fn captures_of_a_decorated_window() {
        use compositor::{damage::Ids, render::Scene, wallpaper};
        use sherlock_ui::color::{self, Theme};
        use smithay::backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::gles::GlesRenderer,
        };

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
        let (w, ht) = (800, 600);
        // The window, 400 × 342 at (200, 129), and its shadow: 480 × 400 from (160, 99).
        let (cx, cy, cw, ch) = (160usize, 99usize, 480usize, 400usize);
        let save = |name: &str, pixels: &[u8]| {
            let mut cut = Vec::with_capacity(cw * ch * 4);
            for row in pixels.chunks(w as usize * 4).skip(cy).take(ch) {
                cut.extend_from_slice(&row[cx * 4..(cx + cw) * 4]);
            }
            let path = format!(
                "{}/screenshots/decorations-{name}.png",
                env!("CARGO_MANIFEST_DIR")
            );
            image::save_buffer(&path, &cut, cw as u32, ch as u32, image::ColorType::Rgba8).unwrap();
        };
        for (theme, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
            color::set_active(theme);
            let mut h = Harness::start();
            let (_shown, _decoration) = h.show_decorated_in(theme.palette().window);
            let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());
            // Once the opening is over, then each hover once settled: a second later.
            let mut later = 2;
            for (button, x) in [
                ("", 0.0),
                ("minimize", 505.0),
                ("maximize", 539.0),
                ("close", 573.0),
            ] {
                if x > 0.0 {
                    h.state.pointer_motion((x, 150.0).into(), later);
                }
                later += 1;
                // The first frame starts the opening, the second is a second later.
                let mut pixels = Vec::new();
                for step in [0, 1] {
                    let at = h.state.now() + Duration::from_secs(u64::from(later) + step);
                    let with = (&scene, &mut ids, &mut textures);
                    pixels = software_frame_at(&mut renderer, with, &mut h, (w, ht), at);
                }
                let at = |x: usize, y: usize| &pixels[(y * w as usize + x) * 4..][..3];
                let bar = color::palette().title_bar;
                // The bar has the color of its theme, under the title.
                assert_eq!(at(300, 140), [bar.r, bar.g, bar.b], "{name}");
                let suffix = if button.is_empty() {
                    name.to_owned()
                } else {
                    format!("{name}-{button}")
                };
                if button == "close" {
                    // Red under the pointer, around its icon.
                    let salmon = color::palette().salmon;
                    assert_eq!(at(562, 141), [salmon.r, salmon.g, salmon.b], "{name}");
                }
                save(&suffix, &pixels);
            }
        }
        color::set_active(Theme::Dark);
    }

    /// The desktop of `h` drawn into a texture of `w` × `ht` with the software renderer, at the
    /// compositor's time; its RGBA pixels, top row first.
    fn software_frame(
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        with: (
            &compositor::render::Scene,
            &mut compositor::damage::Ids,
            &mut compositor::wallpaper::Textures,
        ),
        h: &mut Harness,
        size: (i32, i32),
    ) -> Vec<u8> {
        let now = h.state.now();
        software_frame_at(renderer, with, h, size, now)
    }

    /// `software_frame` at `now`.
    fn software_frame_at(
        renderer: &mut smithay::backend::renderer::gles::GlesRenderer,
        (scene, ids, textures): (
            &compositor::render::Scene,
            &mut compositor::damage::Ids,
            &mut compositor::wallpaper::Textures,
        ),
        h: &mut Harness,
        (w, ht): (i32, i32),
        now: Duration,
    ) -> Vec<u8> {
        use smithay::backend::{
            allocator::Fourcc,
            renderer::{
                Bind as _, ExportMem as _, Offscreen as _, TextureMapping as _,
                damage::OutputDamageTracker, gles::GlesTexture,
            },
        };
        use smithay::utils::Transform;
        let views = h.state.views();
        let drawn = scene.screens(renderer, ids, &mut h.state, textures, &views, None, now);
        let elements = drawn.0.into_iter().next().unwrap();
        let mut texture: GlesTexture = renderer
            .create_buffer(Fourcc::Abgr8888, (w, ht).into())
            .unwrap();
        let mut target = renderer.bind(&mut texture).unwrap();
        let mut tracker = OutputDamageTracker::new((w, ht), 1.0, Transform::Normal);
        let background = sherlock_ui::color::palette().desktop.premultiplied();
        tracker
            .render_output(renderer, &mut target, 0, &elements, background)
            .unwrap();
        let region = Rectangle::from_size((w, ht).into());
        let mapping = renderer
            .copy_framebuffer(&target, region, Fourcc::Abgr8888)
            .unwrap();
        let pixels = renderer.map_texture(&mapping).unwrap().to_vec();
        if mapping.flipped() {
            return pixels;
        }
        // GL reads the rows from the bottom up.
        pixels
            .chunks(w as usize * 4)
            .rev()
            .flatten()
            .copied()
            .collect()
    }

    /// Captures of the desktop, dark then light, the change made by rewriting `Theme=` in the
    /// appearance file while everything runs: the default wallpaper and its pendant, a
    /// decorated window, and the real dock (`target/debug/dock`), hovered over Sherlock. The
    /// scene of a screen as the real session draws it, into a texture with Mesa's software
    /// renderer: `screenshots/desktop-dark.png`, then `desktop-light.png`.
    #[test]
    #[ignore = "needs Mesa's EGL and `cargo build -p dock`; writes the captures"]
    fn captures_of_the_desktop_in_both_themes() {
        use compositor::{damage::Ids, render::Scene, theme, wallpaper};
        use sherlock_ui::color::{self, Theme};
        use smithay::backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::gles::GlesRenderer,
        };
        use smithay::desktop::layer_map_for_output;
        use std::{path::PathBuf, process::Command};

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dock = root.join("target/debug/dock");
        assert!(dock.is_file(), "build the dock first: cargo build -p dock");
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("desktop-captures");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config/appearance")).unwrap();
        let file = dir.join("config/appearance/appearance.conf");
        let write_theme = |theme: &str| {
            let new = file.with_extension("new");
            std::fs::write(&new, format!("[Appearance]\nTheme={theme}\n")).unwrap();
            std::fs::rename(&new, &file).unwrap();
        };
        write_theme("dark");
        // The default pair, reduced from the originals to the screen.
        let data = dir.join("data");
        for id in ["first-clue", "first-clue-light"] {
            let original = root.join(format!("assets/wallpapers/{id}.jpg"));
            let image = image::open(&original).unwrap().thumbnail(1067, 600);
            let folder = data.join("wallpapers").join(id);
            std::fs::create_dir_all(&folder).unwrap();
            let size = format!("{}x{}.png", image.width(), image.height());
            image.to_rgb8().save(folder.join(size)).unwrap();
        }

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
        let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());

        let mut h = Harness::start();
        let handle = h.event_loop.handle();
        theme::follow_at(&handle, &mut h.state, file.clone());
        h.state.wallpaper =
            wallpaper::Wallpaper::start(&handle, None, vec![data.join("wallpapers")]).unwrap();
        let socket = dir.join("wayland-capture");
        h.state.listen_at(socket.clone(), &h.event_loop).unwrap();
        let fill = |theme: Theme| theme.palette().window;
        let (shown, _decoration) = h.show_decorated_in(fill(Theme::Dark));
        // The dock, on the same appearance file, never on the session's display. In software
        // rendering, its buffers have no alpha (softbuffer's `XRGB8888`, see "Layer-shell
        // toolkit of the shell"): the room around its glass comes out black.
        let mut child = Command::new(&dock)
            .env_clear()
            .env("XDG_RUNTIME_DIR", &dir)
            .env("WAYLAND_DISPLAY", "wayland-capture")
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("HOME", &dir)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("RUST_LOG", "warn")
            // Software rendering, as the compositor here: no GPU is asked anything.
            .env("ICED_BACKEND", "tiny-skia")
            .spawn()
            .unwrap();
        // Runs the compositor, the test client and the dock for `ms`, a frame every 16 ms;
        // returns the last frame.
        let mut run = |h: &mut Harness, ms: u64| {
            let end = std::time::Instant::now() + Duration::from_millis(ms);
            let mut frame = Vec::new();
            while std::time::Instant::now() < end {
                h.conn.flush().unwrap();
                h.event_loop
                    .dispatch(Some(Duration::from_millis(16)), &mut h.state)
                    .unwrap();
                h.state.refresh();
                if let Some(guard) = h.queue.prepare_read() {
                    let _ = guard.read();
                }
                h.queue.dispatch_pending(&mut h.client).unwrap();
                let pass = (&scene, &mut ids, &mut textures);
                frame = software_frame(&mut renderer, pass, h, (800, 600));
                let output = h.state.space.outputs().next().unwrap().clone();
                let time = h.state.now();
                for layer in layer_map_for_output(&output).layers() {
                    layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                        Some(output.clone())
                    });
                }
            }
            frame
        };
        run(&mut h, 3000);
        let output = h.state.space.outputs().next().unwrap().clone();
        let docked = !layer_map_for_output(&output)
            .layers()
            .collect::<Vec<_>>()
            .is_empty();
        assert!(docked, "the dock is anchored");
        // The pointer over Sherlock, in the middle of the dock: its label shows.
        h.state.pointer_motion((400.0, 568.0).into(), 1);
        run(&mut h, 1500);
        let save = |pixels: &[u8], name: &str| {
            let path = format!("{}/screenshots/{name}.png", env!("CARGO_MANIFEST_DIR"));
            image::save_buffer(&path, pixels, 800, 600, image::ColorType::Rgba8).unwrap();
        };
        let dark = run(&mut h, 16);
        save(&dark, "desktop-dark");

        // Light: the file changes, everything follows; the application redraws in its colors.
        write_theme("light");
        let at = |pixels: &[u8], x: usize, y: usize| pixels[(y * 800 + x) * 4..][..3].to_vec();
        run(&mut h, 3000);
        assert_eq!(h.state.theme(), Theme::Light);
        let mut file = shm_file(400 * 300 * 4);
        let light = fill(Theme::Light);
        file.write_all(&[light.b, light.g, light.r, 255].repeat(400 * 300))
            .unwrap();
        let qh = h.queue.handle();
        let pool = shown.shm.create_pool(file.as_fd(), 400 * 300 * 4, &qh, ());
        let buffer = pool.create_buffer(0, 400, 300, 1600, Format::Argb8888, &qh, ());
        shown.surface.attach(Some(&buffer), 0, 0);
        shown.surface.damage(0, 0, 400, 300);
        shown.surface.commit();
        let light = run(&mut h, 1000);
        save(&light, "desktop-light");
        let _ = child.kill();
        let _ = child.wait();
        h.state.shutdown();
        // The title bar took the light palette, and the wallpaper changed under the window.
        let bar = color::palette().title_bar;
        assert_eq!(at(&light, 400, 140), [bar.r, bar.g, bar.b]);
        assert_ne!(at(&dark, 40, 300), at(&light, 40, 300));
        color::set_active(Theme::Dark);
    }

    /// Lot UI-07: a client names its cursor the way iced_exwlshell does (a cursor shape device
    /// made for the request, the serial of the pointer's entry, the device destroyed at once),
    /// and the compositor shows it: the hand that points over a button, the open hand over a
    /// slider, the closed hand while the slider is held. A shape named with another serial is
    /// refused, as the protocol says. Over the dock, the arrow.
    #[test]
    fn the_compositor_shows_the_cursor_a_client_names() {
        let mut h = Harness::start();
        let shown = h.show_window();
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("the globals", |c, _| {
            c.globals
                .iter()
                .any(|g| g.1 == "wp_cursor_shape_manager_v1")
        });
        let manager: WpCursorShapeManagerV1 = h.bind(&registry);
        let pointer = shown.seat.get_pointer(&qh, ());
        h.run_until("the pointer", |_, _| true);
        h.client.enter_serial = None;
        h.state.pointer_motion((400.0, 300.0).into(), 1);
        h.run_until("the entry", |c, _| c.enter_serial.is_some());
        let serial = h.client.enter_serial.unwrap();
        let name = |h: &mut Harness, shape: CursorShape, serial: u32| {
            let device = manager.get_pointer(&pointer, &h.queue.handle(), ());
            device.set_shape(serial, shape);
            device.destroy();
            h.run_until("the shape", |_, _| true);
            h.run_until("the shape", |_, _| true);
        };
        // Over a button.
        name(&mut h, CursorShape::Pointer, serial);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Pointer));
        // The pointer moves on the button: the shape stays.
        h.state.pointer_motion((410.0, 305.0).into(), 2);
        h.run_until("the motion", |_, _| true);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Pointer));
        // Over a slider, then pressed on it.
        name(&mut h, CursorShape::Grab, serial);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Grab));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 3);
        name(&mut h, CursorShape::Grabbing, serial);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Grabbing));
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 4);
        // A stale serial changes nothing.
        name(&mut h, CursorShape::Text, serial.wrapping_sub(1));
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Grabbing));

        // The dock keeps the arrow, as on macOS: over an icon, pressed and dragged. Its icons
        // name no shape (`mouse_area` without `interaction`), which iced_exwlshell sends as
        // the default one at the pointer's entry.
        let _dock = anchor(&mut h, DOCK);
        h.client.enter_serial = None;
        h.state.pointer_motion((400.0, 560.0).into(), 5);
        h.run_until("the entry on the dock", |c, _| c.enter_serial.is_some());
        let serial = h.client.enter_serial.unwrap();
        name(&mut h, CursorShape::Default, serial);
        assert_eq!(
            h.state.cursor_icon(),
            Some(CursorIcon::Default),
            "over an icon"
        );
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 6);
        h.state.pointer_motion((440.0, 556.0).into(), 7);
        h.run_until("the drag", |_, _| true);
        assert_eq!(
            h.state.cursor_icon(),
            Some(CursorIcon::Default),
            "dragging an icon"
        );
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 8);
    }
}
