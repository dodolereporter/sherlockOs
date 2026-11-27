//! What keeps the session awake (lot D-43), without a screen: the inactivity delay is short, and
//! the session must not lock while a video inhibits it. A Wayland client on a socket pair holds an
//! idle inhibitor; a client of a private bus (its own `dbus-daemon`) takes a cookie of
//! `org.freedesktop.ScreenSaver`; a fake logind on that bus blocks `idle`.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        io::{BufRead as _, BufReader},
        os::{fd::AsFd, unix::net::UnixStream},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use compositor::{State, lock::Programs};
    use smithay::{
        backend::input::KeyState, desktop::Window, input::keyboard::Keycode,
        reexports::calloop::EventLoop,
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_registry, wl_shm::Format,
            wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
        },
    };
    use wayland_protocols::{
        wp::idle_inhibit::zv1::client::{
            zwp_idle_inhibit_manager_v1::ZwpIdleInhibitManagerV1,
            zwp_idle_inhibitor_v1::ZwpIdleInhibitorV1,
        },
        xdg::shell::client::{
            xdg_surface::{self, XdgSurface},
            xdg_toplevel::XdgToplevel,
            xdg_wm_base::{self, XdgWmBase},
        },
    };

    // xkb codes: the evdev codes plus 8.
    const KEY_SUPER: u32 = 125 + 8;
    const KEY_L: u32 = 38 + 8;
    /// Inactivity before the lock: the check runs every second.
    const IDLE: Duration = Duration::from_millis(300);
    /// Long enough for the lock to come several times over, were nothing inhibiting.
    const AWAKE: Duration = Duration::from_millis(2500);
    const BUDGET: Duration = Duration::from_secs(15);

    #[derive(Default)]
    struct Seen {
        globals: Vec<(u32, String, u32)>,
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

    delegate_noop!(Seen: WlCompositor);
    delegate_noop!(Seen: ignore WlSurface);
    delegate_noop!(Seen: ignore WlShm);
    delegate_noop!(Seen: WlShmPool);
    delegate_noop!(Seen: ignore WlBuffer);
    delegate_noop!(Seen: ignore XdgToplevel);
    delegate_noop!(Seen: ZwpIdleInhibitManagerV1);
    delegate_noop!(Seen: ZwpIdleInhibitorV1);

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        client: Option<(Connection, EventQueue<Seen>, wl_registry::WlRegistry)>,
        seen: Seen,
        _file: Option<File>,
    }

    impl Harness {
        /// One screen, the inactivity lock after `IDLE`, no lock program.
        fn start() -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            state.set_lock_programs(Programs {
                client: Vec::new(),
                rescue: None,
                swaylock: None,
            });
            state.watch_idle(&event_loop.handle(), None).unwrap();
            state.set_idle_lock(Some(IDLE));
            Self {
                event_loop,
                state,
                client: None,
                seen: Seen::default(),
                _file: None,
            }
        }

        fn pump(&mut self) {
            if let Some((conn, ..)) = &self.client {
                let _ = conn.flush();
            }
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .unwrap();
            self.state.refresh();
            if let Some((conn, queue, _)) = &mut self.client {
                let _ = conn.flush();
                if let Some(guard) = queue.prepare_read() {
                    let _ = guard.read();
                }
                queue.dispatch_pending(&mut self.seen).unwrap();
            }
        }

        fn wait(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + BUDGET;
            while !done(self) {
                assert!(Instant::now() < deadline, "timed out waiting for: {what}");
                self.pump();
            }
        }

        /// Runs `time` without input, and checks the session did not lock.
        fn stays_awake(&mut self, what: &str) {
            let end = Instant::now() + AWAKE;
            while Instant::now() < end {
                self.pump();
                assert!(!self.state.is_locked(), "locked despite {what}");
            }
        }

        fn bind<I: Proxy + 'static>(&self) -> I
        where
            Seen: Dispatch<I, ()>,
        {
            let (_, queue, registry) = self.client.as_ref().unwrap();
            let name = I::interface().name;
            let (id, _, version) = self.seen.globals.iter().find(|g| g.1 == name).unwrap();
            registry.bind(
                *id,
                (*version).min(I::interface().version),
                &queue.handle(),
                (),
            )
        }

        /// A Wayland client shows a window holding an idle inhibitor, as a video player does.
        fn show_video(&mut self) -> Window {
            let (server_side, client_side) = UnixStream::pair().unwrap();
            self.state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            self.client = Some((conn, queue, registry));
            self.wait("the globals", |h| !h.seen.globals.is_empty());
            let qh = self.client.as_ref().unwrap().1.handle();
            let surface = self.bind::<WlCompositor>().create_surface(&qh, ());
            let toplevel = self
                .bind::<XdgWmBase>()
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title("video".into());
            surface.commit();
            self.wait("the configure", |h| h.seen.configured);
            let (w, h) = (100, 80);
            let file = shm_file((w * h * 4) as u64);
            let pool = self
                .bind::<WlShm>()
                .create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self._file = Some(file);
            self.bind::<ZwpIdleInhibitManagerV1>()
                .create_inhibitor(&surface, &qh, ());
            self.wait("the inhibitor", |h| h.state.idle_inhibited());
            self.state.space.elements().next().cloned().unwrap()
        }

        fn super_l(&mut self) {
            for (code, state) in [
                (KEY_SUPER, KeyState::Pressed),
                (KEY_L, KeyState::Pressed),
                (KEY_L, KeyState::Released),
                (KEY_SUPER, KeyState::Released),
            ] {
                self.state.keyboard_key(Keycode::new(code), state, 1);
            }
        }
    }

    fn shm_file(size: u64) -> File {
        // The tests run side by side: one name each.
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("idle-inhibit-{}-{n}", std::process::id());
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

    /// A private bus: its own `dbus-daemon` in a test folder, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn bus(name: &str) -> Bus {
        let dir = std::env::temp_dir().join(format!("idle-inhibit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>unix:dir={}</listen>\
                 <auth>EXTERNAL</auth><policy context=\"default\">\
                 <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                 <allow own=\"*\"/></policy></busconfig>",
                dir.display()
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .env_clear()
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon (package dbus-daemon) is needed by these tests");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Bus {
            daemon,
            address: address.trim().to_owned(),
            dir,
        }
    }

    fn connect(bus: &Bus) -> zbus::blocking::Connection {
        zbus::blocking::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .build()
            .unwrap()
    }

    /// logind as the test wants it: what it blocks.
    struct Logind(Arc<Mutex<String>>);

    #[zbus::interface(name = "org.freedesktop.login1.Manager")]
    impl Logind {
        #[zbus(property)]
        fn block_inhibited(&self) -> String {
            self.0.lock().unwrap().clone()
        }
    }

    #[test]
    fn a_visible_video_keeps_the_session_awake_not_a_minimized_one() {
        let mut h = Harness::start();
        let video = h.show_video();
        h.stays_awake("a visible video");
        // Minimized, it no longer counts: the inactivity starts, and locks.
        h.state.minimize(&video);
        assert!(!h.state.idle_inhibited());
        h.wait("the lock after inactivity", |h| h.state.is_locked());
    }

    #[test]
    fn super_l_locks_despite_a_video() {
        let mut h = Harness::start();
        h.show_video();
        h.super_l();
        h.wait("the lock of Super + L", |h| h.state.is_locked());
    }

    #[test]
    fn a_cookie_of_the_bus_keeps_awake_until_its_holder_leaves() {
        let bus = bus("screensaver");
        let mut h = Harness::start();
        let handle = h.event_loop.handle();
        // logind is absent from this bus: its thread gives up, the service goes on.
        h.state
            .watch_inhibitors(
                &handle,
                Some(bus.address.clone()),
                Some(bus.address.clone()),
            )
            .unwrap();
        let player = connect(&bus);
        let proxy = zbus::blocking::Proxy::new(
            &player,
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver",
        )
        .unwrap();
        let call = |proxy: &zbus::blocking::Proxy<'_>| -> u32 {
            let deadline = Instant::now() + BUDGET;
            loop {
                match proxy.call("Inhibit", &("video", "Playing a video")) {
                    Ok(cookie) => return cookie,
                    // The service may not have its name yet.
                    Err(_) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(err) => panic!("Inhibit: {err}"),
                }
            }
        };
        let cookie = call(&proxy);
        assert_ne!(cookie, 0);
        h.wait("the cookie", |h| h.state.idle_inhibited());
        h.stays_awake("a cookie of the bus");
        // Another program cannot give it back.
        let other = connect(&bus);
        let proxy_other = zbus::blocking::Proxy::new(
            &other,
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver",
        )
        .unwrap();
        let () = proxy_other.call("UnInhibit", &(cookie,)).unwrap();
        h.stays_awake("a cookie given back by another program");
        // The player leaves the bus without giving it back: it goes, and the session locks.
        drop(proxy);
        drop(player);
        h.wait("the cookie gone", |h| !h.state.idle_inhibited());
        h.wait("the lock after inactivity", |h| h.state.is_locked());
    }

    #[test]
    fn logind_blocking_idle_keeps_awake() {
        let bus = bus("logind");
        let blocked = Arc::new(Mutex::new(String::from("sleep:idle")));
        let _logind = zbus::blocking::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .serve_at("/org/freedesktop/login1", Logind(blocked.clone()))
            .unwrap()
            .name("org.freedesktop.login1")
            .unwrap()
            .build()
            .unwrap();
        let mut h = Harness::start();
        let handle = h.event_loop.handle();
        h.state
            .watch_inhibitors(
                &handle,
                Some(bus.address.clone()),
                Some(bus.address.clone()),
            )
            .unwrap();
        h.wait("logind read", |h| h.state.idle_inhibited());
        h.stays_awake("logind blocking idle");
        *blocked.lock().unwrap() = String::from("sleep");
        h.wait("logind no longer blocking", |h| !h.state.idle_inhibited());
        h.wait("the lock after inactivity", |h| h.state.is_locked());
    }
}
