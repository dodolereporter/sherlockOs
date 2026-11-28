//! The session lock (lot D-25a, ADR 0023), without a screen: a test lock client takes the lock
//! through `ext-session-lock-v1`, beside an ordinary client that shows a window. The frames are
//! read as the list of items each screen would draw; the input goes through the entry points of
//! the backends. A frame shown is reported by hand (`lock_frame_shown`), as the backends do at
//! the vertical blank.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::net::UnixStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use std::io::Write as _;

    use compositor::{State, lock::Programs, render::Item};
    use smithay::{
        backend::input::{ButtonState, KeyState},
        input::keyboard::Keycode,
        reexports::{calloop::EventLoop, wayland_server::Resource as _},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_compositor::WlCompositor,
            wl_keyboard::{self, WlKeyboard},
            wl_output::WlOutput,
            wl_pointer::{self, WlPointer},
            wl_registry,
            wl_seat::WlSeat,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::ext::session_lock::v1::client::{
        ext_session_lock_manager_v1::ExtSessionLockManagerV1,
        ext_session_lock_surface_v1::{self, ExtSessionLockSurfaceV1},
        ext_session_lock_v1::{self, ExtSessionLockV1},
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    };
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, ZwlrLayerSurfaceV1},
    };

    // xkb codes: the evdev codes of linux/input-event-codes.h, plus 8.
    const KEY_SUPER: u32 = 125 + 8;
    const KEY_L: u32 = 38 + 8;
    const KEY_A: u32 = 16 + 8;
    const KEY_TAB: u32 = 15 + 8;
    const KEY_ENTER: u32 = 28 + 8;
    const BTN_LEFT: u32 = 0x110;

    /// What a client has received.
    #[derive(Default)]
    struct Seen {
        globals: Vec<(u32, String, u32)>,
        keys: Vec<u32>,
        presses: usize,
        window_configured: bool,
        layer_configured: bool,
        lock_size: Option<(u32, u32)>,
        /// Every configure of the lock surfaces.
        lock_sizes: Vec<(u32, u32)>,
        locked: bool,
        finished: bool,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Seen {
        fn event(
            seen: &mut Self,
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
                seen.globals.push((name, interface, version));
            }
        }
    }

    impl Dispatch<WlKeyboard, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &WlKeyboard,
            event: wl_keyboard::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_keyboard::Event::Key { key, .. } = event {
                seen.keys.push(key);
            }
        }
    }

    impl Dispatch<WlPointer, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &WlPointer,
            event: wl_pointer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_pointer::Event::Button { .. } = event {
                seen.presses += 1;
            }
        }
    }

    impl Dispatch<XdgWmBase, ()> for Seen {
        fn event(
            _: &mut Self,
            wm_base: &XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                wm_base.pong(serial);
            }
        }
    }

    impl Dispatch<XdgSurface, ()> for Seen {
        fn event(
            seen: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
                seen.window_configured = true;
            }
        }
    }

    impl Dispatch<ZwlrLayerSurfaceV1, ()> for Seen {
        fn event(
            seen: &mut Self,
            surface: &ZwlrLayerSurfaceV1,
            event: zwlr_layer_surface_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
                surface.ack_configure(serial);
                seen.layer_configured = true;
            }
        }
    }

    impl Dispatch<ExtSessionLockV1, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &ExtSessionLockV1,
            event: ext_session_lock_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                ext_session_lock_v1::Event::Locked => seen.locked = true,
                ext_session_lock_v1::Event::Finished => seen.finished = true,
                _ => {}
            }
        }
    }

    impl Dispatch<ExtSessionLockSurfaceV1, ()> for Seen {
        fn event(
            seen: &mut Self,
            surface: &ExtSessionLockSurfaceV1,
            event: ext_session_lock_surface_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let ext_session_lock_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } = event
            {
                surface.ack_configure(serial);
                seen.lock_size = Some((width, height));
                seen.lock_sizes.push((width, height));
            }
        }
    }

    delegate_noop!(Seen: WlCompositor);
    delegate_noop!(Seen: ignore WlSurface);
    delegate_noop!(Seen: ignore WlSeat);
    delegate_noop!(Seen: ignore WlShm);
    delegate_noop!(Seen: WlShmPool);
    delegate_noop!(Seen: ignore WlBuffer);
    delegate_noop!(Seen: ignore WlOutput);
    delegate_noop!(Seen: ignore XdgToplevel);
    delegate_noop!(Seen: ZwlrLayerShellV1);
    delegate_noop!(Seen: ExtSessionLockManagerV1);

    /// A client connected to the compositor.
    struct Peer {
        conn: Connection,
        queue: EventQueue<Seen>,
        seen: Seen,
        registry: wl_registry::WlRegistry,
        /// Keeps the buffers' memory alive.
        files: Vec<File>,
        /// Hung: reads and answers nothing any more.
        frozen: bool,
    }

    impl Peer {
        fn qh(&self) -> QueueHandle<Seen> {
            self.queue.handle()
        }

        fn bind<I: Proxy + 'static>(&self) -> I
        where
            Seen: Dispatch<I, ()>,
        {
            let name = I::interface().name;
            let (id, _, version) = self.seen.globals.iter().find(|g| g.1 == name).unwrap();
            self.registry
                .bind(*id, (*version).min(I::interface().version), &self.qh(), ())
        }

        /// The connection ended with a protocol error.
        fn dead(&self) -> bool {
            self.conn.protocol_error().is_some()
        }

        /// A buffer of `w` × `h`, attached and committed.
        fn show(&mut self, surface: &WlSurface, w: i32, h: i32) {
            let shm: WlShm = self.bind();
            let file = shm_file((w * h * 4) as u64);
            let pool = shm.create_pool(file.as_fd(), w * h * 4, &self.qh(), ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &self.qh(), ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self.files.push(file);
        }
    }

    fn shm_file(size: u64) -> File {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("session-lock-test-{}-{n}", std::process::id());
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

    /// A lock taken by a client, and its surface on the screen.
    struct Locking {
        lock: ExtSessionLockV1,
        surface: WlSurface,
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        peers: Vec<Peer>,
    }

    impl Harness {
        /// One screen of 800 × 600; the lock programs are `client`, then `rescue`.
        fn start(client: &[&str], rescue: &[&str]) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            let command = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
            state.set_lock_programs(Programs {
                client: command(client),
                rescue: Some(command(rescue)),
                swaylock: None,
            });
            Self {
                event_loop,
                state,
                peers: Vec::new(),
            }
        }

        /// A new ordinary client, with the globals listed. Returns its index.
        fn connect(&mut self) -> usize {
            self.connect_as(false)
        }

        /// A new client on a connection the compositor takes as the lock program's, as it does
        /// for the program it starts.
        fn connect_program(&mut self) -> usize {
            self.connect_as(true)
        }

        fn connect_as(&mut self, program: bool) -> usize {
            let (server_side, client_side) = UnixStream::pair().unwrap();
            match program {
                true => self.state.insert_lock_program(server_side),
                false => self.state.insert_client(server_side),
            }
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            self.peers.push(Peer {
                conn,
                queue,
                seen: Seen::default(),
                registry,
                files: Vec::new(),
                frozen: false,
            });
            let i = self.peers.len() - 1;
            self.run_until("the globals", |h| !h.peers[i].seen.globals.is_empty());
            i
        }

        /// Runs the compositor and the clients until `done` is true.
        fn run_until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
            for _ in 0..3000 {
                for peer in &mut self.peers {
                    let _ = peer.conn.flush();
                }
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                for peer in self.peers.iter_mut().filter(|p| !p.frozen) {
                    if let Some(guard) = peer.queue.prepare_read() {
                        // Nothing to read is not an error, nor a connection that ended.
                        let _ = guard.read();
                    }
                    let _ = peer.queue.dispatch_pending(&mut peer.seen);
                }
                if done(self) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        /// Client `i` shows a window of 100 × 80, listening to the keyboard and the pointer.
        fn show_window(&mut self, i: usize) -> WlSurface {
            let peer = &self.peers[i];
            let (qh, seat) = (peer.qh(), peer.bind::<WlSeat>());
            let _pointer = seat.get_pointer(&qh, ());
            let _keyboard = seat.get_keyboard(&qh, ());
            let surface = peer.bind::<WlCompositor>().create_surface(&qh, ());
            let wm_base: XdgWmBase = peer.bind();
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title("window".into());
            surface.commit();
            self.run_until("the window's configure", |h| {
                h.peers[i].seen.window_configured
            });
            self.peers[i].show(&surface, 100, 80);
            self.run_until("the window shown", |h| h.window_items() == 1);
            surface
        }

        /// Client `i` asks for the lock and a surface on the screen, and shows it once
        /// configured. `None` if the compositor refused the lock.
        fn lock(&mut self, i: usize) -> Option<Locking> {
            let peer = &self.peers[i];
            let (qh, seat) = (peer.qh(), peer.bind::<WlSeat>());
            let _keyboard = seat.get_keyboard(&qh, ());
            let _pointer = seat.get_pointer(&qh, ());
            let output: WlOutput = peer.bind();
            // What the compositor pings; the client answers from its loop (`Seen`).
            let _wm_base: XdgWmBase = peer.bind();
            let manager: ExtSessionLockManagerV1 = peer.bind();
            let lock = manager.lock(&qh, ());
            let surface = peer.bind::<WlCompositor>().create_surface(&qh, ());
            let _lock_surface = lock.get_lock_surface(&surface, &output, &qh, ());
            self.run_until("the lock surface configured, or the lock refused", |h| {
                let seen = &h.peers[i].seen;
                seen.lock_size.is_some() || seen.finished
            });
            if self.peers[i].seen.finished {
                return None;
            }
            assert_eq!(self.peers[i].seen.lock_size, Some((800, 600)));
            self.peers[i].show(&surface, 800, 600);
            self.run_until("the lock surface shown", |h| h.lock_surface_drawn());
            Some(Locking { lock, surface })
        }

        /// The screen shows a locked frame: the lock is confirmed.
        fn show_frame(&mut self) {
            let output = self.state.outputs()[0].clone();
            self.state.lock_frame_shown(&output);
        }

        fn items(&self) -> Vec<Item> {
            let (output, view) = self.state.views().remove(0);
            compositor::render::items(&self.state, (&output, &view), None, false, self.state.now())
                .0
        }

        fn window_items(&self) -> usize {
            let items = self.items();
            items
                .iter()
                .filter(|i| matches!(i, Item::Window { .. }))
                .count()
        }

        /// Every client surface drawn, by the client that owns it.
        fn drawn_surfaces(&self) -> Vec<smithay::reexports::wayland_server::Client> {
            self.items()
                .iter()
                .filter_map(|item| match item {
                    Item::Surface { surface, .. } | Item::Window { surface, .. } => {
                        surface.client()
                    }
                    _ => None,
                })
                .collect()
        }

        fn lock_surface_drawn(&self) -> bool {
            self.items()
                .iter()
                .any(|i| matches!(i, Item::Surface { .. }))
        }

        fn key(&mut self, code: u32) {
            for state in [KeyState::Pressed, KeyState::Released] {
                self.state.keyboard_key(Keycode::new(code), state, 10);
            }
        }

        fn chord(&mut self, held: u32, code: u32) {
            self.state
                .keyboard_key(Keycode::new(held), KeyState::Pressed, 10);
            self.key(code);
            self.state
                .keyboard_key(Keycode::new(held), KeyState::Released, 11);
        }

        fn click(&mut self, x: f64, y: f64) {
            self.state.pointer_motion((x, y).into(), 20);
            self.state
                .pointer_button(BTN_LEFT, ButtonState::Pressed, 21);
            self.state
                .pointer_button(BTN_LEFT, ButtonState::Released, 22);
        }

        /// Runs the loop for a while, without waiting for anything.
        fn settle(&mut self) {
            let mut turns = 0;
            self.run_until("nothing", |_| {
                turns += 1;
                turns > 20
            });
        }
    }

    /// The evdev code a client receives for an xkb code.
    fn evdev(code: u32) -> u32 {
        code - 8
    }

    #[test]
    fn once_locked_only_the_lock_surface_is_drawn_and_gets_input() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        h.show_window(app);
        let locker = h.connect_program();
        let locking = h.lock(locker).unwrap();
        assert!(h.state.is_locked());
        // The lock is confirmed once the screen has shown a locked frame.
        assert!(!h.peers[locker].seen.locked);
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);

        // Only the lock surface is drawn, in front of the background.
        let drawn = h.drawn_surfaces();
        assert_eq!(drawn.len(), 1, "only the lock surface is drawn");
        assert_eq!(h.window_items(), 0);
        let items = h.items();
        assert!(
            matches!(items.last(), Some(Item::Shape { .. })),
            "background behind"
        );
        assert!(locking.surface.is_alive());

        // The keyboard and the pointer reach the lock surface, not the window under it.
        h.peers[app].seen.keys.clear();
        h.key(KEY_A);
        h.click(400.0, 300.0);
        h.run_until("the key and the click on the lock", |h| {
            let seen = &h.peers[locker].seen;
            seen.keys.contains(&evdev(KEY_A)) && seen.presses == 2
        });
        assert!(h.peers[app].seen.keys.is_empty());
        assert_eq!(h.peers[app].seen.presses, 0);

        // The shortcuts of the compositor do nothing: Mission Control, the launcher's key.
        h.chord(KEY_SUPER, KEY_TAB);
        assert!(!h.state.overview_open());
        assert!(h.state.is_locked());
    }

    #[test]
    fn an_ordinary_client_cannot_show_anything_over_the_lock() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        let locker = h.connect_program();
        let _locking = h.lock(locker).unwrap();
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);

        // A new window and an overlay surface anchored to every edge, with the keyboard.
        h.show_window_while_locked(app);
        let peer = &h.peers[app];
        let qh = peer.qh();
        let surface = peer.bind::<WlCompositor>().create_surface(&qh, ());
        let shell: ZwlrLayerShellV1 = peer.bind();
        let layer = shell.get_layer_surface(&surface, None, Layer::Overlay, "over".into(), &qh, ());
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
        surface.commit();
        h.run_until("the overlay configured", |h| {
            h.peers[app].seen.layer_configured
        });
        h.peers[app].show(&surface, 800, 600);
        h.settle();

        assert_eq!(
            h.drawn_surfaces().len(),
            1,
            "only the lock surface is drawn"
        );
        h.peers[app].seen.keys.clear();
        h.key(KEY_A);
        h.click(400.0, 300.0);
        h.settle();
        assert!(h.peers[app].seen.keys.is_empty(), "no key for the overlay");
        assert_eq!(h.peers[app].seen.presses, 0);
        assert!(h.peers[locker].seen.keys.contains(&evdev(KEY_A)));
    }

    impl Harness {
        /// `show_window`, while locked: the window is placed, never drawn.
        fn show_window_while_locked(&mut self, i: usize) {
            let peer = &self.peers[i];
            let (qh, seat) = (peer.qh(), peer.bind::<WlSeat>());
            let _pointer = seat.get_pointer(&qh, ());
            let _keyboard = seat.get_keyboard(&qh, ());
            let surface = peer.bind::<WlCompositor>().create_surface(&qh, ());
            let wm_base: XdgWmBase = peer.bind();
            let _toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            surface.commit();
            self.run_until("the window's configure", |h| {
                h.peers[i].seen.window_configured
            });
            self.peers[i].show(&surface, 100, 80);
            self.run_until("the window placed", |h| {
                h.state.space.elements().count() == 1
            });
        }
    }

    #[test]
    fn only_the_lock_that_holds_the_session_unlocks_it() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        let window = h.show_window(app);
        let locker = h.connect_program();
        let locking = h.lock(locker).unwrap();
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);

        // Another client's lock is refused: `finished`. Its unlock is a protocol error, and the
        // session stays locked.
        let intruder = h.connect();
        assert!(h.lock(intruder).is_none());
        let manager: ExtSessionLockManagerV1 = h.peers[intruder].bind();
        let qh = h.peers[intruder].qh();
        manager.lock(&qh, ()).unlock_and_destroy();
        h.run_until("the intruder's error", |h| h.peers[intruder].dead());
        assert!(h.state.is_locked());
        assert_eq!(h.window_items(), 0);

        // The lock that holds it unlocks: the window is drawn and gets the keyboard back.
        locking.lock.unlock_and_destroy();
        h.run_until("unlocked", |h| !h.state.is_locked());
        assert_eq!(h.window_items(), 1);
        h.peers[app].seen.keys.clear();
        h.key(KEY_A);
        h.run_until("the key on the window", |h| {
            h.peers[app].seen.keys.contains(&evdev(KEY_A))
        });
        assert!(window.is_alive());
    }

    #[test]
    fn an_unlock_before_locked_is_refused() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let locker = h.connect_program();
        let locking = h.lock(locker).unwrap();
        // No frame shown yet: `locked` was not sent, the unlock is a protocol error.
        locking.lock.unlock_and_destroy();
        h.run_until("the error", |h| h.peers[locker].dead());
        assert!(h.state.is_locked());
        assert!(!h.lock_surface_drawn());
    }

    #[test]
    fn a_killed_lock_client_leaves_the_session_locked_then_the_rescue_shows() {
        // The lock program never locks: three failures, then the rescue program, which is not
        // there either, then the rescue screen of the compositor.
        let mut h = Harness::start(&["true"], &["/nonexistent/swaylock"]);
        let app = h.connect();
        h.show_window(app);
        let locker = h.connect_program();
        let _locking = h.lock(locker).unwrap();
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);

        // The lock client is killed: its connection ends without an unlock.
        let killed = h.peers.remove(locker);
        drop(killed);
        h.run_until("the lock surface gone", |h| !h.lock_surface_drawn());
        assert!(h.state.is_locked());
        assert_eq!(h.drawn_surfaces().len(), 0, "the plain background only");
        assert_eq!(h.window_items(), 0);

        h.run_until("the rescue screen", |h| h.state.lock_rescue_shown());
        assert!(h.state.is_locked());
        let labels = h
            .items()
            .iter()
            .filter(|i| matches!(i, Item::Label { .. }))
            .count();
        assert_eq!(labels, 2, "« Verrouillé » and the hint");
        h.peers[app].seen.keys.clear();
        h.key(KEY_A);
        h.click(400.0, 300.0);
        h.settle();
        assert!(h.peers[app].seen.keys.is_empty());
        assert_eq!(h.peers[app].seen.presses, 0);

        // Enter tries the lock programs again.
        h.key(KEY_ENTER);
        assert!(!h.state.lock_rescue_shown());
        assert!(h.state.is_locked());
    }

    #[test]
    fn once_the_lock_program_is_dead_another_client_can_neither_lock_nor_unlock() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let locker = h.connect_program();
        let _locking = h.lock(locker).unwrap();
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);
        let killed = h.peers.remove(locker);
        drop(killed);
        h.run_until("the lock surface gone", |h| !h.lock_surface_drawn());

        // A client that connects after its death is refused, and its unlock is an error.
        let other = h.connect();
        assert!(h.lock(other).is_none());
        let manager: ExtSessionLockManagerV1 = h.peers[other].bind();
        let qh = h.peers[other].qh();
        manager.lock(&qh, ()).unlock_and_destroy();
        h.run_until("the error", |h| h.peers[other].dead());
        assert!(h.state.is_locked());
        assert!(!h.lock_surface_drawn());

        // The next lock program is accepted, and confirmed at once: the screen already shows
        // the lock.
        let program = h.connect_program();
        let _locking = h.lock(program).unwrap();
        h.run_until("locked", |h| h.peers[program].seen.locked);
    }

    #[test]
    fn an_ordinary_client_cannot_lock_the_session() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        h.show_window(app);
        let other = h.connect();
        assert!(h.lock(other).is_none(), "refused: finished");
        assert!(!h.state.is_locked());
        assert_eq!(h.window_items(), 1);
    }

    #[test]
    fn the_surfaces_of_a_lock_given_up_before_locked_go_with_it() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let locker = h.connect_program();
        let locking = h.lock(locker).unwrap();
        assert!(h.lock_surface_drawn());
        // Destroying a lock before `locked` is allowed: the client stays connected.
        locking.lock.destroy();
        h.run_until("the surface no longer drawn", |h| !h.lock_surface_drawn());
        assert!(h.state.is_locked());
        h.peers[locker].seen.keys.clear();
        h.key(KEY_A);
        h.settle();
        assert!(
            h.peers[locker].seen.keys.is_empty(),
            "no key for an orphan surface"
        );
    }

    #[test]
    fn super_l_locks_at_once() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        h.show_window(app);
        h.chord(KEY_SUPER, KEY_L);
        assert!(h.state.is_locked());
        assert_eq!(h.drawn_surfaces().len(), 0, "the plain background only");
        assert_eq!(h.window_items(), 0);
        // What the window received before the lock: Super pressed.
        h.settle();
        h.peers[app].seen.keys.clear();
        h.key(KEY_A);
        h.settle();
        assert_eq!(h.peers[app].seen.keys, []);
    }

    /// The screens of the brief: a small laptop, the author's screen, a 2560 × 1600 at 1.5 and a
    /// 4K at 2. Each gets a lock surface of its logical size, drawn on it alone.
    const SCREENS: [((i32, i32), f64); 4] = [
        ((1366, 768), 1.0),
        ((1920, 1080), 1.0),
        ((2560, 1600), 1.5),
        ((3840, 2160), 2.0),
    ];

    /// A harness whose screens are `SCREENS`, side by side.
    fn start_on_every_screen(client: &[&str], rescue: &[&str]) -> Harness {
        let mut h = Harness::start(client, rescue);
        let first = h.state.outputs()[0].clone();
        h.state.remove_screen(&first);
        for (size, scale) in SCREENS {
            h.state.add_output(size.into(), scale);
        }
        h
    }

    #[test]
    fn every_screen_gets_a_lock_surface_of_its_own_size() {
        let mut h = start_on_every_screen(&["sleep", "5"], &["false"]);
        let locker = h.connect_program();
        let peer = &h.peers[locker];
        let qh = peer.qh();
        let manager: ExtSessionLockManagerV1 = peer.bind();
        let lock = manager.lock(&qh, ());
        let outputs: Vec<(u32, u32)> = peer
            .seen
            .globals
            .iter()
            .filter(|g| g.1 == "wl_output")
            .map(|g| (g.0, g.2))
            .collect();
        assert_eq!(outputs.len(), SCREENS.len());
        let mut surfaces = Vec::new();
        for (name, version) in outputs {
            let output: WlOutput = peer.registry.bind(name, version.min(4), &qh, ());
            let surface = peer.bind::<WlCompositor>().create_surface(&qh, ());
            let _lock_surface = lock.get_lock_surface(&surface, &output, &qh, ());
            surfaces.push(surface);
        }
        h.run_until("a configure per screen", |h| {
            h.peers[locker].seen.lock_sizes.len() == SCREENS.len()
        });
        // The logical size of each screen, in the order the screens were added.
        let mut expected: Vec<(u32, u32)> = h
            .state
            .views()
            .iter()
            .map(|(_, view)| (view.area.size.w as u32, view.area.size.h as u32))
            .collect();
        let mut sizes = h.peers[locker].seen.lock_sizes.clone();
        assert_eq!(
            expected,
            [(1366, 768), (1920, 1080), (1707, 1067), (1920, 1080)]
        );
        expected.sort();
        sizes.sort();
        assert_eq!(sizes, expected);
        for (surface, (w, h_)) in surfaces.iter().zip(h.peers[locker].seen.lock_sizes.clone()) {
            h.peers[locker].show(surface, w as i32, h_ as i32);
        }
        h.settle();
        // Each screen draws one surface, its own, over the whole screen.
        for (output, view) in h.state.views() {
            let items =
                compositor::render::items(&h.state, (&output, &view), None, false, h.state.now()).0;
            let drawn: Vec<_> = items
                .iter()
                .filter_map(|i| match i {
                    Item::Surface { origin, scale, .. } => Some((*origin, *scale)),
                    _ => None,
                })
                .collect();
            assert_eq!(drawn, [(view.frame.loc, view.scale)], "{output:?}");
            let background = items
                .iter()
                .any(|i| matches!(i, Item::Shape { rect, .. } if *rect == view.frame));
            assert!(background, "the background covers the screen");
        }
    }

    #[test]
    fn the_rescue_screen_is_centered_on_every_screen() {
        let mut h = start_on_every_screen(&["true"], &["/nonexistent/swaylock"]);
        h.chord(KEY_SUPER, KEY_L);
        h.run_until("the rescue screen", |h| h.state.lock_rescue_shown());
        for (output, view) in h.state.views() {
            let items =
                compositor::render::items(&h.state, (&output, &view), None, false, h.state.now()).0;
            let labels: Vec<_> = items
                .iter()
                .filter_map(|i| match i {
                    Item::Label { rect, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(labels.len(), 2, "{output:?}");
            let frame = view.frame;
            let center = frame.loc.x + frame.size.w / 2;
            for rect in &labels {
                assert!(
                    frame.contains_rect(*rect),
                    "{output:?}: {rect:?} out of {frame:?}"
                );
                let middle = rect.loc.x + rect.size.w / 2;
                assert!((middle - center).abs() <= 1, "{output:?}: not centered");
            }
            // Their height follows the scale of the screen.
            let height = labels[0].size.h as f64 / view.scale;
            assert!((height - 24.0).abs() <= 1.0, "{output:?}: {height}");
        }
    }

    #[test]
    fn the_session_locks_after_the_delay_without_input() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let handle = h.event_loop.handle();
        h.state.watch_idle(&handle, None).unwrap();
        // Never: a second and a half without input leaves it unlocked.
        h.state.set_idle_lock(None);
        let start = std::time::Instant::now();
        h.run_until("a while", |_| start.elapsed() > Duration::from_millis(1500));
        assert!(!h.state.is_locked());
        h.state.set_idle_lock(Some(Duration::from_millis(300)));
        h.run_until("the lock after the delay", |h| h.state.is_locked());
    }

    /// A private bus with a fake logind: its manager answers `GetSessionByPID` and `Inhibit`
    /// (one end of a pipe; the other is kept, to see when the compositor lets it go).
    struct FakeLogind {
        daemon: std::process::Child,
        address: String,
        connection: zbus::blocking::Connection,
        _dir: std::path::PathBuf,
    }

    impl Drop for FakeLogind {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self._dir);
        }
    }

    struct Manager(std::sync::Arc<std::sync::Mutex<Vec<std::io::PipeReader>>>);

    const SESSION: &str = "/org/freedesktop/login1/session/test";

    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Manager {
        #[zbus(name = "GetSessionByPID")]
        fn get_session_by_pid(&self, _pid: u32) -> zbus::zvariant::OwnedObjectPath {
            SESSION.try_into().unwrap()
        }

        fn inhibit(
            &self,
            what: String,
            _who: String,
            _why: String,
            mode: String,
        ) -> zbus::zvariant::OwnedFd {
            assert_eq!((what.as_str(), mode.as_str()), ("sleep", "delay"));
            let (reader, writer) = std::io::pipe().unwrap();
            self.0.lock().unwrap().push(reader);
            std::os::fd::OwnedFd::from(writer).into()
        }
    }

    fn fake_logind(name: &str) -> FakeLogind {
        use std::io::BufRead as _;
        let dir = std::env::temp_dir().join(format!("lock-logind-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>system</type><listen>unix:dir={}</listen>\
                 <auth>EXTERNAL</auth><policy context=\"default\">\
                 <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                 <allow own=\"*\"/></policy></busconfig>",
                dir.display()
            ),
        )
        .unwrap();
        let mut daemon = std::process::Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address"])
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("dbus-daemon (package dbus-daemon) is needed by these tests");
        let mut address = String::new();
        std::io::BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_owned();
        let connection = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .serve_at("/org/freedesktop/login1", Manager(Default::default()))
            .unwrap()
            .name("org.freedesktop.login1")
            .unwrap()
            .build()
            .unwrap();
        FakeLogind {
            daemon,
            address,
            connection,
            _dir: dir,
        }
    }

    impl FakeLogind {
        fn signal(&self, path: &str, interface: &str, name: &str, body: &impl serde_like::Body) {
            body.emit(&self.connection, path, interface, name);
        }
    }

    /// The bodies of the two signals: none, or a boolean.
    mod serde_like {
        pub trait Body {
            fn emit(&self, c: &zbus::blocking::Connection, path: &str, iface: &str, name: &str);
        }
        impl Body for () {
            fn emit(&self, c: &zbus::blocking::Connection, path: &str, iface: &str, name: &str) {
                c.emit_signal(None::<&str>, path, iface, name, &()).unwrap();
            }
        }
        impl Body for bool {
            fn emit(&self, c: &zbus::blocking::Connection, path: &str, iface: &str, name: &str) {
                c.emit_signal(None::<&str>, path, iface, name, &(*self,))
                    .unwrap();
            }
        }
    }

    #[test]
    fn logind_locks_the_session_of_this_process() {
        let logind = fake_logind("lock");
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let handle = h.event_loop.handle();
        h.state
            .watch_logind(&handle, Some(logind.address.clone()))
            .unwrap();
        h.run_until("the sleep inhibitor", |h| h.state.holds_sleep_inhibitor());
        // Another session's lock does nothing.
        let session = "org.freedesktop.login1.Session";
        logind.signal(
            "/org/freedesktop/login1/session/other",
            session,
            "Lock",
            &(),
        );
        h.settle();
        assert!(!h.state.is_locked());
        logind.signal(SESSION, session, "Lock", &());
        h.run_until("the lock of logind", |h| h.state.is_locked());
    }

    #[test]
    fn before_the_sleep_the_session_locks_then_lets_the_machine_sleep() {
        let logind = fake_logind("sleep");
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        h.show_window(app);
        let handle = h.event_loop.handle();
        h.state
            .watch_logind(&handle, Some(logind.address.clone()))
            .unwrap();
        h.run_until("the sleep inhibitor", |h| h.state.holds_sleep_inhibitor());
        let manager = "org.freedesktop.login1.Manager";
        logind.signal("/org/freedesktop/login1", manager, "PrepareForSleep", &true);
        h.run_until("locked before the sleep", |h| h.state.is_locked());
        assert_eq!(h.window_items(), 0);
        // The inhibitor is held until the screen shows the lock.
        assert!(h.state.holds_sleep_inhibitor());
        h.show_frame();
        h.run_until("the inhibitor released", |h| {
            !h.state.holds_sleep_inhibitor()
        });
        // At wake-up, a new one is taken; the session is still locked.
        logind.signal(
            "/org/freedesktop/login1",
            manager,
            "PrepareForSleep",
            &false,
        );
        h.run_until("a new inhibitor", |h| h.state.holds_sleep_inhibitor());
        assert!(h.state.is_locked());
    }

    /// The lock program of `the_rescue_mode_takes_the_lock_and_comes_back_when_killed`: this
    /// test binary, started by the compositor of that test with the connection it opened
    /// (`WAYLAND_SOCKET`). It locks, writes its pid into the file `LOCK_CHILD` names once
    /// `locked` comes, then waits to be killed. Without `LOCK_CHILD`, it does nothing.
    #[test]
    #[ignore = "started by the compositor of another test, as its lock program"]
    fn lock_program_child() {
        let Some(mark) = std::env::var_os("LOCK_CHILD") else {
            return;
        };
        let conn = Connection::connect_to_env().unwrap();
        let mut queue = conn.new_event_queue();
        let registry = conn.display().get_registry(&queue.handle(), ());
        let mut seen = Seen::default();
        queue.roundtrip(&mut seen).unwrap();
        let mut peer = Peer {
            conn,
            queue,
            seen,
            registry,
            files: Vec::new(),
            frozen: false,
        };
        let qh = peer.qh();
        let output: WlOutput = peer.bind();
        let manager: ExtSessionLockManagerV1 = peer.bind();
        let lock = manager.lock(&qh, ());
        let surface = peer.bind::<WlCompositor>().create_surface(&qh, ());
        let _lock_surface = lock.get_lock_surface(&surface, &output, &qh, ());
        while peer.seen.lock_size.is_none() && !peer.seen.finished {
            peer.queue.blocking_dispatch(&mut peer.seen).unwrap();
        }
        let (w, h) = peer.seen.lock_size.unwrap();
        peer.show(&surface, w as i32, h as i32);
        while !peer.seen.locked {
            peer.queue.blocking_dispatch(&mut peer.seen).unwrap();
        }
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(mark)
            .unwrap();
        writeln!(file, "{}", std::process::id()).unwrap();
        drop(file);
        while peer.queue.blocking_dispatch(&mut peer.seen).is_ok() {}
    }

    #[test]
    fn the_rescue_mode_takes_the_lock_and_comes_back_when_killed() {
        let dir = std::env::temp_dir().join(format!("lock-rescue-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (started, mark) = (dir.join("started"), dir.join("locked"));
        // The lock program never locks; its rescue mode is the child above.
        let client = format!("echo client >> {}", started.display());
        let exe = std::env::current_exe().unwrap();
        let child = format!("LOCK_CHILD={}", mark.display());
        let rescue = [
            "env",
            child.as_str(),
            exe.to_str().unwrap(),
            "--exact",
            "tests::lock_program_child",
            "--ignored",
            "--test-threads=1",
        ];
        let mut h = Harness::start(&["sh", "-c", client.as_str()], &rescue);
        let app = h.connect();
        h.show_window(app);
        h.chord(KEY_SUPER, KEY_L);
        h.show_frame();
        let pids = |mark: &std::path::Path| {
            std::fs::read_to_string(mark)
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        h.run_until("the rescue mode locked", |h| {
            h.state.lock_confirmed() && pids(&mark).len() == 1
        });
        assert_eq!(h.state.lock_stage(), compositor::lock::Stage::Rescue);
        let clients = std::fs::read_to_string(&started).unwrap();
        assert_eq!(
            clients.lines().count(),
            3,
            "three tries of the lock program first"
        );
        assert!(h.lock_surface_drawn(), "the rescue's surface is drawn");
        assert_eq!(h.window_items(), 0);

        // An ordinary client can neither take the lock nor unlock while the rescue holds it.
        let other = h.connect();
        assert!(h.lock(other).is_none());
        assert!(h.state.is_locked());

        // The rescue killed: the session stays locked, another rescue takes the lock.
        let first = pids(&mark)[0].clone();
        let killed = std::process::Command::new("kill")
            .args(["-KILL", first.as_str()])
            .status()
            .unwrap();
        assert!(killed.success());
        h.run_until("the rescue gone", |h| !h.state.lock_confirmed());
        assert!(h.state.is_locked());
        h.run_until("the rescue again", |h| {
            h.state.lock_confirmed() && pids(&mark).len() == 2
        });
        assert_eq!(h.state.lock_stage(), compositor::lock::Stage::Rescue);
        let second = pids(&mark)[1].clone();
        let _ = std::process::Command::new("kill")
            .args(["-KILL", second.as_str()])
            .status();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Runs the loop for `duration`.
    fn wait(h: &mut Harness, duration: Duration) {
        let start = std::time::Instant::now();
        h.run_until("a while", |_| start.elapsed() > duration);
    }

    #[test]
    fn a_lock_program_that_hangs_is_killed_and_started_again_the_session_staying_locked() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let app = h.connect();
        h.show_window(app);
        let locker = h.connect_program();
        let _locking = h.lock(locker).unwrap();
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);
        let syncs = h.state.log_sync_requests();
        assert!(syncs >= 1, "the log is written to disk at the lock");
        h.state.set_lock_ping_timeout(Duration::from_millis(300));

        // A lock program that answers stays, key after key.
        h.key(KEY_A);
        wait(&mut h, Duration::from_millis(1100));
        h.key(KEY_A);
        wait(&mut h, Duration::from_millis(600));
        assert!(h.state.lock_program_connected());
        assert_eq!(h.state.lock_attempts(), 0, "nothing started again");

        // It hangs: the next input pings it, and nothing answers.
        h.peers[locker].frozen = true;
        wait(&mut h, Duration::from_millis(1100));
        h.click(400.0, 300.0);
        h.run_until("the hung lock program replaced", |h| {
            h.state.lock_attempts() == 1
        });
        assert!(h.state.is_locked(), "the session stays locked");
        assert!(!h.lock_surface_drawn());
        assert_eq!(h.window_items(), 0, "nothing of the session shows");
        assert_eq!(h.state.lock_stage(), compositor::lock::Stage::Client);
        // While locked, the log goes on being written to disk.
        wait(
            &mut h,
            compositor::lock::LOG_SYNC + Duration::from_millis(200),
        );
        assert!(h.state.log_sync_requests() > syncs + 1);
    }

    #[test]
    fn the_console_switch_works_while_locked() {
        let mut h = Harness::start(&["sleep", "5"], &["false"]);
        let switched = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = switched.clone();
        h.state
            .set_switch_vt(Box::new(move |vt| seen.borrow_mut().push(vt)));
        let locker = h.connect_program();
        let _locking = h.lock(locker).unwrap();
        h.show_frame();
        h.run_until("locked", |h| h.peers[locker].seen.locked);
        h.settle();
        h.peers[locker].seen.keys.clear();
        // Ctrl + Alt + F2.
        const KEY_LEFTCTRL: u32 = 29 + 8;
        const KEY_LEFTALT: u32 = 56 + 8;
        const KEY_F2: u32 = 60 + 8;
        for code in [KEY_LEFTCTRL, KEY_LEFTALT] {
            h.state
                .keyboard_key(Keycode::new(code), KeyState::Pressed, 10);
        }
        h.key(KEY_F2);
        for code in [KEY_LEFTALT, KEY_LEFTCTRL] {
            h.state
                .keyboard_key(Keycode::new(code), KeyState::Released, 11);
        }
        h.settle();
        assert_eq!(*switched.borrow(), [2]);
        assert!(h.state.is_locked(), "the session stays locked");
        assert!(
            !h.peers[locker].seen.keys.contains(&evdev(KEY_F2)),
            "the switch goes to no client"
        );
    }
}
