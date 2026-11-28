//! Screen capture for screen sharing (lots D-28b and D-28c, ADR 0028): a capture client on the
//! shell socket speaks `ext-image-copy-capture-v1` with `ext-output-image-capture-source-v1` and
//! `ext-foreign-toplevel-image-capture-source-v1`, an application on the ordinary socket shows a
//! window. Without a screen: the backend's drawing
//! is a `Capturer` given by the test, a pattern, or Mesa's software renderer for the test that
//! compares with the real rendering.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        ffi::OsString,
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::fs::FileExt as _, unix::net::UnixStream},
        rc::Rc,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compositor::{
        State,
        capture::{self, Image},
        lock::Programs,
        screencast::Access,
    };
    use smithay::reexports::calloop::EventLoop;
    use wayland_client::event_created_child;
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_compositor::WlCompositor,
            wl_output::WlOutput,
            wl_registry,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    };
    use wayland_protocols::ext::image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    };
    use wayland_protocols::ext::image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    };

    const MANAGER: &str = "ext_image_copy_capture_manager_v1";

    /// What a connection received.
    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        configured: bool,
        /// The constraints of the session: size, shared memory formats, `done`, `stopped`.
        size: Option<(u32, u32)>,
        formats: Vec<u32>,
        done: bool,
        stopped: bool,
        ready: usize,
        failed: Vec<u32>,
        /// The windows listed, with their title, until closed.
        toplevels: Vec<(ExtForeignToplevelHandleV1, String)>,
        closed: usize,
    }

    impl Dispatch<ExtForeignToplevelListV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ExtForeignToplevelListV1,
            event: ext_foreign_toplevel_list_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
                client.toplevels.push((toplevel, String::new()));
            }
        }

        event_created_child!(Client, ExtForeignToplevelListV1, [
            ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
        ]);
    }

    impl Dispatch<ExtForeignToplevelHandleV1, ()> for Client {
        fn event(
            client: &mut Self,
            handle: &ExtForeignToplevelHandleV1,
            event: ext_foreign_toplevel_handle_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use ext_foreign_toplevel_handle_v1::Event;
            match event {
                Event::Title { title } => {
                    if let Some(t) = client.toplevels.iter_mut().find(|t| &t.0 == handle) {
                        t.1 = title;
                    }
                }
                Event::Closed => {
                    client.toplevels.retain(|t| &t.0 != handle);
                    client.closed += 1;
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

    impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ExtImageCopyCaptureSessionV1,
            event: ext_image_copy_capture_session_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use ext_image_copy_capture_session_v1::Event;
            match event {
                Event::BufferSize { width, height } => client.size = Some((width, height)),
                Event::ShmFormat {
                    format: WEnum::Value(format),
                } => client.formats.push(format as u32),
                Event::Done => client.done = true,
                Event::Stopped => client.stopped = true,
                _ => {}
            }
        }
    }

    impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ExtImageCopyCaptureFrameV1,
            event: ext_image_copy_capture_frame_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use ext_image_copy_capture_frame_v1::Event;
            match event {
                Event::Ready => client.ready += 1,
                Event::Failed {
                    reason: WEnum::Value(reason),
                } => client.failed.push(reason as u32),
                _ => {}
            }
        }
    }

    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: ignore WlOutput);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);
    delegate_noop!(Client: ignore XdgToplevel);
    delegate_noop!(Client: ExtImageCopyCaptureManagerV1);
    delegate_noop!(Client: ExtOutputImageCaptureSourceManagerV1);
    delegate_noop!(Client: ExtImageCaptureSourceV1);
    delegate_noop!(Client: ExtForeignToplevelImageCaptureSourceManagerV1);

    /// A connection, its queue and what it received.
    struct Side {
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        registry: wl_registry::WlRegistry,
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

        fn read(&mut self) {
            let _ = self.conn.flush();
            if let Some(guard) = self.queue.prepare_read() {
                // Nothing to read is not an error: try again on the next turn.
                let _ = guard.read();
            }
            let _ = self.queue.dispatch_pending(&mut self.client);
        }

        /// A shared memory buffer of `w` × `h` in `format`, `stride` bytes a row, starting
        /// `offset` bytes into its pool, filled with `pixel`; and its file.
        fn buffer(
            &self,
            (w, h, stride, offset): (i32, i32, i32, i32),
            format: Format,
            pixel: [u8; 4],
        ) -> (WlBuffer, File) {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "compositor-capture-test-{}-{n}",
                std::process::id()
            ));
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            std::fs::remove_file(&path).unwrap();
            let size = offset + stride * h;
            file.write_all_at(&pixel.repeat(size as usize / 4), 0)
                .unwrap();
            let qh = self.queue.handle();
            let shm: WlShm = self.bind();
            let pool = shm.create_pool(file.as_fd(), size, &qh, ());
            let buffer = pool.create_buffer(offset, w, h, stride, format, &qh, ());
            (buffer, file)
        }
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        app: Side,
        shell: Side,
        /// How many times the screen was drawn for a capture.
        draws: Rc<Cell<usize>>,
        /// Whether the last drawing had the cursor.
        cursor: Rc<Cell<bool>>,
    }

    /// The pattern the test's capturer draws: red and green from the place, blue 7.
    fn pattern(x: u32, y: u32) -> [u8; 4] {
        [(x % 256) as u8, (y % 256) as u8, 7, 255]
    }

    /// The same pixel in a buffer of `xrgb8888`: blue first in memory.
    fn in_buffer(x: u32, y: u32) -> [u8; 4] {
        let [r, g, b, a] = pattern(x, y);
        [b, g, r, a]
    }

    impl Harness {
        /// One screen of 800 × 600, an application client, a shell client that may capture,
        /// and a capturer that draws `pattern`.
        fn start() -> Self {
            Self::with_access(Access::ShellClients)
        }

        fn with_access(access: Access) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            // A lock program that leaves at once: the compositor's own lock screen, still
            // locked, and nothing started on the machine.
            state.set_lock_programs(Programs {
                client: vec![OsString::from("true")],
                rescue: None,
                swaylock: None,
            });
            state.set_capture_access(access);
            let (server, app) = UnixStream::pair().unwrap();
            state.insert_client(server);
            let (server, shell) = UnixStream::pair().unwrap();
            state.insert_shell_client(server);
            let draws = Rc::new(Cell::new(0));
            let counted = draws.clone();
            let cursor = Rc::new(Cell::new(false));
            let with_cursor = cursor.clone();
            state.set_capturer(Box::new(move |state, target, cursor| {
                counted.set(counted.get() + 1);
                with_cursor.set(cursor);
                let size = capture::target_size(state, target).unwrap();
                let (width, height) = (size.w as u32, size.h as u32);
                let rgba = (0..height)
                    .flat_map(|y| (0..width).flat_map(move |x| pattern(x, y)))
                    .collect();
                Some(Image {
                    width,
                    height,
                    rgba,
                })
            }));
            let mut h = Self {
                event_loop,
                state,
                app: Side::new(app),
                shell: Side::new(shell),
                draws,
                cursor,
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

        /// The shell client opens a capture session on the screen and waits for its
        /// constraints.
        fn session(&mut self) -> ExtImageCopyCaptureSessionV1 {
            let qh = self.shell.queue.handle();
            let sources: ExtOutputImageCaptureSourceManagerV1 = self.shell.bind();
            let output: WlOutput = self.shell.bind();
            let source = sources.create_source(&output, &qh, ());
            self.session_on(&source, Options::empty())
        }

        fn session_on(
            &mut self,
            source: &ExtImageCaptureSourceV1,
            options: Options,
        ) -> ExtImageCopyCaptureSessionV1 {
            let qh = self.shell.queue.handle();
            let manager: ExtImageCopyCaptureManagerV1 = self.shell.bind();
            let session = manager.create_session(source, options, &qh, ());
            self.run_until("the constraints", |h| h.shell.client.done);
            session
        }

        /// The application shows a window of `w` × `h`, all of one color; its surface and
        /// buffer file.
        fn window(&mut self, (w, h): (i32, i32)) -> (WlSurface, XdgToplevel, File) {
            let qh = self.app.queue.handle();
            let compositor: WlCompositor = self.app.bind();
            let wm_base: XdgWmBase = self.app.bind();
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title(String::from("Notes"));
            surface.commit();
            self.run_until("the configure", |h| h.app.client.configured);
            let file = self.attach(&surface, (w, h));
            self.run_until("the window", |h| h.state.space.elements().count() == 1);
            (surface, toplevel, file)
        }

        /// A new buffer of `w` × `h` on `surface`.
        fn attach(&mut self, surface: &WlSurface, (w, h): (i32, i32)) -> File {
            let layout = (w, h, w * 4, 0);
            let (buffer, file) = self.app.buffer(layout, Format::Argb8888, [0, 0, 255, 255]);
            surface.attach(Some(&buffer), 0, 0);
            surface.damage_buffer(0, 0, w, h);
            surface.commit();
            file
        }

        /// The shell client lists the windows and makes a source of the one titled `title`.
        fn window_source(&mut self, title: &str) -> ExtImageCaptureSourceV1 {
            let qh = self.shell.queue.handle();
            let _list: ExtForeignToplevelListV1 = self.shell.bind();
            self.run_until("the listed window", |h| {
                h.shell.client.toplevels.iter().any(|t| t.1 == title)
            });
            let toplevels = &self.shell.client.toplevels;
            let handle = &toplevels.iter().find(|t| t.1 == title).unwrap().0;
            let sources: ExtForeignToplevelImageCaptureSourceManagerV1 = self.shell.bind();
            sources.create_source(handle, &qh, ())
        }

        /// A frame into `buffer`, asked for.
        fn capture(
            &mut self,
            session: &ExtImageCopyCaptureSessionV1,
            buffer: &WlBuffer,
        ) -> ExtImageCopyCaptureFrameV1 {
            let frame = session.create_frame(&self.shell.queue.handle(), ());
            frame.attach_buffer(buffer);
            frame.damage_buffer(0, 0, 800, 600);
            frame.capture();
            frame
        }
    }

    /// The pixel at (`x`, `y`) of a buffer read from `file`, `stride` bytes a row from `offset`.
    fn pixel(file: &File, (stride, offset): (u64, u64), x: u64, y: u64) -> [u8; 4] {
        let mut pixel = [0; 4];
        file.read_exact_at(&mut pixel, offset + y * stride + x * 4)
            .unwrap();
        pixel
    }

    const GLOBALS: [&str; 4] = [
        MANAGER,
        "ext_output_image_capture_source_manager_v1",
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        "ext_foreign_toplevel_list_v1",
    ];

    /// A capture protocol shows every pixel of the session: only the shell socket sees it.
    #[test]
    fn only_the_shell_sees_the_capture_globals() {
        let h = Harness::start();
        for global in GLOBALS {
            assert!(h.shell.sees(global), "{global}");
            assert!(!h.app.sees(global), "{global}");
        }
    }

    /// In the real session only the portal's program may capture: another program on the shell
    /// socket (here the test itself) sees the shell's globals, not the capture's.
    #[test]
    fn a_shell_client_that_is_not_the_portal_cannot_capture() {
        let h = Harness::with_access(Access::Portal);
        assert!(h.shell.sees("sherlock_shell_manager_v1"));
        for global in GLOBALS {
            assert!(!h.shell.sees(global), "{global}");
        }
    }

    /// A window is listed with its title, captured at its size with its title bar, the cursor
    /// drawn when asked for; it grows, and the session gives its new size, failing the frame
    /// asked for; it closes, and the session stops.
    #[test]
    fn a_window_is_captured_and_followed() {
        let mut h = Harness::start();
        h.state.set_window_animations(false);
        let (surface, toplevel, _file) = h.window((200, 100));
        let source = h.window_source("Notes");
        let session = h.session_on(&source, Options::PaintCursors);
        let (w, h_) = h.shell.client.size.unwrap();
        // The window and its title bar, at scale 1.
        assert_eq!(w, 200);
        assert!(h_ >= 100, "{h_}");
        let layout = (w as i32, h_ as i32, w as i32 * 4, 0);
        let (buffer, _target) = h.shell.buffer(layout, Format::Xrgb8888, [0; 4]);
        let frame = h.capture(&session, &buffer);
        h.run_until("the frame", |h| h.shell.client.ready == 1);
        assert!(h.cursor.get(), "the cursor is drawn");
        frame.destroy();

        // Larger: new constraints, and the frame asked for fails.
        let output = h.state.space.outputs().next().unwrap().clone();
        h.state.screen_changed(&output);
        let frame = h.capture(&session, &buffer);
        let _file = h.attach(&surface, (300, 150));
        h.run_until("the new size", |h| {
            h.shell.client.size.is_some_and(|s| s.0 == 300)
        });
        h.run_until("the failure", |h| !h.shell.client.failed.is_empty());
        let reason = FailureReason::BufferConstraints as u32;
        assert_eq!(h.shell.client.failed, vec![reason]);
        frame.destroy();

        // Closed: the list says so, and the session stops.
        toplevel.destroy();
        surface.destroy();
        h.run_until("the stop", |h| h.shell.client.stopped);
        h.run_until("the closed window", |h| h.shell.client.closed == 1);
    }

    /// The session asks for a buffer of the screen's size, in the two formats every compositor
    /// has. A frame is the screen as drawn, copied row by row into a buffer with padded rows;
    /// the next one waits until something changes on the screen.
    #[test]
    fn a_frame_is_the_drawn_screen_and_the_next_waits_for_a_change() {
        let mut h = Harness::start();
        let session = h.session();
        assert_eq!(h.shell.client.size, Some((800, 600)));
        let formats = &h.shell.client.formats;
        assert!(formats.contains(&(Format::Xrgb8888 as u32)), "{formats:?}");
        assert!(!h.shell.client.stopped);
        // Rows of 800 pixels and 16 more, from 64 bytes into the pool.
        let (stride, offset) = (816 * 4, 64);
        let layout = (800, 600, stride, offset);
        let (buffer, file) = h.shell.buffer(layout, Format::Xrgb8888, [1, 2, 3, 4]);
        let frame = h.capture(&session, &buffer);
        h.run_until("the frame", |h| h.shell.client.ready == 1);
        assert_eq!(h.draws.get(), 1);
        let place = (stride as u64, offset as u64);
        for (x, y) in [(0, 0), (799, 0), (400, 300), (13, 599), (799, 599)] {
            assert_eq!(
                pixel(&file, place, x, y),
                in_buffer(x as u32, y as u32),
                "({x}, {y})"
            );
        }
        // The padding stays the client's.
        assert_eq!(pixel(&file, place, 805, 10), [1, 2, 3, 4]);
        frame.destroy();

        // The screen shows nothing new: the next frame waits.
        let frame = h.capture(&session, &buffer);
        for _ in 0..10 {
            h.turn();
        }
        assert_eq!((h.shell.client.ready, h.draws.get()), (1, 1));
        // A new picture on the screen, as the backend reports it: drawn again.
        let output = h.state.space.outputs().next().unwrap().clone();
        h.state.screen_changed(&output);
        h.run_until("the next frame", |h| h.shell.client.ready == 2);
        assert_eq!(h.draws.get(), 2);
        frame.destroy();
        assert!(h.shell.client.failed.is_empty());
    }

    /// While the session is locked the screen is never drawn for a capture: every frame is
    /// black. Unlocked, it is drawn again.
    #[test]
    fn nothing_is_captured_while_locked() {
        let mut h = Harness::start();
        let session = h.session();
        h.state.lock_session();
        assert!(h.state.is_locked());
        let layout = (800, 600, 800 * 4, 0);
        let (buffer, file) = h.shell.buffer(layout, Format::Xrgb8888, [9, 9, 9, 9]);
        let frame = h.capture(&session, &buffer);
        h.run_until("the frame", |h| h.shell.client.ready == 1);
        assert_eq!(h.draws.get(), 0, "never drawn");
        let place = (800 * 4, 0);
        for (x, y) in [(0, 0), (400, 300), (799, 599)] {
            assert_eq!(pixel(&file, place, x, y), [0, 0, 0, 255], "({x}, {y})");
        }
        frame.destroy();
        assert_eq!(h.state.capture_sessions(), 1, "the session goes on, black");
    }

    /// A buffer of another size fails the frame, which says why; the session goes on, and a
    /// buffer that fits then works.
    #[test]
    fn a_buffer_that_does_not_fit_fails_the_frame() {
        let mut h = Harness::start();
        let session = h.session();
        let (small, _file) = h
            .shell
            .buffer((400, 300, 1600, 0), Format::Xrgb8888, [0; 4]);
        let frame = h.capture(&session, &small);
        h.run_until("the failure", |h| !h.shell.client.failed.is_empty());
        let reason = FailureReason::BufferConstraints as u32;
        assert_eq!(h.shell.client.failed, vec![reason]);
        frame.destroy();
        assert!(!h.shell.client.stopped);
        let (fits, _file) = h
            .shell
            .buffer((800, 600, 3200, 0), Format::Argb8888, [0; 4]);
        let _frame = h.capture(&session, &fits);
        h.run_until("the frame", |h| h.shell.client.ready == 1);
        assert_eq!(h.shell.client.failed.len(), 1);
    }

    /// The real rendering, with Mesa's software renderer: a red window shown in the middle of
    /// the screen is red in the captured frame, and the desktop around it is not.
    #[test]
    #[ignore = "needs Mesa's EGL: run it with --ignored"]
    fn a_captured_screen_matches_the_rendering() {
        use compositor::{render::Scene, wallpaper};
        use smithay::backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::gles::GlesRenderer,
        };
        let mut h = Harness::start();
        let mut device = EGLDevice::enumerate().unwrap();
        let device = device
            .find(EGLDevice::is_software)
            .expect("no software EGL device");
        // SAFETY: the display lives as long as the renderer, moved into the capturer.
        let display = unsafe { EGLDisplay::new(device) }.unwrap();
        let context = EGLContext::new(&display).unwrap();
        // SAFETY: the context was just made and is used by this renderer alone.
        let mut renderer = unsafe { GlesRenderer::new(context) }.unwrap();
        let scene = Scene::new(&mut renderer).unwrap();
        let mut textures = wallpaper::Textures::default();
        h.state.set_capturer(Box::new(move |state, target, _| {
            let _ = &display;
            compositor::capture::image(&scene, &mut renderer, &mut textures, state, target, None)
        }));
        // The window appears at once: its opening would start at this first drawing.
        h.state.set_window_animations(false);
        // A window of 100 × 80, all red (Argb8888 is little-endian: blue, green, red, alpha).
        let qh = h.app.queue.handle();
        let compositor: WlCompositor = h.app.bind();
        let wm_base: XdgWmBase = h.app.bind();
        let surface = compositor.create_surface(&qh, ());
        let _toplevel = wm_base
            .get_xdg_surface(&surface, &qh, ())
            .get_toplevel(&qh, ());
        surface.commit();
        h.run_until("the configure", |h| h.app.client.configured);
        let (buffer, _red) = h
            .app
            .buffer((100, 80, 400, 0), Format::Argb8888, [0, 0, 255, 255]);
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();
        h.run_until("the window", |h| h.state.space.elements().count() == 1);
        for _ in 0..60 {
            h.turn();
        }
        let session = h.session();
        let layout = (800, 600, 3200, 0);
        let (target, file) = h.shell.buffer(layout, Format::Xrgb8888, [0; 4]);
        let _frame = h.capture(&session, &target);
        h.run_until("the frame", |h| h.shell.client.ready == 1);
        let window = h.state.space.elements().next().unwrap().clone();
        let at = h.state.space.element_geometry(&window).unwrap();
        let center = at.loc + at.size.downscale(2).to_point();
        let place = (3200, 0);
        let (x, y) = (center.x as u64, center.y as u64);
        // Blue first in memory: red is the third byte.
        assert_eq!(pixel(&file, place, x, y), [0, 0, 255, 255]);
        assert_ne!(pixel(&file, place, 20, 300), [0, 0, 255, 255]);
    }
}
