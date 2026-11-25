//! linux-dmabuf without a GPU: the global, its default feedback, and buffers the renderer
//! refuses. The renderer is replaced by an importer that accepts or refuses on demand; the CI has
//! no GPU, and a buffer refused by the real renderer takes the same path. Compositor and client
//! run in the same thread, as in `headless.rs`.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::HashSet,
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::fs::FileExt, unix::net::UnixStream},
        rc::Rc,
        time::Duration,
    };

    use compositor::State;
    use smithay::{
        backend::allocator::{Buffer as _, Format, Fourcc, Modifier},
        reexports::calloop::EventLoop,
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, QueueHandle, delegate_noop, event_created_child,
        protocol::{
            wl_buffer::WlBuffer,
            wl_callback::{self, WlCallback},
            wl_compositor::WlCompositor,
            wl_registry,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::wp::linux_dmabuf::zv1::client::{
        zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
        zwp_linux_dmabuf_feedback_v1::{self, ZwpLinuxDmabufFeedbackV1},
        zwp_linux_dmabuf_v1::{self, ZwpLinuxDmabufV1},
    };

    /// Device number given as the main device: 226:128, the first render node.
    const DEVICE: u64 = (226 << 8) | 128;
    const SIZE: (i32, i32) = (64, 48);

    fn announced() -> Vec<Format> {
        let format = |code, modifier| Format { code, modifier };
        vec![
            format(Fourcc::Argb8888, Modifier::Linear),
            format(Fourcc::Xrgb8888, Modifier::Linear),
            // A tiled layout, as GPU drivers allocate (here an AMD one, by its value).
            format(Fourcc::Argb8888, Modifier::from(0x0200_0000_1866_1b04)),
        ]
    }

    /// What the client has received.
    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        synced: bool,
        /// Version 3: `modifier` events.
        modifiers: Vec<(u32, u64)>,
        /// Default feedback: main device, table, tranches (target device, indices), done.
        main_device: Option<Vec<u8>>,
        table: Option<File>,
        tranches: Vec<(Vec<u8>, Vec<u16>)>,
        target: Option<Vec<u8>>,
        indices: Vec<u16>,
        feedback_done: bool,
        /// Answers to `create`.
        created: Option<WlBuffer>,
        failed: usize,
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

    impl Dispatch<ZwpLinuxDmabufV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwpLinuxDmabufV1,
            event: zwp_linux_dmabuf_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwp_linux_dmabuf_v1::Event::Modifier {
                format,
                modifier_hi,
                modifier_lo,
            } = event
            {
                let modifier = (u64::from(modifier_hi) << 32) | u64::from(modifier_lo);
                client.modifiers.push((format, modifier));
            }
        }
    }

    impl Dispatch<ZwpLinuxDmabufFeedbackV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwpLinuxDmabufFeedbackV1,
            event: zwp_linux_dmabuf_feedback_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            use zwp_linux_dmabuf_feedback_v1::Event;
            match event {
                Event::MainDevice { device } => client.main_device = Some(device),
                Event::FormatTable { fd, .. } => client.table = Some(File::from(fd)),
                Event::TrancheTargetDevice { device } => client.target = Some(device),
                Event::TrancheFormats { indices } => client.indices.extend(
                    indices
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| u16::from_ne_bytes(*b)),
                ),
                Event::TrancheDone => {
                    let target = client.target.take().unwrap_or_default();
                    let indices = std::mem::take(&mut client.indices);
                    client.tranches.push((target, indices));
                }
                Event::Done => client.feedback_done = true,
                _ => {}
            }
        }
    }

    impl Dispatch<ZwpLinuxBufferParamsV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwpLinuxBufferParamsV1,
            event: zwp_linux_buffer_params_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwp_linux_buffer_params_v1::Event::Created { buffer } => {
                    client.created = Some(buffer);
                }
                zwp_linux_buffer_params_v1::Event::Failed => client.failed += 1,
                _ => {}
            }
        }

        event_created_child!(Client, ZwpLinuxBufferParamsV1, [
            zwp_linux_buffer_params_v1::EVT_CREATED_OPCODE => (WlBuffer, ()),
        ]);
    }

    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlBuffer);

    /// Imports seen by the fake renderer, and whether it accepts them.
    #[derive(Default)]
    struct Renderer {
        accept: Cell<bool>,
        imports: RefCell<Vec<((i32, i32), Format)>>,
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        registry: wl_registry::WlRegistry,
    }

    impl Harness {
        /// One screen and one client. With a renderer, linux-dmabuf is offered.
        fn start(renderer: Option<Rc<Renderer>>) -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            if let Some(renderer) = renderer {
                let import = Box::new(
                    move |buffer: &smithay::backend::allocator::dmabuf::Dmabuf| {
                        let size = buffer.size();
                        let format = buffer.format();
                        renderer
                            .imports
                            .borrow_mut()
                            .push(((size.w, size.h), format));
                        if renderer.accept.get() {
                            Ok(())
                        } else {
                            Err("the renderer cannot draw it".into())
                        }
                    },
                );
                state.enable_dmabuf(DEVICE, announced(), import).unwrap();
            }
            let (server_side, client_side) = UnixStream::pair().unwrap();
            state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            let mut harness = Self {
                event_loop,
                state,
                conn,
                queue,
                client: Client::default(),
                registry,
            };
            harness.sync();
            harness
        }

        /// Runs compositor and client until `done` is true, or until the client is ended by a
        /// protocol error: `Err` with its code.
        fn run_until(&mut self, done: impl Fn(&Client) -> bool) -> Result<(), u32> {
            for _ in 0..200 {
                let ended = self.conn.flush().is_err();
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if let Some(guard) = self.queue.prepare_read() {
                    let _ = guard.read();
                }
                if ended || self.queue.dispatch_pending(&mut self.client).is_err() {
                    return Err(self.conn.protocol_error().map_or(u32::MAX, |e| e.code));
                }
                if done(&self.client) {
                    return Ok(());
                }
            }
            panic!("timed out");
        }

        /// A full round trip: everything sent before has been handled.
        fn sync(&mut self) {
            self.client.synced = false;
            self.conn.display().sync(&self.queue.handle(), ());
            self.run_until(|c| c.synced).unwrap();
        }

        fn global(&self, interface: &str) -> Option<(u32, u32)> {
            self.client
                .globals
                .iter()
                .find(|g| g.1 == interface)
                .map(|g| (g.0, g.2))
        }

        fn dmabuf(&self, version: u32) -> ZwpLinuxDmabufV1 {
            let (name, _) = self.global("zwp_linux_dmabuf_v1").unwrap();
            self.registry.bind(name, version, &self.queue.handle(), ())
        }

        /// Parameters of a one-plane buffer of `SIZE`, backed by a plain file: no GPU can import
        /// it, but its size and stride are those of a real buffer.
        fn params(
            &self,
            dmabuf: &ZwpLinuxDmabufV1,
            plane: &File,
            offset: u32,
            stride: u32,
            modifier: u64,
        ) -> ZwpLinuxBufferParamsV1 {
            let params = dmabuf.create_params(&self.queue.handle(), ());
            let (hi, lo) = ((modifier >> 32) as u32, modifier as u32);
            params.add(plane.as_fd(), 0, offset, stride, hi, lo);
            params
        }
    }

    /// A file of `len` bytes, removed from the disk right away.
    fn plane_file(len: u64) -> File {
        let path = std::env::temp_dir().join(format!(
            "compositor-dmabuf-{}-{len}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        file.set_len(len).unwrap();
        file
    }

    #[test]
    fn without_a_renderer_dmabuf_is_not_offered() {
        let harness = Harness::start(None);
        assert!(harness.global("wl_shm").is_some());
        assert!(harness.global("zwp_linux_dmabuf_v1").is_none());
    }

    #[test]
    fn the_global_and_its_default_feedback() {
        let mut harness = Harness::start(Some(Rc::default()));
        // Version 5: default feedback, and modifiers checked across planes.
        let (_, version) = harness.global("zwp_linux_dmabuf_v1").unwrap();
        assert_eq!(version, 5);

        let dmabuf = harness.dmabuf(5);
        let _feedback = dmabuf.get_default_feedback(&harness.queue.handle(), ());
        harness.run_until(|c| c.feedback_done).unwrap();
        let device = DEVICE.to_ne_bytes().to_vec();
        assert_eq!(harness.client.main_device.as_ref(), Some(&device));
        // One tranche, on the main device, with every announced format, read from the table.
        let table = harness.client.table.as_ref().unwrap();
        let mut bytes = vec![0; announced().len() * 16];
        table.read_exact_at(&mut bytes, 0).unwrap();
        let entry = |i: usize| {
            let e = &bytes[i * 16..i * 16 + 16];
            let code = u32::from_ne_bytes(e[0..4].try_into().unwrap());
            let modifier = u64::from_ne_bytes(e[8..16].try_into().unwrap());
            (code, modifier)
        };
        let [(target, indices)] = harness.client.tranches.as_slice() else {
            panic!("one tranche expected: {:?}", harness.client.tranches);
        };
        assert_eq!(target, &device);
        let got: HashSet<_> = indices.iter().map(|&i| entry(usize::from(i))).collect();
        let wanted: HashSet<_> = announced()
            .iter()
            .map(|f| (f.code as u32, u64::from(f.modifier)))
            .collect();
        assert_eq!(got, wanted);

        // A client of version 3 gets the same formats as `modifier` events.
        let _old = harness.dmabuf(3);
        harness.sync();
        let got: HashSet<_> = harness.client.modifiers.iter().copied().collect();
        assert_eq!(got, wanted);
    }

    #[test]
    fn a_buffer_the_renderer_refuses_fails_without_ending_the_client() {
        let renderer = Rc::new(Renderer::default());
        let mut harness = Harness::start(Some(renderer.clone()));
        let dmabuf = harness.dmabuf(5);
        let (w, h) = SIZE;
        let stride = w as u32 * 4;
        let plane = plane_file(u64::from(stride) * h as u64);
        let format = Fourcc::Argb8888 as u32;

        // Refused: `failed`, and the client goes on.
        let params = harness.params(&dmabuf, &plane, 0, stride, u64::from(Modifier::Linear));
        params.create(w, h, format, zwp_linux_buffer_params_v1::Flags::empty());
        harness.run_until(|c| c.failed == 1).unwrap();
        harness.sync();
        assert!(harness.client.created.is_none());
        let wanted = Format {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        };
        assert_eq!(renderer.imports.borrow().as_slice(), &[((w, h), wanted)]);

        // Accepted: a `wl_buffer`, which a surface can show.
        renderer.accept.set(true);
        let params = harness.params(&dmabuf, &plane, 0, stride, u64::from(Modifier::Linear));
        params.create(w, h, format, zwp_linux_buffer_params_v1::Flags::empty());
        harness.run_until(|c| c.created.is_some()).unwrap();
        let (name, _) = harness.global("wl_compositor").unwrap();
        let compositor: WlCompositor = harness.registry.bind(name, 1, &harness.queue.handle(), ());
        let surface = compositor.create_surface(&harness.queue.handle(), ());
        surface.attach(harness.client.created.as_ref(), 0, 0);
        surface.commit();
        harness.sync();
        assert_eq!(harness.client.failed, 1);
        assert_eq!(renderer.imports.borrow().len(), 2);
    }

    #[test]
    fn invalid_parameters_end_the_client_not_the_compositor() {
        let (w, h) = SIZE;
        let stride = w as u32 * 4;
        let linear = u64::from(Modifier::Linear);
        let plane = plane_file(u64::from(stride) * h as u64);
        // A format that was not announced, a height of zero with a stride that would overflow
        // the compositor's bound checks, a plane larger than its file.
        use zwp_linux_buffer_params_v1::Error;
        let cases = [
            (Fourcc::Nv12 as u32, h, 0, stride, Error::InvalidFormat),
            (
                Fourcc::Argb8888 as u32,
                0,
                4,
                u32::MAX,
                Error::InvalidDimensions,
            ),
            (
                Fourcc::Argb8888 as u32,
                h * 2,
                0,
                stride,
                Error::OutOfBounds,
            ),
        ];
        for (format, height, offset, stride, error) in cases {
            let renderer = Rc::new(Renderer::default());
            let mut harness = Harness::start(Some(renderer.clone()));
            let dmabuf = harness.dmabuf(5);
            let params = harness.params(&dmabuf, &plane, offset, stride, linear);
            params.create(
                w,
                height,
                format,
                zwp_linux_buffer_params_v1::Flags::empty(),
            );
            let ended = harness.run_until(|_| false).err();
            assert_eq!(ended, Some(error as u32), "{error:?}");
            assert!(renderer.imports.borrow().is_empty(), "{error:?}");
            // The compositor goes on: a new client gets its globals.
            let (server_side, client_side) = UnixStream::pair().unwrap();
            harness.state.insert_client(server_side);
            harness.conn = Connection::from_socket(client_side).unwrap();
            harness.queue = harness.conn.new_event_queue();
            harness.client = Client::default();
            harness.registry = harness
                .conn
                .display()
                .get_registry(&harness.queue.handle(), ());
            harness.sync();
            assert!(harness.global("zwp_linux_dmabuf_v1").is_some(), "{error:?}");
        }
    }
}
