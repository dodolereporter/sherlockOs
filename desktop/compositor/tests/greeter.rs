//! The compositor of the login screen (`--greeter`, lot D-24, ADR 0026), without a screen: a
//! test client plays the greeter. Its window covers the screen from its first configure, every
//! key of the desktop reaches it, and nothing of the desktop opens or locks.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::net::UnixStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compositor::State;
    use smithay::{backend::input::KeyState, input::keyboard::Keycode, reexports::calloop};
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_compositor::WlCompositor,
            wl_keyboard::{self, WlKeyboard},
            wl_registry,
            wl_seat::WlSeat,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::{self, XdgToplevel},
        xdg_wm_base::{self, XdgWmBase},
    };

    // xkb codes: the evdev codes of linux/input-event-codes.h, plus 8.
    const KEY_SUPER: u32 = 125 + 8;
    const KEY_TAB: u32 = 15 + 8;
    const KEY_SPACE: u32 = 57 + 8;
    const KEY_L: u32 = 38 + 8;
    const KEY_F4: u32 = 62 + 8;
    const KEY_RIGHT: u32 = 106 + 8;

    /// What the greeter has received.
    #[derive(Default)]
    struct Seen {
        globals: Vec<(u32, String, u32)>,
        keys: Vec<u32>,
        /// The last configure of the window: size and states.
        toplevel: Option<(i32, i32, Vec<u8>)>,
        configured: bool,
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
                seen.configured = true;
            }
        }
    }

    impl Dispatch<XdgToplevel, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &XdgToplevel,
            event: xdg_toplevel::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } = event
            {
                seen.toplevel = Some((width, height, states));
            }
        }
    }

    delegate_noop!(Seen: WlCompositor);
    delegate_noop!(Seen: ignore WlSurface);
    delegate_noop!(Seen: ignore WlSeat);
    delegate_noop!(Seen: ignore WlShm);
    delegate_noop!(Seen: WlShmPool);
    delegate_noop!(Seen: ignore WlBuffer);

    struct Harness {
        event_loop: calloop::EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Seen>,
        registry: wl_registry::WlRegistry,
        seen: Seen,
        /// Keeps the buffer's memory alive.
        _file: Option<File>,
    }

    impl Harness {
        /// A login screen of 1366 × 768, the smallest laptop of the brief, and its greeter
        /// connected.
        fn start() -> Self {
            let event_loop = calloop::EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.enter_greeter_mode();
            state.add_output((1366, 768).into(), 1.0);
            let (server_side, client_side) = UnixStream::pair().unwrap();
            state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            let mut h = Self {
                event_loop,
                state,
                conn,
                queue,
                registry,
                seen: Seen::default(),
                _file: None,
            };
            h.run_until("the globals", |h| !h.seen.globals.is_empty());
            h
        }

        fn bind<I: Proxy + 'static>(&self) -> I
        where
            Seen: Dispatch<I, ()>,
        {
            let name = I::interface().name;
            let (id, _, version) = self.seen.globals.iter().find(|g| g.1 == name).unwrap();
            self.registry.bind(
                *id,
                (*version).min(I::interface().version),
                &self.queue.handle(),
                (),
            )
        }

        fn run_until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
            for _ in 0..1000 {
                let _ = self.conn.flush();
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if let Some(guard) = self.queue.prepare_read() {
                    let _ = guard.read();
                }
                let _ = self.queue.dispatch_pending(&mut self.seen);
                if done(self) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        fn settle(&mut self) {
            let mut turns = 0;
            self.run_until("nothing", |_| {
                turns += 1;
                turns > 20
            });
        }

        /// The greeter opens its window and listens to the keyboard; it draws once
        /// configured, at the size it was given.
        fn show_greeter(&mut self) -> XdgToplevel {
            let qh = self.queue.handle();
            let _keyboard = self.bind::<WlSeat>().get_keyboard(&qh, ());
            let surface = self.bind::<WlCompositor>().create_surface(&qh, ());
            let toplevel = self
                .bind::<XdgWmBase>()
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            surface.commit();
            self.run_until("the first configure", |h| h.seen.configured);
            let (w, h, _) = self.seen.toplevel.clone().unwrap();
            self.draw(&surface, w, h);
            self.run_until("the greeter shown", |h| h.state.focused_window().is_some());
            toplevel
        }

        fn draw(&mut self, surface: &WlSurface, w: i32, h: i32) {
            let qh = self.queue.handle();
            let file = shm_file((w * h * 4) as u64);
            let pool = self
                .bind::<WlShm>()
                .create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self._file = Some(file);
        }

        fn chord(&mut self, held: u32, code: u32) {
            for (key, state) in [
                (held, KeyState::Pressed),
                (code, KeyState::Pressed),
                (code, KeyState::Released),
                (held, KeyState::Released),
            ] {
                self.state.keyboard_key(Keycode::new(key), state, 10);
            }
        }
    }

    fn shm_file(size: u64) -> File {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("greeter-test-{}-{n}", std::process::id());
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

    /// The state `fullscreen` of xdg-shell, in the array of a configure (native-endian u32s).
    fn fullscreen(states: &[u8]) -> bool {
        let fullscreen = xdg_toplevel::State::Fullscreen as u32;
        let (words, _) = states.as_chunks::<4>();
        words.iter().any(|c| u32::from_ne_bytes(*c) == fullscreen)
    }

    #[test]
    fn the_greeter_covers_the_screen_from_its_first_configure() {
        let mut h = Harness::start();
        let toplevel = h.show_greeter();
        let (w, height, states) = h.seen.toplevel.clone().unwrap();
        assert_eq!((w, height), (1366, 768));
        assert!(fullscreen(&states), "{states:?}");
        let window = h.state.focused_window().unwrap();
        let geometry = h.state.space.element_geometry(&window).unwrap();
        assert_eq!(geometry.loc, (0, 0).into());
        // It stays so, even if it asks to leave fullscreen.
        toplevel.unset_fullscreen();
        h.settle();
        let (_, _, states) = h.seen.toplevel.clone().unwrap();
        assert!(fullscreen(&states), "{states:?}");
    }

    #[test]
    fn every_key_of_the_desktop_goes_to_the_greeter() {
        let mut h = Harness::start();
        h.show_greeter();
        h.seen.keys.clear();
        for key in [KEY_TAB, KEY_SPACE, KEY_L, KEY_RIGHT] {
            h.chord(KEY_SUPER, key);
        }
        h.state
            .keyboard_key(Keycode::new(KEY_F4), KeyState::Pressed, 12);
        h.state
            .keyboard_key(Keycode::new(KEY_F4), KeyState::Released, 13);
        // Super alone, which opens Mission Control on the desktop.
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Pressed, 14);
        h.state
            .keyboard_key(Keycode::new(KEY_SUPER), KeyState::Released, 15);
        h.settle();
        assert!(!h.state.overview_open());
        assert!(!h.state.is_locked());
        // Every press and release reached the greeter: four chords of four, F4 and Super.
        assert_eq!(h.seen.keys.len(), 4 * 4 + 2 + 2, "{:?}", h.seen.keys);
        // The window was not snapped by Super + Right.
        let window = h.state.focused_window().unwrap();
        let geometry = h.state.space.element_geometry(&window).unwrap();
        assert_eq!(geometry.size, (1366, 768).into());
    }

    #[test]
    fn nothing_locks_the_login_screen() {
        let mut h = Harness::start();
        h.show_greeter();
        h.state.lock_session();
        assert!(!h.state.is_locked());
    }

    /// Runs the loop until `done`, for 10 s at most.
    fn wait(h: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(10);
        while !done(h) {
            assert!(
                std::time::Instant::now() < end,
                "timed out waiting for: {what}"
            );
            h.settle();
        }
    }

    #[test]
    fn the_login_screen_goes_dark_after_the_veil_and_never_locks() {
        let mut h = Harness::start();
        h.show_greeter();
        let handle = h.event_loop.handle();
        h.state.watch_idle(&handle, None).unwrap();
        h.state
            .set_screen_off_delay(Some(Duration::from_millis(1500)));
        wait(&mut h, "the veil", |h| h.state.dim().0 > 0.0);
        assert!(!h.state.is_locked());
        wait(&mut h, "the screens dark", |h| h.state.screens_dark());
        assert!(!h.state.is_locked());
        // A key brings them back, and does not reach the greeter.
        h.seen.keys.clear();
        h.state
            .keyboard_key(Keycode::new(KEY_L), KeyState::Pressed, 20);
        h.state
            .keyboard_key(Keycode::new(KEY_L), KeyState::Released, 21);
        h.settle();
        assert!(!h.state.screens_dark() && !h.state.is_locked());
        assert!(h.seen.keys.is_empty(), "{:?}", h.seen.keys);
    }

    #[test]
    fn the_login_screen_opens_no_shell_socket() {
        let dir = std::env::temp_dir().join(format!("greeter-socket-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("wayland-greeter");
        let event_loop = calloop::EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.enter_greeter_mode();
        assert!(state.is_greeter());
        state.listen_at(socket.clone(), &event_loop).unwrap();
        assert!(socket.exists());
        assert!(!shell_protocol::socket_path(&socket).exists());
        state.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
