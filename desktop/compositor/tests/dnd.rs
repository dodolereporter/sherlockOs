//! Drag and drop from a Wayland window to an X11 window (lot D-29c), without a screen: the
//! compositor launches XWayland, a Wayland client (wayland-client, on a socket pair) shows a
//! window and starts a drag from it, an X11 client (x11rb, in its thread) shows a window that
//! speaks XDND. The pointer goes through `State::pointer_motion` and `State::pointer_button`,
//! as in the real session, on a screen at scale 2.
//!
//! It needs `Xwayland` on the machine, as launching the compositor does (`docs/compositor.md`).

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs::{File, OpenOptions},
        io::Write as _,
        os::{fd::AsFd, unix::net::UnixStream},
        rc::Rc,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use compositor::{State, lock::Programs};
    use smithay::{
        backend::input::{ButtonState, KeyState},
        desktop::Window,
        input::keyboard::Keycode,
        reexports::calloop::EventLoop,
        utils::{Logical, Point},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_compositor::WlCompositor,
            wl_data_device::WlDataDevice,
            wl_data_device_manager::{DndAction, WlDataDeviceManager},
            wl_data_source::{self, WlDataSource},
            wl_pointer::{self, WlPointer},
            wl_registry,
            wl_seat::WlSeat,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    };
    use x11rb::{
        CURRENT_TIME, NONE,
        connection::Connection as _,
        protocol::{
            Event,
            xproto::{
                Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask,
                PropMode, Window as XWindow, WindowClass,
            },
        },
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    const URI_LIST: &str = "text/uri-list";
    const FILES: &[u8] = b"file:///tmp/photo.png\r\n";
    const STEP_BUDGET: Duration = Duration::from_secs(20);
    const XWAYLAND_STARTUP_BUDGET: Duration = Duration::from_secs(120);
    const BTN_LEFT: u32 = 0x110;
    /// xkb code of Escape.
    const ESCAPE: u32 = 9;

    #[derive(Default)]
    struct Seen {
        globals: Vec<(u32, String, u32)>,
        configured: bool,
        /// Serial of the last button press on the window.
        pressed: Option<u32>,
        /// What the sources were told, in order.
        source_events: Vec<&'static str>,
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

    impl Dispatch<WlPointer, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &WlPointer,
            event: wl_pointer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_pointer::Event::Button {
                serial,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } = event
            {
                seen.pressed = Some(serial);
            }
        }
    }

    impl Dispatch<WlDataSource, ()> for Seen {
        fn event(
            seen: &mut Self,
            source: &WlDataSource,
            event: wl_data_source::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                wl_data_source::Event::Send { mime_type, fd } => {
                    if mime_type == URI_LIST {
                        thread::spawn(move || {
                            let _ = File::from(fd).write_all(FILES);
                        });
                    }
                }
                wl_data_source::Event::DndDropPerformed => {
                    seen.source_events.push("drop performed")
                }
                wl_data_source::Event::DndFinished => {
                    seen.source_events.push("finished");
                    source.destroy();
                }
                wl_data_source::Event::Cancelled => {
                    seen.source_events.push("cancelled");
                    source.destroy();
                }
                _ => {}
            }
        }
    }

    delegate_noop!(Seen: WlCompositor);
    delegate_noop!(Seen: ignore WlSurface);
    delegate_noop!(Seen: ignore WlSeat);
    delegate_noop!(Seen: ignore WlShm);
    delegate_noop!(Seen: WlShmPool);
    delegate_noop!(Seen: ignore WlBuffer);
    delegate_noop!(Seen: ignore XdgToplevel);
    delegate_noop!(Seen: WlDataDeviceManager);
    delegate_noop!(Seen: ignore WlDataDevice);

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Seen>,
        seen: Seen,
        registry: wl_registry::WlRegistry,
        surface: Option<WlSurface>,
        device: Option<WlDataDevice>,
        _file: Option<File>,
    }

    impl Harness {
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

        fn pump(&mut self) {
            let _ = self.conn.flush();
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .unwrap();
            self.state.refresh();
            let _ = self.conn.flush();
            if let Some(guard) = self.queue.prepare_read() {
                let _ = guard.read();
            }
            self.queue.dispatch_pending(&mut self.seen).unwrap();
        }

        fn wait<T>(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> Option<T>) -> T {
            self.wait_within(STEP_BUDGET, what, &mut done)
        }

        fn wait_within<T>(
            &mut self,
            budget: Duration,
            what: &str,
            done: &mut impl FnMut(&mut Self) -> Option<T>,
        ) -> T {
            let deadline = Instant::now() + budget;
            loop {
                if let Some(value) = done(self) {
                    return value;
                }
                assert!(Instant::now() < deadline, "timed out waiting for: {what}");
                self.pump();
            }
        }

        fn idle(&mut self, time: Duration) {
            let end = Instant::now() + time;
            while Instant::now() < end {
                self.pump();
            }
        }

        fn join<T>(&mut self, what: &str, reply: &mpsc::Receiver<T>) -> T {
            self.wait(what, |_| reply.try_recv().ok())
        }

        /// The Wayland client shows a 400 × 300 window, with a pointer and a data device.
        fn show_window(&mut self) {
            let qh = self.qh();
            let seat: WlSeat = self.bind();
            seat.get_pointer(&qh, ());
            let surface = self.bind::<WlCompositor>().create_surface(&qh, ());
            let wm_base: XdgWmBase = self.bind();
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            toplevel.set_title("wayland".into());
            surface.commit();
            self.wait("the window's configure", |h| {
                h.seen.configured.then_some(())
            });
            let (w, h) = (400, 300);
            let shm: WlShm = self.bind();
            let file = shm_file((w * h * 4) as u64);
            let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self._file = Some(file);
            let device = self
                .bind::<WlDataDeviceManager>()
                .get_data_device(&seat, &qh, ());
            self.device = Some(device);
            self.surface = Some(surface);
            self.wait("the Wayland window", |h| wayland_window(&h.state));
        }

        /// Presses the left button at `at`, on the Wayland window, and starts a drag of a file
        /// from there.
        fn start_drag(&mut self, at: Point<f64, Logical>) {
            self.seen.pressed = None;
            self.state.pointer_motion(at, 1);
            self.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
            let serial = self.wait("the press on the window", |h| h.seen.pressed);
            let manager: WlDataDeviceManager = self.bind();
            let source = manager.create_data_source(&self.qh(), ());
            source.offer(URI_LIST.into());
            source.offer("x-special/gnome-copied-files".into());
            source.set_actions(DndAction::Copy | DndAction::Move);
            let (device, surface) = (self.device.clone().unwrap(), self.surface.clone());
            device.start_drag(Some(&source), surface.as_ref().unwrap(), None, serial);
            self.wait("the drag", |h| h.state.dragging().then_some(()));
        }
    }

    fn shm_file(size: u64) -> File {
        let path = std::env::temp_dir().join(format!("dnd-test-{}", std::process::id()));
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

    fn wayland_window(state: &State) -> Option<Window> {
        let mut windows = state.space.elements();
        windows.find(|w| w.toplevel().is_some()).cloned()
    }

    fn x11_window(state: &State) -> Option<Window> {
        let mut windows = state.space.elements();
        windows.find(|w| w.x11_surface().is_some()).cloned()
    }

    fn atom(conn: &RustConnection, name: &str) -> Atom {
        conn.intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    }

    /// What the X11 window saw.
    #[derive(Debug)]
    enum Heard {
        Enter {
            types: Vec<String>,
        },
        /// A position, and where it is in the window.
        Position {
            inside: (i16, i16),
        },
        Leave,
        /// The data read after the drop.
        Dropped(Option<Vec<u8>>),
    }

    /// An X11 window of 200 × 100 that speaks XDND 5: it accepts copies, reads `text/uri-list`
    /// after the drop and says it is done.
    fn x11_target(display: u32, heard: mpsc::Sender<Heard>) -> mpsc::Sender<()> {
        let (stop, stopped) = mpsc::channel::<()>();
        thread::spawn(move || {
            let (conn, screen) = RustConnection::connect(Some(&format!(":{display}"))).unwrap();
            let root = conn.setup().roots[screen].root;
            let window = conn.generate_id().unwrap();
            let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
            conn.create_window(
                0,
                window,
                root,
                0,
                0,
                200,
                100,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &aux,
            )
            .unwrap();
            let name = |n: &str| atom(&conn, n);
            let (aware, enter, position) =
                (name("XdndAware"), name("XdndEnter"), name("XdndPosition"));
            let (status, leave, drop) = (name("XdndStatus"), name("XdndLeave"), name("XdndDrop"));
            let (finished, selection) = (name("XdndFinished"), name("XdndSelection"));
            let (copy, uri_list, got) = (name("XdndActionCopy"), name(URI_LIST), name("GOT"));
            conn.change_property32(PropMode::REPLACE, window, aware, AtomEnum::ATOM, &[5])
                .unwrap();
            conn.map_window(window).unwrap();
            conn.flush().unwrap();
            let send = |to: XWindow, kind: Atom, data: [u32; 5]| {
                let message = ClientMessageEvent::new(32, to, kind, data);
                conn.send_event(false, to, EventMask::NO_EVENT, message)
                    .unwrap();
                conn.flush().unwrap();
            };
            let mut source = NONE;
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(Duration::from_millis(2))
            {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    match event {
                        Event::ClientMessage(m) if m.type_ == enter => {
                            let data = m.data.as_data32();
                            source = data[0];
                            let types = data[2..]
                                .iter()
                                .filter(|a| **a != NONE)
                                .map(|a| {
                                    let name = conn.get_atom_name(*a).unwrap().reply().unwrap();
                                    String::from_utf8(name.name).unwrap()
                                })
                                .collect();
                            let _ = heard.send(Heard::Enter { types });
                        }
                        Event::ClientMessage(m) if m.type_ == position => {
                            let data = m.data.as_data32();
                            let (x, y) = ((data[2] >> 16) as i16, (data[2] & 0xffff) as i16);
                            let inside = conn
                                .translate_coordinates(root, window, x, y)
                                .unwrap()
                                .reply()
                                .unwrap();
                            let inside = (inside.dst_x, inside.dst_y);
                            let _ = heard.send(Heard::Position { inside });
                            send(source, status, [window, 1, 0, 0, copy]);
                        }
                        Event::ClientMessage(m) if m.type_ == leave => {
                            let _ = heard.send(Heard::Leave);
                        }
                        Event::ClientMessage(m) if m.type_ == drop => {
                            conn.convert_selection(window, selection, uri_list, got, CURRENT_TIME)
                                .unwrap();
                            conn.flush().unwrap();
                        }
                        Event::SelectionNotify(n) => {
                            let data = (n.property != NONE).then(|| {
                                conn.get_property(true, window, got, AtomEnum::ANY, 0, 1 << 20)
                                    .unwrap()
                                    .reply()
                                    .unwrap()
                                    .value
                            });
                            let accepted = u32::from(data.is_some());
                            let _ = heard.send(Heard::Dropped(data));
                            send(source, finished, [window, accepted, copy, 0, 0]);
                        }
                        _ => {}
                    }
                }
            }
        });
        stop
    }

    /// Another X11 client tries to read the drag: `None` if refused.
    fn x11_snoop(display: u32) -> Option<Vec<u8>> {
        let (conn, screen) = RustConnection::connect(Some(&format!(":{display}"))).unwrap();
        let root = conn.setup().roots[screen].root;
        let window = conn.generate_id().unwrap();
        conn.create_window(
            0,
            window,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        )
        .unwrap();
        let (selection, target, got) = (
            atom(&conn, "XdndSelection"),
            atom(&conn, URI_LIST),
            atom(&conn, "SNOOP"),
        );
        conn.convert_selection(window, selection, target, got, CURRENT_TIME)
            .unwrap();
        conn.flush().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(Event::SelectionNotify(n)) = conn.poll_for_event().unwrap() {
                if n.property == NONE {
                    return None;
                }
                let reply = conn.get_property(true, window, got, AtomEnum::ANY, 0, 1 << 20);
                return Some(reply.unwrap().reply().unwrap().value);
            }
            thread::sleep(Duration::from_millis(2));
        }
        None
    }

    #[test]
    fn a_file_dragged_from_wayland_drops_on_x11_and_escape_cancels() {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        // 800 × 600 logical: positions in X11 are logical too, at every scale.
        state.add_output((1600, 1200).into(), 2.0);
        state.set_lock_programs(Programs {
            client: Vec::new(),
            rescue: None,
            swaylock: None,
        });
        let started = Rc::new(Cell::new(None));
        let done = started.clone();
        state
            .start_xwayland(&event_loop, move |_, display| done.set(Some(display)))
            .unwrap();
        let (server_side, client_side) = UnixStream::pair().unwrap();
        state.insert_client(server_side);
        let conn = Connection::from_socket(client_side).unwrap();
        let queue = conn.new_event_queue();
        let registry = conn.display().get_registry(&queue.handle(), ());
        let mut h = Harness {
            event_loop,
            state,
            conn,
            queue,
            seen: Seen::default(),
            registry,
            surface: None,
            device: None,
            _file: None,
        };
        let mut startup = |_: &mut Harness| started.get();
        h.wait_within(XWAYLAND_STARTUP_BUDGET, "XWayland startup", &mut startup);
        let display = started.get().flatten().expect("XWayland did not start");
        h.wait("the globals", |h| {
            (!h.seen.globals.is_empty()).then_some(())
        });

        // The Wayland window, then the X11 window in front of its middle.
        h.show_window();
        let wayland = wayland_window(&h.state).unwrap();
        let (heard, hearing) = mpsc::channel();
        let _target = x11_target(display, heard);
        let x11 = h.wait("the X11 window", |h| {
            x11_window(&h.state).filter(|w| h.state.focused_window().as_ref() == Some(w))
        });
        let w_rect = h.state.space.element_geometry(&wayland).unwrap();
        let x_rect = h.state.space.element_geometry(&x11).unwrap();
        assert!(w_rect.contains_rect(x_rect), "{w_rect:?} {x_rect:?}");
        let beside = (w_rect.loc + (10, 10).into()).to_f64();
        let over = (x_rect.loc + (100, 50).into()).to_f64();

        // Dragged from the Wayland window onto the X11 one: it is offered the types, told the
        // position inside it, and reads the files once dropped.
        h.start_drag(beside);
        h.state.pointer_motion(over, 3);
        let heard_enter = h.join("XdndEnter", &hearing);
        let Heard::Enter { types } = heard_enter else {
            panic!("{heard_enter:?}");
        };
        assert_eq!(types, [URI_LIST, "x-special/gnome-copied-files"]);
        let heard_position = h.join("XdndPosition", &hearing);
        assert!(
            matches!(heard_position, Heard::Position { inside: (100, 50) }),
            "{heard_position:?}"
        );
        h.idle(Duration::from_millis(50));
        // Only the window under the pointer may read the drag.
        let (snooped, snoop) = mpsc::channel();
        thread::spawn(move || {
            let _ = snooped.send(x11_snoop(display));
        });
        assert_eq!(h.join("the refused read", &snoop), None);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 4);
        let heard_drop = h.join("the drop", &hearing);
        assert!(
            matches!(&heard_drop, Heard::Dropped(Some(data)) if data == FILES),
            "{heard_drop:?}"
        );
        h.wait("the end of the drag", |h| {
            (!h.state.dragging()).then_some(())
        });
        h.idle(Duration::from_millis(50));
        assert_eq!(h.seen.source_events, ["drop performed", "finished"]);

        // Escape over the X11 window: it is left, the source is cancelled.
        h.seen.source_events.clear();
        h.start_drag(beside);
        h.state.pointer_motion(over, 5);
        h.join("XdndEnter", &hearing);
        h.join("XdndPosition", &hearing);
        h.state
            .keyboard_key(Keycode::new(ESCAPE), KeyState::Pressed, 6);
        h.state
            .keyboard_key(Keycode::new(ESCAPE), KeyState::Released, 7);
        let heard_leave = h.join("XdndLeave", &hearing);
        assert!(matches!(heard_leave, Heard::Leave), "{heard_leave:?}");
        h.wait("the cancel", |h| (!h.state.dragging()).then_some(()));
        h.idle(Duration::from_millis(50));
        assert_eq!(h.seen.source_events, ["cancelled"]);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 8);

        h.state.shutdown();
    }
}
