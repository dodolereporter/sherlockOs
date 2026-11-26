//! The shell protocol (`sherlock_shell_v1`) and the workspaces (`ext-workspace-v1`): only the
//! clients of the shell socket see them, the windows and screens they describe, and the
//! requests on windows. Everything runs in one thread, the compositor loop and two clients
//! taking turns: an application on the ordinary socket, and a shell program.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::net::UnixStream},
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compositor::State;
    use shell_protocol::client::{
        sherlock_shell_manager_v1::{self, SherlockShellManagerV1},
        sherlock_shell_toplevel_v1::{self, SherlockShellToplevelV1, State as Flag},
    };
    use smithay::{
        backend::input::{ButtonState, KeyState},
        input::keyboard::Keycode,
        reexports::calloop::EventLoop,
        utils::Rectangle,
        wayland::seat::WaylandFocus,
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop, event_created_child,
        protocol::{
            wl_buffer::WlBuffer,
            wl_compositor::WlCompositor,
            wl_output::WlOutput,
            wl_registry,
            wl_seat::WlSeat,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::ext::workspace::v1::client::{
        ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1},
        ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1},
        ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::{self, XdgToplevel},
        xdg_wm_base::{self, XdgWmBase},
    };
    use wayland_protocols_wlr::layer_shell::v1::client::{
        zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
        zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
    };

    const MANAGER: &str = "sherlock_shell_manager_v1";
    const WORKSPACES: &str = "ext_workspace_manager_v1";
    const BTN_LEFT: u32 = 0x110;
    // xkb codes (evdev + 8): left Super, Shift and Ctrl, the arrows, and 1 to 3 of the top row.
    const KEY_SUPER: u32 = 133;
    const KEY_SHIFT: u32 = 50;
    const KEY_CTRL: u32 = 37;
    const KEY_LEFT: u32 = 113;
    const KEY_RIGHT: u32 = 114;
    const KEY_1: u32 = 10;
    const KEY_2: u32 = 11;
    const KEY_3: u32 = 12;

    /// A window as the shell client knows it, applied at `done`.
    #[derive(Debug, Default, Clone)]
    struct Seen {
        id: Option<u32>,
        title: String,
        app_id: String,
        states: Vec<u32>,
        output: String,
        workspace: String,
        geometry: (i32, i32, i32, i32),
        closed: bool,
    }

    /// A workspace as the shell client knows it: id, name, coordinates, active.
    #[derive(Debug, Default, Clone, PartialEq)]
    struct SeenWorkspace {
        id: String,
        name: String,
        coordinates: Vec<u32>,
        active: bool,
    }

    /// What a client has received. One type for both clients: each one only receives the
    /// events of what it created.
    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        configured: bool,
        close_asked: bool,
        layer_configured: bool,
        /// Toplevels of the shell protocol, in the order they came.
        toplevels: Vec<(SherlockShellToplevelV1, Seen)>,
        /// Screens and their state.
        outputs: Vec<(String, Vec<u32>)>,
        removed_outputs: Vec<String>,
        done: usize,
        /// Workspaces: groups, outputs entered, each workspace.
        groups: Vec<ExtWorkspaceGroupHandleV1>,
        group_outputs: usize,
        workspaces: Vec<(ExtWorkspaceHandleV1, SeenWorkspace)>,
        workspaces_done: usize,
    }

    fn values(bytes: &[u8]) -> Vec<u32> {
        let (chunks, _) = bytes.as_chunks::<4>();
        chunks.iter().map(|c| u32::from_ne_bytes(*c)).collect()
    }

    impl Client {
        fn window(&self, id: u32) -> &Seen {
            &self
                .toplevels
                .iter()
                .find(|t| t.1.id == Some(id))
                .unwrap()
                .1
        }

        fn handle(&self, id: u32) -> &SherlockShellToplevelV1 {
            &self
                .toplevels
                .iter()
                .find(|t| t.1.id == Some(id))
                .unwrap()
                .0
        }

        fn has(&self, id: u32, flag: Flag) -> bool {
            self.window(id).states.contains(&(flag as u32))
        }

        fn output(&self, name: &str) -> Option<&Vec<u32>> {
            self.outputs.iter().find(|o| o.0 == name).map(|o| &o.1)
        }
    }

    impl Dispatch<SherlockShellManagerV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &SherlockShellManagerV1,
            event: sherlock_shell_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use sherlock_shell_manager_v1::Event;
            match event {
                Event::Toplevel { toplevel } => client.toplevels.push((toplevel, Seen::default())),
                Event::Output { name, state } => {
                    client.outputs.retain(|o| o.0 != name);
                    client.outputs.push((name, values(&state)));
                }
                Event::OutputRemoved { name } => {
                    client.outputs.retain(|o| o.0 != name);
                    client.removed_outputs.push(name);
                }
                Event::Done => client.done += 1,
                _ => {}
            }
        }

        event_created_child!(Client, SherlockShellManagerV1, [
            sherlock_shell_manager_v1::EVT_TOPLEVEL_OPCODE => (SherlockShellToplevelV1, ()),
        ]);
    }

    impl Dispatch<SherlockShellToplevelV1, ()> for Client {
        fn event(
            client: &mut Self,
            proxy: &SherlockShellToplevelV1,
            event: sherlock_shell_toplevel_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use sherlock_shell_toplevel_v1::Event;
            let seen = &mut client
                .toplevels
                .iter_mut()
                .find(|t| &t.0 == proxy)
                .unwrap()
                .1;
            match event {
                Event::Id { id } => seen.id = Some(id),
                Event::Title { title } => seen.title = title,
                Event::AppId { app_id } => seen.app_id = app_id,
                Event::State { state } => seen.states = values(&state),
                Event::Output { name } => seen.output = name,
                Event::Workspace { workspace } => seen.workspace = workspace,
                Event::Geometry {
                    x,
                    y,
                    width,
                    height,
                } => seen.geometry = (x, y, width, height),
                Event::Closed => seen.closed = true,
                _ => {}
            }
        }
    }

    impl Dispatch<ExtWorkspaceManagerV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ExtWorkspaceManagerV1,
            event: ext_workspace_manager_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use ext_workspace_manager_v1::Event;
            match event {
                Event::WorkspaceGroup { workspace_group } => client.groups.push(workspace_group),
                Event::Workspace { workspace } => {
                    client
                        .workspaces
                        .push((workspace, SeenWorkspace::default()));
                }
                Event::Done => client.workspaces_done += 1,
                _ => {}
            }
        }

        event_created_child!(Client, ExtWorkspaceManagerV1, [
            ext_workspace_manager_v1::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
            ext_workspace_manager_v1::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
        ]);
    }

    impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ExtWorkspaceGroupHandleV1,
            event: ext_workspace_group_handle_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let ext_workspace_group_handle_v1::Event::OutputEnter { .. } = event {
                client.group_outputs += 1;
            }
        }
    }

    impl Dispatch<ExtWorkspaceHandleV1, ()> for Client {
        fn event(
            client: &mut Self,
            handle: &ExtWorkspaceHandleV1,
            event: ext_workspace_handle_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use ext_workspace_handle_v1::{Event, State as WorkspaceState};
            if let Event::Removed = event {
                client.workspaces.retain(|w| &w.0 != handle);
                return;
            }
            let workspace = &mut client
                .workspaces
                .iter_mut()
                .find(|w| &w.0 == handle)
                .unwrap()
                .1;
            match event {
                Event::Id { id } => workspace.id = id,
                Event::Name { name } => workspace.name = name,
                Event::Coordinates { coordinates } => workspace.coordinates = values(&coordinates),
                Event::State { state } => {
                    workspace.active = state
                        .into_result()
                        .is_ok_and(|s| s.contains(WorkspaceState::Active));
                }
                _ => {}
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
            client: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
                client.configured = true;
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
                client.close_asked = true;
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
            if let zwlr_layer_surface_v1::Event::Configure { serial, .. } = event {
                surface.ack_configure(serial);
                client.layer_configured = true;
            }
        }
    }

    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlSeat);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: ignore WlOutput);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);
    delegate_noop!(Client: ZwlrLayerShellV1);

    /// A connection, its queue and what it received.
    struct Side {
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        registry: wl_registry::WlRegistry,
        /// Buffer files, alive as long as their buffers.
        files: Vec<File>,
    }

    impl Side {
        fn new(stream: UnixStream) -> Self {
            let conn = Connection::from_socket(stream).unwrap();
            let queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            Self {
                conn,
                queue,
                client: Client::default(),
                registry,
                files: Vec::new(),
            }
        }

        fn sees(&self, interface: &str) -> bool {
            self.client.globals.iter().any(|g| g.1 == interface)
        }

        fn bind<I: Proxy + 'static>(&self) -> I
        where
            Client: Dispatch<I, ()>,
        {
            let name = I::interface().name;
            let (id, _, version) = self.client.globals.iter().find(|g| g.1 == name).unwrap();
            self.registry.bind(
                *id,
                (*version).min(I::interface().version),
                &self.queue.handle(),
                (),
            )
        }

        /// A buffer of `w` × `h` attached and committed on `surface`.
        fn draw(&mut self, surface: &WlSurface, w: i32, h: i32) {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("compositor-shell-test-{}-{n}", std::process::id()));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            std::fs::remove_file(&path).unwrap();
            file.set_len((w * h * 4) as u64).unwrap();
            let qh = self.queue.handle();
            let shm: WlShm = self.bind();
            let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self.files.push(file);
        }

        /// Reads what came. A dead connection still holds the error that killed it, which
        /// `Connection::protocol_error` gives afterwards.
        fn read(&mut self) {
            let _ = self.conn.flush();
            if let Some(guard) = self.queue.prepare_read() {
                // Nothing to read is not an error: try again on the next turn.
                let _ = guard.read();
            }
            let _ = self.queue.dispatch_pending(&mut self.client);
        }
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        app: Side,
        shell: Side,
    }

    impl Harness {
        /// One screen of 800 × 600, an application client and a shell client.
        fn start() -> Self {
            Self::start_with(1)
        }

        /// `outputs` screens of 800 × 600, an application client and a shell client.
        fn start_with(outputs: usize) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            for _ in 0..outputs {
                state.add_output((800, 600).into(), 1.0);
            }
            let (server, app) = UnixStream::pair().unwrap();
            state.insert_client(server);
            let (server, shell) = UnixStream::pair().unwrap();
            state.insert_shell_client(server);
            let mut h = Self {
                event_loop,
                state,
                app: Side::new(app),
                shell: Side::new(shell),
            };
            h.run_until("the globals", |h| {
                h.app.sees("wl_shm") && h.shell.sees("wl_shm")
            });
            h
        }

        fn turn(&mut self) {
            self.app.conn.flush().ok();
            self.shell.conn.flush().ok();
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .unwrap();
            self.state.refresh();
            self.app.read();
            self.shell.read();
        }

        fn run_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            for _ in 0..200 {
                self.turn();
                if done(self) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        /// The application opens a titled window of 100 × 80, shown in the center.
        fn open_window(&mut self, title: &str) -> (XdgToplevel, WlSurface) {
            let qh = self.app.queue.handle();
            let compositor: WlCompositor = self.app.bind();
            let wm_base: XdgWmBase = self.app.bind();
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title(title.into());
            toplevel.set_app_id(String::from("org.test.App"));
            self.app.client.configured = false;
            surface.commit();
            self.run_until("the first configure", |h| h.app.client.configured);
            self.app.draw(&surface, 100, 80);
            self.run_until("the window shown", |h| {
                h.state
                    .focused_window()
                    .is_some_and(|w| State::title(&w).as_deref() == Some(title))
            });
            (toplevel, surface)
        }

        /// Binds the shell manager and waits for its first state.
        fn bind_manager(&mut self) -> SherlockShellManagerV1 {
            let manager: SherlockShellManagerV1 = self.shell.bind();
            let done = self.shell.client.done;
            self.run_until("the first done", |h| h.shell.client.done > done);
            manager
        }

        /// Waits for a `done` after the requests just sent.
        fn next_done(&mut self, what: &str) {
            let done = self.shell.client.done;
            self.run_until(what, |h| h.shell.client.done > done);
        }

        fn only_window(&self) -> smithay::desktop::Window {
            self.state.space.elements().next().unwrap().clone()
        }

        /// The window titled `title`.
        fn titled(&self, title: &str) -> smithay::desktop::Window {
            let mut windows = self.state.space.elements();
            let window = windows.find(|w| State::title(w).as_deref() == Some(title));
            window.unwrap().clone()
        }

        /// `key` pressed and released while `held` are held, then a few turns.
        fn keys(&mut self, held: &[u32], key: u32) {
            let mut time = 0;
            let mut press = |h: &mut Self, code, state| {
                time += 1;
                h.state.keyboard_key(Keycode::new(code), state, time);
            };
            for code in held {
                press(self, *code, KeyState::Pressed);
            }
            press(self, key, KeyState::Pressed);
            press(self, key, KeyState::Released);
            for code in held.iter().rev() {
                press(self, *code, KeyState::Released);
            }
            for _ in 0..5 {
                self.turn();
            }
        }

        fn focused_title(&self) -> Option<String> {
            self.state.focused_window().and_then(|w| State::title(&w))
        }
    }

    #[test]
    fn an_ordinary_client_neither_sees_nor_binds_the_shell_globals() {
        let mut h = Harness::start();
        assert!(h.shell.sees(MANAGER) && h.shell.sees(WORKSPACES));
        assert!(
            !h.app.sees(MANAGER),
            "the application sees the shell manager"
        );
        assert!(
            !h.app.sees(WORKSPACES),
            "the application sees the workspaces"
        );

        // Global names are shared by every client: the application tries the shell's.
        let (name, _, _) = h
            .shell
            .client
            .globals
            .iter()
            .find(|g| g.1 == MANAGER)
            .cloned()
            .unwrap();
        let _: SherlockShellManagerV1 = h.app.registry.bind(name, 1, &h.app.queue.handle(), ());
        for _ in 0..50 {
            h.turn();
        }
        // The compositor answers with wl_display.error invalid_object, which libwayland-client
        // turns into EINVAL on the connection (wl_display_get_error).
        let error = h.app.queue.roundtrip(&mut h.app.client).unwrap_err();
        let os_error = match error {
            wayland_client::DispatchError::Backend(wayland_client::backend::WaylandError::Io(
                e,
            )) => e.raw_os_error(),
            other => panic!("unexpected error: {other:?}"),
        };
        assert_eq!(
            os_error,
            Some(22),
            "not EINVAL: the bind was not refused as invalid"
        );
        assert!(h.app.client.toplevels.is_empty());
        // The shell client is not affected.
        h.bind_manager();
        assert!(h.shell.conn.protocol_error().is_none());
    }

    #[test]
    fn the_sockets_sort_the_clients() {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), 1.0);
        let path: PathBuf =
            std::env::temp_dir().join(format!("compositor-shell-{}", std::process::id()));
        state.listen_at(path.clone(), &event_loop).unwrap();
        let shell_path = shell_protocol::socket_path(&path);
        assert!(
            shell_path.exists(),
            "no shell socket next to the Wayland socket"
        );
        let mut h = Harness {
            event_loop,
            state,
            app: Side::new(UnixStream::connect(&path).unwrap()),
            shell: Side::new(UnixStream::connect(&shell_path).unwrap()),
        };
        h.run_until("the globals", |h| {
            h.app.sees("wl_shm") && h.shell.sees("wl_shm")
        });
        assert!(h.shell.sees(MANAGER) && h.shell.sees(WORKSPACES));
        assert!(!h.app.sees(MANAGER) && !h.app.sees(WORKSPACES));
        h.state.shutdown();
        assert!(!path.exists() && !shell_path.exists());
    }

    #[test]
    fn the_shell_follows_the_windows_and_acts_on_them() {
        let mut h = Harness::start();
        let manager = h.bind_manager();
        assert!(h.shell.client.toplevels.is_empty());
        assert_eq!(h.shell.client.output("virtual-1"), Some(&Vec::new()));

        // A new window: its id, title, app_id, state, screen and place in the screen.
        let (toplevel, surface) = h.open_window("First");
        h.run_until("the window listed", |h| h.shell.client.toplevels.len() == 1);
        let seen = h.shell.client.toplevels[0].1.clone();
        let id = seen.id.unwrap();
        assert_eq!(seen.title, "First");
        assert_eq!(seen.app_id, "org.test.App");
        assert_eq!(seen.states, vec![Flag::Activated as u32]);
        assert_eq!(seen.output, "virtual-1");
        assert_eq!(seen.geometry, (350, 260, 100, 80));

        // A second binding of the manager gets the same id for the same window.
        let _second: SherlockShellManagerV1 = h.shell.bind();
        h.run_until("the second list", |h| h.shell.client.toplevels.len() == 2);
        assert_eq!(h.shell.client.toplevels[1].1.id, Some(id));

        // Title and maximize: the window's state, and the screen's.
        toplevel.set_title(String::from("Renamed"));
        toplevel.set_maximized();
        surface.commit();
        h.run_until("maximized", |h| {
            h.shell.client.has(id, Flag::Maximized)
                && h.shell.client.output("virtual-1")
                    == Some(&vec![
                        sherlock_shell_manager_v1::OutputState::Maximized as u32,
                    ])
        });
        assert_eq!(h.shell.client.window(id).title, "Renamed");
        assert_eq!(h.shell.client.window(id).geometry.0, 0);

        // Minimized by the shell: hidden, no longer active, the screen uncovered.
        let window = h.only_window();
        h.shell.client.handle(id).minimize();
        h.next_done("minimized");
        assert!(h.shell.client.has(id, Flag::Minimized));
        assert!(!h.shell.client.has(id, Flag::Activated));
        assert_eq!(h.shell.client.output("virtual-1"), Some(&Vec::new()));
        assert!(!h.state.is_visible(&window));
        assert!(h.state.focused_window().is_none());
        assert!(h.state.is_minimized(&window));
        assert_eq!(h.shell.client.window(id).output, "virtual-1");

        // Activated: back in its place, with the keyboard.
        h.shell.client.handle(id).activate();
        h.next_done("activated");
        assert!(h.shell.client.has(id, Flag::Activated));
        assert!(!h.shell.client.has(id, Flag::Minimized));
        assert_eq!(h.state.focused_window(), Some(window.clone()));
        assert_eq!(
            h.state.space.element_geometry(&window).unwrap().loc,
            (0, 0).into()
        );

        // The rectangle of the dock icon, kept for the minimize animation; 0 removes it.
        h.shell
            .client
            .handle(id)
            .set_rectangle("virtual-1".into(), 380, 540, 52, 52);
        h.run_until("the icon rectangle", |h| {
            h.state.icon_rectangle(&window)
                == Some((
                    String::from("virtual-1"),
                    Rectangle::new((380, 540).into(), (52, 52).into()),
                ))
        });
        h.shell
            .client
            .handle(id)
            .set_rectangle("virtual-1".into(), 0, 0, 0, 0);
        h.run_until("no icon rectangle", |h| {
            h.state.icon_rectangle(&window).is_none()
        });

        // Closing goes to the application, which closes its window: the shell sees it go.
        h.shell.client.handle(id).close();
        h.run_until("the close request", |h| h.app.client.close_asked);
        toplevel.destroy();
        h.run_until("the window closed", |h| h.shell.client.window(id).closed);
        manager.destroy();
    }

    #[test]
    fn the_active_window_stays_active_while_the_dock_has_the_keyboard() {
        let mut h = Harness::start();
        h.bind_manager();
        h.open_window("Behind");
        let (_front, _surface) = h.open_window("Front");
        h.run_until("both windows", |h| h.shell.client.toplevels.len() == 2);
        let front = h.shell.client.toplevels[1].1.id.unwrap();
        let behind = h.shell.client.toplevels[0].1.id.unwrap();
        assert!(h.shell.client.has(front, Flag::Activated));
        assert!(!h.shell.client.has(behind, Flag::Activated));

        // The dock: an anchored surface of the shell client, keyboard on demand, clicked.
        let qh = h.shell.queue.handle();
        let compositor: WlCompositor = h.shell.bind();
        let layer_shell: ZwlrLayerShellV1 = h.shell.bind();
        let surface = compositor.create_surface(&qh, ());
        let dock =
            layer_shell.get_layer_surface(&surface, None, Layer::Top, "dock".into(), &qh, ());
        dock.set_anchor(Anchor::Bottom);
        dock.set_size(200, 76);
        dock.set_keyboard_interactivity(KeyboardInteractivity::OnDemand);
        surface.commit();
        h.run_until("the dock configure", |h| h.shell.client.layer_configured);
        h.shell.draw(&surface, 200, 76);
        for _ in 0..5 {
            h.turn();
        }
        h.state.pointer_motion((400.0, 560.0).into(), 1);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 3);
        h.run_until("the keyboard on the dock", |h| {
            h.state.focused_window().is_none()
        });
        for _ in 0..5 {
            h.turn();
        }
        assert!(
            h.shell.client.has(front, Flag::Activated),
            "the click took the active state"
        );

        // The dock minimizes the active window: the one behind becomes active.
        h.shell.client.handle(front).minimize();
        h.run_until("the window behind active", |h| {
            h.shell.client.has(behind, Flag::Activated)
                && h.shell.client.has(front, Flag::Minimized)
        });
    }

    /// The shell client sends `set_rectangle` with these values: it must end with
    /// `invalid_rectangle`, and the compositor goes on.
    fn refused_rectangle(x: i32, y: i32, width: i32, height: i32) {
        let mut h = Harness::start();
        h.bind_manager();
        h.open_window("Window");
        h.run_until("the window listed", |h| {
            !h.shell.client.toplevels.is_empty()
        });
        let id = h.shell.client.toplevels[0].1.id.unwrap();
        let window = h.only_window();
        h.shell
            .client
            .handle(id)
            .set_rectangle("virtual-1".into(), x, y, width, height);
        for _ in 0..20 {
            h.turn();
        }
        let error = h.shell.conn.protocol_error().expect("no protocol error");
        assert_eq!(error.object_interface, "sherlock_shell_toplevel_v1");
        assert_eq!(
            error.code,
            sherlock_shell_toplevel_v1::Error::InvalidRectangle as u32
        );
        assert!(
            h.state.icon_rectangle(&window).is_none(),
            "the rectangle was kept"
        );
        // The compositor goes on: the application still has its window.
        h.turn();
        assert_eq!(h.state.space.elements().count(), 1);
    }

    #[test]
    fn a_negative_rectangle_is_a_protocol_error() {
        refused_rectangle(0, 0, -1, 10);
    }

    #[test]
    fn a_rectangle_past_the_32_bit_range_is_a_protocol_error() {
        refused_rectangle(i32::MAX, 0, 1, 10);
        refused_rectangle(0, i32::MAX - 5, 10, 6);
    }

    #[test]
    fn the_first_done_comes_even_without_screen_nor_window() {
        let mut h = Harness::start_with(0);
        h.bind_manager();
        assert!(h.shell.client.toplevels.is_empty() && h.shell.client.outputs.is_empty());
    }

    #[test]
    fn a_workspace_activation_waits_for_the_commit_of_its_own_client() {
        let mut h = Harness::start();
        h.state.set_workspace_count(2);
        let first: ExtWorkspaceManagerV1 = h.shell.bind();
        // A second shell client, with its own manager.
        let (server, stream) = UnixStream::pair().unwrap();
        h.state.insert_shell_client(server);
        let mut other = Side::new(stream);
        let turn = |h: &mut Harness, other: &mut Side| {
            other.conn.flush().ok();
            h.turn();
            other.read();
        };
        for _ in 0..200 {
            turn(&mut h, &mut other);
            if h.shell.client.workspaces.len() == 2 && other.sees(WORKSPACES) {
                break;
            }
        }
        let second: ExtWorkspaceManagerV1 = other.bind();
        for _ in 0..20 {
            turn(&mut h, &mut other);
        }
        assert_eq!(other.client.workspaces.len(), 2);
        assert_eq!(h.state.workspaces().active(), "1");

        // The first client activates workspace 2 without committing; the second commits.
        let two = &h
            .shell
            .client
            .workspaces
            .iter()
            .find(|w| w.1.id == "2")
            .unwrap()
            .0;
        two.activate();
        // The activation reaches the compositor before the other client's commit.
        for _ in 0..10 {
            turn(&mut h, &mut other);
        }
        second.commit();
        for _ in 0..10 {
            turn(&mut h, &mut other);
        }
        assert_eq!(
            h.state.workspaces().active(),
            "1",
            "another client's commit applied it"
        );

        // Its own commit applies it, and both clients see workspace 2 active.
        first.commit();
        for _ in 0..200 {
            turn(&mut h, &mut other);
            let active = |c: &Client| c.workspaces.iter().any(|w| w.1.id == "2" && w.1.active);
            if active(&h.shell.client) && active(&other.client) {
                break;
            }
        }
        assert_eq!(h.state.workspaces().active(), "2");
        assert!(
            other
                .client
                .workspaces
                .iter()
                .any(|w| w.1.id == "2" && w.1.active)
        );
        assert!(
            other
                .client
                .workspaces
                .iter()
                .any(|w| w.1.id == "1" && !w.1.active)
        );
    }

    #[test]
    fn each_screen_is_named_and_described() {
        let mut h = Harness::start();
        h.state.add_output((800, 600).into(), 1.0);
        h.bind_manager();
        assert!(h.shell.client.output("virtual-2").is_some());

        // A window moved to the second screen: its place is relative to that screen.
        h.open_window("Window");
        let window = h.only_window();
        h.state.space.map_element(window, (1000, 100), true);
        h.run_until("the window on the second screen", |h| {
            h.shell
                .client
                .toplevels
                .first()
                .is_some_and(|t| t.1.output == "virtual-2")
        });
        assert_eq!(h.shell.client.toplevels[0].1.geometry, (200, 100, 100, 80));
    }

    #[test]
    fn the_workspaces_are_shared_by_every_screen() {
        let mut h = Harness::start();
        let _output: WlOutput = h.shell.bind();
        let manager: ExtWorkspaceManagerV1 = h.shell.bind();
        h.run_until("the workspaces", |h| h.shell.client.workspaces_done > 0);
        assert_eq!(h.shell.client.groups.len(), 1);
        assert_eq!(
            h.shell.client.group_outputs, 1,
            "the group did not enter the output"
        );
        let first = SeenWorkspace {
            id: String::from("1"),
            name: String::from("1"),
            coordinates: vec![0],
            active: true,
        };
        assert_eq!(h.shell.client.workspaces.len(), 3, "three by default");
        assert_eq!(h.shell.client.workspaces[0].1, first);
        assert_eq!(h.shell.client.workspaces[2].1.coordinates, vec![2]);
        assert_eq!(h.state.workspaces().active(), "1");

        // A new screen joins the same group, once the client binds its wl_output.
        h.state.add_output((800, 600).into(), 1.0);
        h.run_until("the second wl_output", |h| {
            h.shell
                .client
                .globals
                .iter()
                .filter(|g| g.1 == "wl_output")
                .count()
                == 2
        });
        let (name, _, _) = h
            .shell
            .client
            .globals
            .iter()
            .rfind(|g| g.1 == "wl_output")
            .cloned()
            .unwrap();
        let _second: WlOutput = h.shell.registry.bind(name, 4, &h.shell.queue.handle(), ());
        h.run_until("the group on both screens", |h| {
            h.shell.client.group_outputs == 2
        });
        assert_eq!(h.shell.client.groups.len(), 1);
        assert_eq!(h.shell.client.workspaces.len(), 3);

        // Activating the active workspace, then committing: nothing changes, nothing breaks.
        h.shell.client.workspaces[0].0.activate();
        manager.commit();
        for _ in 0..5 {
            h.turn();
        }
        assert!(h.shell.conn.protocol_error().is_none());
        assert_eq!(h.state.workspaces().active(), "1");
    }

    #[test]
    fn each_window_belongs_to_a_workspace() {
        let mut h = Harness::start();
        h.bind_manager();
        h.open_window("Window");
        h.run_until("the window listed", |h| {
            !h.shell.client.toplevels.is_empty()
        });
        let id = h.shell.client.toplevels[0].1.id.unwrap();
        assert_eq!(h.shell.client.window(id).workspace, "1");
        let window = h.only_window();
        assert_eq!(h.state.workspace_of(&window), "1");

        // Moving it to a workspace that does not exist changes nothing; to its own neither.
        h.shell.client.handle(id).move_to_workspace("9".into());
        h.shell.client.handle(id).move_to_workspace("1".into());
        for _ in 0..5 {
            h.turn();
        }
        assert_eq!(h.state.workspace_of(&window), "1");
        assert_eq!(h.shell.client.window(id).workspace, "1");
        assert!(h.shell.conn.protocol_error().is_none());
    }

    #[test]
    fn the_keys_switch_workspace_and_the_keyboard_follows() {
        let mut h = Harness::start();
        h.open_window("One");
        let one = h.only_window();
        assert_eq!(h.state.workspace_of(&one), "1");

        // Super + 2: the window hides, nothing has the keyboard. The slide starts from the first
        // workspace, and ends on the second.
        h.keys(&[KEY_SUPER], KEY_2);
        assert_eq!(h.state.workspaces().active(), "2");
        assert!(!h.state.is_visible(&one));
        assert!(h.state.focused_window().is_none());
        assert_eq!(h.state.workspace_shift(&one, Duration::ZERO), Some(0.0));
        let later = Duration::from_secs(3600);
        assert_eq!(h.state.workspace_shift(&one, later), None);
        assert!(!h.state.workspaces_sliding(later));
        // The pointer no longer reaches it.
        h.state.pointer_motion((400.0, 300.0).into(), 10);
        let pointer = h.state.seat.get_pointer().unwrap();
        assert_eq!(pointer.current_focus(), None);

        // A window opened here belongs to the second workspace, and stands above the first one.
        h.open_window("Two");
        let two = h.titled("Two");
        assert_eq!(h.state.workspace_of(&two), "2");
        assert_eq!(h.state.workspace_shift(&two, later), Some(0.0));

        // Super + Ctrl + Left: back to the first one, its window takes the keyboard and the
        // pointer back.
        h.keys(&[KEY_SUPER, KEY_CTRL], KEY_LEFT);
        assert_eq!(h.state.workspaces().active(), "1");
        assert!(h.state.is_visible(&one) && !h.state.is_visible(&two));
        assert_eq!(h.focused_title().as_deref(), Some("One"));
        let one_surface = one.wl_surface().map(|s| s.into_owned());
        assert_eq!(pointer.current_focus(), one_surface);

        // No workspace left of the first one; Super + Ctrl + Right goes to the second.
        h.keys(&[KEY_SUPER, KEY_CTRL], KEY_LEFT);
        assert_eq!(h.state.workspaces().active(), "1");
        h.keys(&[KEY_SUPER, KEY_CTRL], KEY_RIGHT);
        assert_eq!(h.state.workspaces().active(), "2");
        assert_eq!(h.focused_title().as_deref(), Some("Two"));

        // Super + Shift + 3 sends the active window to the third workspace: hidden, without
        // following it.
        h.keys(&[KEY_SUPER, KEY_SHIFT], KEY_3);
        assert_eq!(h.state.workspace_of(&two), "3");
        assert_eq!(h.state.workspaces().active(), "2");
        assert!(!h.state.is_visible(&two));
        assert!(h.state.focused_window().is_none());
        h.keys(&[KEY_SUPER], KEY_3);
        assert_eq!(h.focused_title().as_deref(), Some("Two"));
        h.keys(&[KEY_SUPER], KEY_1);
        assert_eq!(h.focused_title().as_deref(), Some("One"));
    }

    #[test]
    fn the_shell_switches_workspace_and_moves_windows() {
        let mut h = Harness::start();
        h.bind_manager();
        let manager: ExtWorkspaceManagerV1 = h.shell.bind();
        h.run_until("the workspaces", |h| h.shell.client.workspaces.len() == 3);
        h.open_window("One");
        h.open_window("Two");
        h.run_until("both windows", |h| h.shell.client.toplevels.len() == 2);
        let (one, two) = (h.titled("One"), h.titled("Two"));
        let two_id = h.shell.client.toplevels[1].1.id.unwrap();

        // The shell moves the active window to workspace 2: the one behind takes the keyboard.
        h.shell.client.handle(two_id).move_to_workspace("2".into());
        h.run_until("the window on workspace 2", |h| {
            h.shell.client.window(two_id).workspace == "2"
        });
        assert!(!h.state.is_visible(&two));
        assert_eq!(h.focused_title().as_deref(), Some("One"));

        // `ext-workspace-v1`: activate and commit go to workspace 2.
        let handle = |h: &Harness, id: &str| {
            let workspaces = &h.shell.client.workspaces;
            workspaces.iter().find(|w| w.1.id == id).unwrap().0.clone()
        };
        handle(&h, "2").activate();
        manager.commit();
        h.run_until("workspace 2 active", |h| {
            h.shell
                .client
                .workspaces
                .iter()
                .any(|w| w.1.id == "2" && w.1.active)
        });
        assert!(h.state.is_visible(&two) && !h.state.is_visible(&one));
        assert_eq!(h.focused_title().as_deref(), Some("Two"));

        // Activating a window of another workspace goes there.
        let one_id = h.shell.client.toplevels[0].1.id.unwrap();
        h.shell.client.handle(one_id).activate();
        h.run_until("back on workspace 1", |h| {
            h.state.workspaces().active() == "1"
        });
        assert_eq!(h.focused_title().as_deref(), Some("One"));

        // Fewer workspaces while on the third: its windows and the active state go to the
        // first one. More: the list grows, nothing moves.
        h.state.move_to_workspace(&two, "3");
        h.state.activate_workspace("3");
        h.state.set_workspace_count(2);
        h.run_until("two workspaces", |h| h.shell.client.workspaces.len() == 2);
        assert_eq!(h.state.workspaces().active(), "1");
        assert_eq!(h.state.workspace_of(&two), "1");
        assert!(h.state.is_visible(&one) && h.state.is_visible(&two));
        h.state.set_workspace_count(5);
        h.run_until("five workspaces", |h| h.shell.client.workspaces.len() == 5);
        assert_eq!(h.state.workspaces().active(), "1");
        assert!(h.shell.conn.protocol_error().is_none());
    }

    #[test]
    fn a_minimized_window_follows_the_work_area() {
        let mut h = Harness::start();
        h.bind_manager();
        let (toplevel, surface) = h.open_window("Window");
        toplevel.set_maximized();
        surface.commit();
        h.run_until("the window listed", |h| {
            h.shell
                .client
                .toplevels
                .first()
                .is_some_and(|t| t.1.id.is_some())
        });
        let id = h.shell.client.toplevels[0].1.id.unwrap();
        h.run_until("maximized", |h| h.shell.client.has(id, Flag::Maximized));
        h.shell.client.handle(id).minimize();
        h.next_done("minimized");

        // A dock with an exclusive zone of 76 appears: the minimized window gets the work area.
        let qh = h.shell.queue.handle();
        let compositor: WlCompositor = h.shell.bind();
        let layer_shell: ZwlrLayerShellV1 = h.shell.bind();
        let dock_surface = compositor.create_surface(&qh, ());
        let dock =
            layer_shell.get_layer_surface(&dock_surface, None, Layer::Top, "dock".into(), &qh, ());
        dock.set_anchor(Anchor::Bottom | Anchor::Left | Anchor::Right);
        dock.set_size(0, 76);
        dock.set_exclusive_zone(76);
        dock_surface.commit();
        h.run_until("the dock configure", |h| h.shell.client.layer_configured);
        h.shell.draw(&dock_surface, 800, 76);
        let window = h.only_window();
        let pending =
            |w: &smithay::desktop::Window| w.toplevel().unwrap().with_pending_state(|s| s.size);
        h.run_until("the work area", |_| {
            pending(&window) == Some((800, 524).into())
        });
        assert!(h.state.is_minimized(&window));
    }
}
