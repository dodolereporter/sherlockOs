//! Mission Control (lot D-33), without a screen or backend: a client opens decorated windows,
//! the view opens and closes by the keys and by three fingers, a click chooses a window or a
//! workspace, a window dragged onto a thumbnail changes workspace, and the client receives
//! nothing while the view is open. The captures are drawn with Mesa's software renderer.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        io::Write as _,
        os::{fd::AsFd, unix::net::UnixStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compositor::{State, mission_control::Target, render};
    use sherlock_ui::color::Color;
    use smithay::{
        backend::input::{ButtonState, KeyState},
        desktop::Window,
        input::keyboard::Keycode,
        output::Output,
        reexports::calloop::EventLoop,
        utils::{Logical, Point},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
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
    use wayland_protocols::xdg::decoration::zv1::client::{
        zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
        zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    };

    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        configures: usize,
        /// Keys received: evdev code, pressed.
        keys: Vec<(u32, bool)>,
        /// Buttons received.
        buttons: usize,
        /// The pointer, then the keyboard, is on one of its surfaces.
        pointer_in: bool,
        keyboard_in: bool,
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
                wl_pointer::Event::Enter { .. } => client.pointer_in = true,
                wl_pointer::Event::Leave { .. } => client.pointer_in = false,
                wl_pointer::Event::Button { .. } => client.buttons += 1,
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
            match event {
                wl_keyboard::Event::Enter { .. } => client.keyboard_in = true,
                wl_keyboard::Event::Leave { .. } => client.keyboard_in = false,
                wl_keyboard::Event::Key { key, state, .. } => {
                    let pressed = state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                    client.keys.push((key, pressed));
                }
                _ => {}
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
            client: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
                client.configures += 1;
            }
        }
    }

    delegate_noop!(Client: ignore XdgToplevel);
    delegate_noop!(Client: ignore ZxdgToplevelDecorationV1);
    delegate_noop!(Client: ZxdgDecorationManagerV1);
    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlSeat);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);

    /// Shared memory filled with `fill`, without a file left on disk.
    fn shm_file(w: i32, h: i32, fill: Color) -> File {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("compositor-overview-{}-{n}", std::process::id());
        let path = std::env::temp_dir().join(name);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        // Argb8888 is little-endian: blue, green, red, alpha.
        let pixel = [fill.b, fill.g, fill.r, 255];
        file.write_all(&pixel.repeat((w * h) as usize)).unwrap();
        file
    }

    const BTN_LEFT: u32 = 0x110;
    // xkb codes (evdev + 8): left Super, Tab, Escape, A.
    const KEY_SUPER: u32 = 133;
    const KEY_TAB: u32 = 23;
    const KEY_ESCAPE: u32 = 9;
    const KEY_A: u32 = 38;

    /// A window of the client; its buffer lives as long as it does.
    struct Shown {
        toplevel: XdgToplevel,
        _decoration: ZxdgToplevelDecorationV1,
        _file: File,
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        globals: Option<(WlCompositor, XdgWmBase, WlShm, ZxdgDecorationManagerV1)>,
    }

    impl Harness {
        /// Screens of `sizes` pixels side by side, a client connected.
        fn start(sizes: &[(i32, i32)]) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            for size in sizes {
                state.add_output((*size).into(), 1.0);
            }
            let (server_side, client_side) = UnixStream::pair().unwrap();
            state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            let mut h = Self {
                event_loop,
                state,
                conn,
                queue,
                client: Client::default(),
                globals: None,
            };
            let qh = h.queue.handle();
            let registry = h.conn.display().get_registry(&qh, ());
            h.run_until("the globals", |c, _| {
                c.globals
                    .iter()
                    .any(|g| g.1 == "zxdg_decoration_manager_v1")
            });
            let bind = |name: &str| {
                let (id, _, version) = h.client.globals.iter().find(|g| g.1 == name).unwrap();
                (*id, *version)
            };
            let (c, w, s, d, seat) = (
                bind("wl_compositor"),
                bind("xdg_wm_base"),
                bind("wl_shm"),
                bind("zxdg_decoration_manager_v1"),
                bind("wl_seat"),
            );
            h.globals = Some((
                registry.bind(c.0, c.1.min(6), &qh, ()),
                registry.bind(w.0, w.1.min(6), &qh, ()),
                registry.bind(s.0, 1, &qh, ()),
                registry.bind(d.0, 1, &qh, ()),
            ));
            let seat: WlSeat = registry.bind(seat.0, seat.1.min(7), &qh, ());
            let _pointer = seat.get_pointer(&qh, ());
            let _keyboard = seat.get_keyboard(&qh, ());
            h.run_until("the seat", |_, _| true);
            h
        }

        fn run_until(&mut self, what: &str, done: impl Fn(&Client, &State) -> bool) {
            for _ in 0..200 {
                self.conn.flush().unwrap();
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if let Some(guard) = self.queue.prepare_read() {
                    let _ = guard.read();
                }
                self.queue.dispatch_pending(&mut self.client).unwrap();
                if done(&self.client, &self.state) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        /// A few turns of the loop, for what the client has to read.
        fn sync(&mut self) {
            for _ in 0..4 {
                self.run_until("a turn", |_, _| true);
            }
        }

        /// A decorated window `title` of `w` × `h` filled with `fill`, its content moved to
        /// `at` in the space.
        fn open(&mut self, title: &str, (w, h): (i32, i32), fill: Color, at: (i32, i32)) -> Shown {
            let qh = self.queue.handle();
            let (compositor, wm_base, shm, manager) = self.globals.clone().unwrap();
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title(title.into());
            let decoration = manager.get_toplevel_decoration(&toplevel, &qh, ());
            let configures = self.client.configures;
            surface.commit();
            self.run_until("the first configure", |c, _| c.configures > configures);
            let file = shm_file(w, h, fill);
            let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            let name = title.to_owned();
            self.run_until("the window", |_, s| find(s, &name).is_some());
            let window = self.window(title);
            self.state.space.map_element(window, at, false);
            Shown {
                toplevel,
                _decoration: decoration,
                _file: file,
            }
        }

        fn window(&self, title: &str) -> Window {
            find(&self.state, title).unwrap()
        }

        fn output(&self, index: usize) -> Output {
            self.state.space.outputs().nth(index).unwrap().clone()
        }

        fn key(&mut self, code: u32, pressed: bool) {
            let state = if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            };
            self.state.keyboard_key(Keycode::new(code), state, 10);
        }

        fn tap(&mut self, code: u32) {
            self.key(code, true);
            self.key(code, false);
        }

        fn button(&mut self, pressed: bool) {
            let state = if pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            };
            self.state.pointer_button(BTN_LEFT, state, 20);
        }

        fn click(&mut self, at: Point<f64, Logical>) {
            self.state.pointer_motion(at, 20);
            self.button(true);
            self.button(false);
        }

        /// The center of the slot of `title` in the open view.
        fn slot(&self, title: &str) -> Point<f64, Logical> {
            let window = self.window(title);
            let outputs: Vec<Output> = self.state.space.outputs().cloned().collect();
            let tile = outputs
                .iter()
                .flat_map(|o| self.state.overview_tiles(o, None))
                .find(|t| t.window == window)
                .unwrap();
            tile.slot.loc + tile.slot.size.downscale(2.0).to_point()
        }

        /// Long enough after now for every animation to have settled.
        fn later(&self) -> Duration {
            self.state.now() + Duration::from_secs(5)
        }

        fn focused_title(&self) -> Option<String> {
            State::title(&self.state.focused_window()?)
        }
    }

    fn find(state: &State, title: &str) -> Option<Window> {
        state
            .space
            .elements()
            .find(|w| State::title(w).as_deref() == Some(title))
            .cloned()
    }

    const FILL: Color = Color::rgb(0x15, 0x22, 0x38);

    /// Two windows on a screen of 1440 × 900, "Back" then "Front", which has the keyboard.
    fn two_windows() -> (Harness, Vec<Shown>) {
        let mut h = Harness::start(&[(1440, 900)]);
        let back = h.open("Back", (600, 400), FILL, (100, 200));
        let front = h.open("Front", (500, 350), FILL, (700, 300));
        h.sync();
        assert_eq!(h.focused_title().as_deref(), Some("Front"));
        (h, vec![back, front])
    }

    #[test]
    fn the_gesture_key_alone_tab_and_escape_open_and_close_the_view() {
        let (mut h, _shown) = two_windows();
        // Alone: pressed then released, the view opens and the windows lose the keyboard.
        h.tap(KEY_SUPER);
        assert!(h.state.overview_open());
        assert!(h.state.overview_animating(h.state.now()));
        assert_eq!(h.state.overview_progress(h.later()), 1.0);
        assert!(h.state.focused_window().is_none());
        // Escape closes it: the keyboard goes back to the window that had it.
        h.tap(KEY_ESCAPE);
        assert!(!h.state.overview_open());
        assert_eq!(h.focused_title().as_deref(), Some("Front"));
        assert!(h.state.overview_shown(h.state.now()), "the windows go back");
        assert!(!h.state.overview_shown(h.later()));
        // Super + Tab opens, and again closes.
        h.key(KEY_SUPER, true);
        h.tap(KEY_TAB);
        assert!(h.state.overview_open());
        h.tap(KEY_TAB);
        h.key(KEY_SUPER, false);
        assert!(
            !h.state.overview_open(),
            "Super released after Tab is not alone"
        );
        // The gesture key with another key, or a click in between, is not alone.
        h.key(KEY_SUPER, true);
        h.tap(KEY_A);
        h.key(KEY_SUPER, false);
        h.key(KEY_SUPER, true);
        h.button(true);
        h.button(false);
        h.key(KEY_SUPER, false);
        assert!(!h.state.overview_open());
        // Alone again, it opens; alone once more, it closes.
        h.tap(KEY_SUPER);
        h.tap(KEY_SUPER);
        assert!(!h.state.overview_open());
    }

    #[test]
    fn the_view_follows_three_fingers_up_and_down() {
        let (mut h, _shown) = two_windows();
        let s = &mut h.state;
        // Up, 150 units: half open, held by the fingers, no animation.
        s.gesture_swipe_begin(3, 0);
        for i in 1..=10 {
            s.gesture_swipe_update(i * 10, (0.0, -15.0).into());
        }
        assert!(s.overview_open());
        let half = 150.0 / compositor::gestures::OVERVIEW_DISTANCE;
        assert!((s.overview_progress(s.now()) - half).abs() < 1e-9);
        assert!(!s.overview_animating(s.now()));
        s.gesture_swipe_update(110, (0.0, 15.0).into());
        let back = 135.0 / compositor::gestures::OVERVIEW_DISTANCE;
        assert!((s.overview_progress(s.now()) - back).abs() < 1e-9);
        // Lifted past the trigger: it opens the rest of the way, on its spring.
        s.gesture_swipe_end(120, false);
        assert!(s.overview_open());
        assert!(s.overview_animating(s.now()));
        assert!((s.overview_progress(s.now()) - back).abs() < 0.05);
        assert_eq!(s.overview_progress(s.now() + Duration::from_secs(5)), 1.0);
        // Down, not far enough: it stays open.
        s.gesture_swipe_begin(3, 200);
        s.gesture_swipe_update(210, (0.0, 30.0).into());
        s.gesture_swipe_end(220, false);
        assert!(s.overview_open());
        // Down, far enough: it closes, and the keyboard is back.
        s.gesture_swipe_begin(3, 300);
        for i in 1..=10 {
            s.gesture_swipe_update(300 + i * 10, (0.0, 15.0).into());
        }
        s.gesture_swipe_end(400, false);
        assert!(!s.overview_open());
        assert_eq!(h.focused_title().as_deref(), Some("Front"));
        // Up, not far enough: it comes back closed; cancelled, too.
        let s = &mut h.state;
        s.gesture_swipe_begin(3, 500);
        s.gesture_swipe_update(510, (0.0, -40.0).into());
        s.gesture_swipe_end(520, false);
        assert!(!s.overview_open());
        s.gesture_swipe_begin(3, 600);
        s.gesture_swipe_update(610, (0.0, -200.0).into());
        s.gesture_swipe_end(620, true);
        assert!(!s.overview_open());
    }

    #[test]
    fn a_click_on_a_window_raises_it_and_closes_the_view() {
        let (mut h, _shown) = two_windows();
        h.state.open_overview();
        // The windows sit apart, Back on the left as on the desktop.
        let (back, front) = (h.slot("Back"), h.slot("Front"));
        assert!(back.x < front.x);
        h.state.pointer_motion(back, 20);
        let hovered = h.state.overview_target(back);
        assert_eq!(hovered, Some(Target::Window(h.window("Back"))));
        h.click(back);
        assert!(!h.state.overview_open());
        assert_eq!(h.focused_title().as_deref(), Some("Back"));
        let top = h.state.space.elements().last().unwrap().clone();
        assert_eq!(top, h.window("Back"));
        // A click on nothing closes the view and leaves the keyboard where it was.
        h.state.open_overview();
        h.click((5.0, 890.0).into());
        assert!(!h.state.overview_open());
        assert_eq!(h.focused_title().as_deref(), Some("Back"));
    }

    #[test]
    fn the_clients_get_no_click_nor_key_while_the_view_is_open() {
        let (mut h, _shown) = two_windows();
        h.state.pointer_motion((800.0, 400.0).into(), 1);
        h.sync();
        assert!(h.client.pointer_in && h.client.keyboard_in);
        h.client.keys.clear();
        h.state.open_overview();
        h.sync();
        assert!(!h.client.pointer_in && !h.client.keyboard_in);
        let buttons = h.client.buttons;
        h.tap(KEY_A);
        // A press then a release on a window that only moves it a little.
        let front = h.slot("Front");
        h.state.pointer_motion(front, 30);
        h.button(true);
        h.state.pointer_motion(front + Point::from((40.0, 0.0)), 31);
        h.button(false);
        h.state.pointer_motion((700.0, 450.0).into(), 32);
        h.sync();
        assert!(h.state.overview_open());
        assert_eq!(h.client.keys, Vec::new());
        assert_eq!(h.client.buttons, buttons);
        assert!(!h.client.pointer_in && !h.client.keyboard_in);
        assert_eq!(
            h.state.cursor_icon(),
            Some(smithay::input::pointer::CursorIcon::Default)
        );
        // Closed, the keyboard and the pointer are back, and keys go through again.
        h.tap(KEY_ESCAPE);
        h.tap(KEY_A);
        h.sync();
        assert!(h.client.keyboard_in);
        assert_eq!(h.client.keys, [(KEY_A - 8, true), (KEY_A - 8, false)]);
    }

    #[test]
    fn a_window_dragged_onto_a_thumbnail_changes_workspace() {
        let (mut h, _shown) = two_windows();
        h.state.open_overview();
        let output = h.output(0);
        let strip = h.state.overview_strip(&output);
        assert_eq!(strip.len(), 3);
        let second = strip[1].loc + strip[1].size.downscale(2.0).to_point();
        let back = h.slot("Back");
        h.state.pointer_motion(back, 40);
        h.button(true);
        h.state
            .pointer_motion(back + Point::from((10.0, -10.0)), 41);
        h.state.pointer_motion(second, 42);
        assert_eq!(h.state.overview_target(second), Some(Target::Workspace(1)));
        h.button(false);
        // Still open, on the first workspace, without the window.
        assert!(h.state.overview_open());
        assert_eq!(h.state.workspace_of(&h.window("Back")), "2");
        assert_eq!(h.state.workspaces().active(), "1");
        let tiles = h.state.overview_tiles(&output, None);
        assert_eq!(tiles.len(), 1);
        // A click on the thumbnail goes to that workspace and closes the view.
        h.click(second);
        assert!(!h.state.overview_open());
        assert_eq!(h.state.workspaces().active(), "2");
        assert!(
            !h.state.workspaces_sliding(h.state.now()),
            "no slide under the view"
        );
        assert_eq!(h.focused_title().as_deref(), Some("Back"));
    }

    #[test]
    fn windows_opened_or_closed_in_the_view_join_or_leave_the_grid() {
        let (mut h, mut shown) = two_windows();
        h.state.open_overview();
        let output = h.output(0);
        let _third = h.open("Third", (400, 300), FILL, (300, 500));
        h.sync();
        assert_eq!(h.state.overview_tiles(&output, None).len(), 3);
        assert!(
            h.state.focused_window().is_none(),
            "no keyboard in the view"
        );
        assert!(!h.client.keyboard_in);
        shown.remove(1).toplevel.destroy();
        h.run_until("the window gone", |_, s| find(s, "Front").is_none());
        let tiles = h.state.overview_tiles(&output, None);
        assert_eq!(tiles.len(), 2);
        // Closed, the keyboard goes to the window opened last.
        h.tap(KEY_ESCAPE);
        assert_eq!(h.focused_title().as_deref(), Some("Third"));
    }

    #[test]
    fn nothing_is_drawn_at_rest_with_the_view_open() {
        let (mut h, _shown) = two_windows();
        let output = h.output(0);
        let view = h.state.views()[0].1;
        h.state.open_overview();
        let (items, moving) = render::items(&h.state, (&output, &view), None, false, h.state.now());
        assert!(moving, "the view opens");
        assert!(items.len() > 10);
        let later = h.later();
        let (_, moving) = render::items(&h.state, (&output, &view), None, false, later);
        assert!(!moving, "nothing moves once open");
        // Time alone moves nothing on: no frame is due.
        let epoch = h.state.redraw_epoch();
        h.sync();
        assert_eq!(h.state.redraw_epoch(), epoch);
    }

    #[test]
    fn each_screen_shows_its_own_windows() {
        let mut h = Harness::start(&[(1280, 800), (1280, 800)]);
        let _left = h.open("Left", (500, 400), FILL, (200, 200));
        let _right = h.open("Right", (500, 400), FILL, (1500, 200));
        h.sync();
        h.state.open_overview();
        let (first, second) = (h.output(0), h.output(1));
        let titles = |o: &Output| -> Vec<_> {
            let tiles = h.state.overview_tiles(o, None);
            tiles
                .iter()
                .filter_map(|t| State::title(&t.window))
                .collect()
        };
        assert_eq!(titles(&first), ["Left"]);
        assert_eq!(titles(&second), ["Right"]);
        let right = h.slot("Right");
        assert!(right.x > 1280.0);
        assert_eq!(h.state.overview_strip(&second).len(), 3);
        h.click(right);
        assert_eq!(h.focused_title().as_deref(), Some("Right"));
    }

    /// Captures of the view with five windows, dark then light: the scene of a screen, as the
    /// real session draws it, into a texture with Mesa's software renderer. They go to
    /// `screenshots/`.
    #[test]
    #[ignore = "needs Mesa's EGL; writes the captures: run it with --ignored"]
    fn captures_of_the_view_with_five_windows() {
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
        let (w, h) = (1440, 900);
        for (theme, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
            color::set_active(theme);
            let p = theme.palette();
            let mut harness = Harness::start(&[(w, h)]);
            let windows = [
                ("Inquiry — ~/sherlockOs", (720, 460), p.deepest, (60, 90)),
                ("Evidence — Documents", (820, 520), p.window, (520, 140)),
                ("Firefox", (900, 600), p.card, (300, 260)),
                ("Control Room", (640, 440), p.sidebar, (40, 420)),
                ("Konsole", (560, 360), p.terminal_block, (820, 460)),
            ];
            let _shown: Vec<_> = windows
                .iter()
                .map(|(title, size, fill, at)| harness.open(title, *size, *fill, *at))
                .collect();
            harness.sync();
            harness.state.open_overview();
            let hover = harness.slot("Firefox");
            harness.state.pointer_motion(hover, 50);
            let (mut ids, mut textures) = (Ids::default(), wallpaper::Textures::default());
            let views = harness.state.views();
            let mut elements = Vec::new();
            for at in [harness.state.now(), harness.later()] {
                let state = &mut harness.state;
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
                "{}/screenshots/mission-control-{name}.png",
                env!("CARGO_MANIFEST_DIR")
            );
            image::save_buffer(&path, &pixels, w as u32, h as u32, image::ColorType::Rgba8)
                .unwrap();
            // The accent border runs around the window under the pointer.
            let firefox = harness.window("Firefox");
            let tile = harness
                .state
                .overview_tiles(&views[0].0, None)
                .into_iter()
                .find(|t| t.window == firefox)
                .unwrap();
            let edge = (tile.slot.loc.x - 1.5) as usize;
            let y = (tile.slot.loc.y + tile.slot.size.h / 2.0) as usize;
            let at = &pixels[(y * w as usize + edge) * 4..][..3];
            let accent = color::palette().accent;
            assert_eq!(at, [accent.r, accent.g, accent.b], "{name}");
        }
        color::set_active(Theme::Dark);
    }
}
