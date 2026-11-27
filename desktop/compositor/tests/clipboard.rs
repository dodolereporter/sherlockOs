//! The clipboard and the primary selection between Wayland and X11 (lot D-29), without a
//! screen: the compositor launches XWayland, a Wayland client (wayland-client, on a socket
//! pair) shows a window, and X11 clients (x11rb, each in its thread) show a window, take or
//! read the selections. Text and images go both ways, a few megabytes too (INCR on the X11
//! side), and nothing passes while the session is locked.
//!
//! It needs `Xwayland` on the machine, as launching the compositor does (`docs/compositor.md`).

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        collections::HashMap,
        fs::{File, OpenOptions},
        io::{Read as _, Write as _},
        os::{fd::AsFd, unix::net::UnixStream},
        rc::Rc,
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use compositor::{State, lock::Programs};
    use shell_protocol::clipboard::client::sherlock_clipboard_history_v1::{
        self as history, SherlockClipboardHistoryV1,
    };
    use smithay::{
        desktop::Window,
        reexports::calloop::EventLoop,
        wayland::selection::{
            data_device::current_data_device_selection_userdata,
            primary_selection::current_primary_selection_userdata,
        },
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle,
        backend::ObjectId,
        delegate_noop, event_created_child,
        protocol::{
            wl_buffer::WlBuffer,
            wl_compositor::WlCompositor,
            wl_data_device::{self, WlDataDevice},
            wl_data_device_manager::WlDataDeviceManager,
            wl_data_offer::{self, WlDataOffer},
            wl_data_source::{self, WlDataSource},
            wl_registry,
            wl_seat::WlSeat,
            wl_shm::{Format, WlShm},
            wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::{
        wp::primary_selection::zv1::client::{
            zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1,
            zwp_primary_selection_device_v1::{self, ZwpPrimarySelectionDeviceV1},
            zwp_primary_selection_offer_v1::{self, ZwpPrimarySelectionOfferV1},
            zwp_primary_selection_source_v1::{self, ZwpPrimarySelectionSourceV1},
        },
        xdg::shell::client::{
            xdg_surface::{self, XdgSurface},
            xdg_toplevel::XdgToplevel,
            xdg_wm_base::{self, XdgWmBase},
        },
    };
    use x11rb::{
        CURRENT_TIME, NONE,
        connection::Connection as _,
        protocol::{
            Event,
            xproto::{
                Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux,
                EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
                SelectionRequestEvent, Window as XWindow, WindowClass,
            },
        },
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    const TEXT: &str = "text/plain;charset=utf-8";
    const PNG: &str = "image/png";
    // XWayland startup gets more time than a step, as in `tests/xwayland.rs`.
    const STEP_BUDGET: Duration = Duration::from_secs(20);
    const XWAYLAND_STARTUP_BUDGET: Duration = Duration::from_secs(120);
    /// Largest piece an X11 client below puts in a property at once: past it, it goes by INCR.
    const X11_CHUNK: usize = 64 * 1024;

    /// `len` bytes that do not repeat on any short period.
    fn bytes(len: usize, seed: usize) -> Arc<Vec<u8>> {
        Arc::new((0..len).map(|i| ((i * 7919 + seed) % 251) as u8).collect())
    }

    /// What the Wayland client has received.
    #[derive(Default)]
    struct Seen {
        globals: Vec<(u32, String, u32)>,
        configured: bool,
        /// Types of each offer.
        types: HashMap<ObjectId, Vec<String>>,
        clipboard: Option<WlDataOffer>,
        primary: Option<ZwpPrimarySelectionOfferV1>,
        /// Selections received, clipboard and primary.
        selections: usize,
    }

    /// What a source of the Wayland client gives, by type.
    type Content = HashMap<String, Arc<Vec<u8>>>;

    /// Writes `data` into `fd` in a thread: the reader may be the compositor's own loop.
    fn give(content: &Content, mime_type: &str, fd: std::os::fd::OwnedFd) {
        if let Some(data) = content.get(mime_type).cloned() {
            thread::spawn(move || {
                let _ = File::from(fd).write_all(&data);
            });
        }
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

    impl Dispatch<WlDataDevice, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &WlDataDevice,
            event: wl_data_device::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                wl_data_device::Event::DataOffer { id } => {
                    seen.types.insert(id.id(), Vec::new());
                }
                wl_data_device::Event::Selection { id } => {
                    seen.clipboard = id;
                    seen.selections += 1;
                }
                _ => {}
            }
        }

        event_created_child!(Seen, WlDataDevice, [
            wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
        ]);
    }

    impl Dispatch<WlDataOffer, ()> for Seen {
        fn event(
            seen: &mut Self,
            offer: &WlDataOffer,
            event: wl_data_offer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_data_offer::Event::Offer { mime_type } = event {
                seen.types.entry(offer.id()).or_default().push(mime_type);
            }
        }
    }

    impl Dispatch<WlDataSource, Content> for Seen {
        fn event(
            _: &mut Self,
            source: &WlDataSource,
            event: wl_data_source::Event,
            content: &Content,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                wl_data_source::Event::Send { mime_type, fd } => give(content, &mime_type, fd),
                wl_data_source::Event::Cancelled => source.destroy(),
                _ => {}
            }
        }
    }

    impl Dispatch<ZwpPrimarySelectionDeviceV1, ()> for Seen {
        fn event(
            seen: &mut Self,
            _: &ZwpPrimarySelectionDeviceV1,
            event: zwp_primary_selection_device_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                    seen.types.insert(offer.id(), Vec::new());
                }
                zwp_primary_selection_device_v1::Event::Selection { id } => {
                    seen.primary = id;
                    seen.selections += 1;
                }
                _ => {}
            }
        }

        event_created_child!(Seen, ZwpPrimarySelectionDeviceV1, [
            zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE
                => (ZwpPrimarySelectionOfferV1, ()),
        ]);
    }

    impl Dispatch<ZwpPrimarySelectionOfferV1, ()> for Seen {
        fn event(
            seen: &mut Self,
            offer: &ZwpPrimarySelectionOfferV1,
            event: zwp_primary_selection_offer_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event {
                seen.types.entry(offer.id()).or_default().push(mime_type);
            }
        }
    }

    impl Dispatch<ZwpPrimarySelectionSourceV1, Content> for Seen {
        fn event(
            _: &mut Self,
            source: &ZwpPrimarySelectionSourceV1,
            event: zwp_primary_selection_source_v1::Event,
            content: &Content,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                zwp_primary_selection_source_v1::Event::Send { mime_type, fd } => {
                    give(content, &mime_type, fd)
                }
                zwp_primary_selection_source_v1::Event::Cancelled => source.destroy(),
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
    delegate_noop!(Seen: ZwpPrimarySelectionDeviceManagerV1);

    fn shm_file(size: u64) -> File {
        let name = format!("clipboard-test-{}", std::process::id());
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

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Seen>,
        seen: Seen,
        registry: wl_registry::WlRegistry,
        devices: Option<(WlDataDevice, ZwpPrimarySelectionDeviceV1)>,
        /// Keeps the window's buffer alive.
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

        /// One turn of the compositor's loop and of the Wayland client.
        fn pump(&mut self) {
            let _ = self.conn.flush();
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .unwrap();
            self.state.refresh();
            let _ = self.conn.flush();
            if let Some(guard) = self.queue.prepare_read() {
                // Nothing to read is not an error.
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

        /// Runs the loop for `time`.
        fn idle(&mut self, time: Duration) {
            let end = Instant::now() + time;
            while Instant::now() < end {
                self.pump();
            }
        }

        /// Waits for what an X11 thread sends back.
        fn join<T>(&mut self, what: &str, reply: mpsc::Receiver<T>) -> T {
            self.wait(what, |_| reply.try_recv().ok())
        }

        /// The Wayland client shows a 100 × 80 window and takes its data devices.
        fn show_window(&mut self) {
            let qh = self.qh();
            let seat: WlSeat = self.bind();
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
            let (w, h) = (100, 80);
            let shm: WlShm = self.bind();
            let file = shm_file((w * h * 4) as u64);
            let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self._file = Some(file);
            let clipboard = self
                .bind::<WlDataDeviceManager>()
                .get_data_device(&seat, &qh, ());
            let primary = self
                .bind::<ZwpPrimarySelectionDeviceManagerV1>()
                .get_device(&seat, &qh, ());
            self.devices = Some((clipboard, primary));
            self.wait("the Wayland window", |h| wayland_window(&h.state));
        }

        /// The Wayland client sets the clipboard (or the primary selection) to `content`.
        fn wl_copy(&mut self, primary: bool, content: &[(&str, &Arc<Vec<u8>>)]) {
            let qh = self.qh();
            let content: Content = content
                .iter()
                .map(|(mime, data)| (mime.to_string(), Arc::clone(data)))
                .collect();
            let mimes: Vec<String> = content.keys().cloned().collect();
            let (clipboard, primary_device) = self.devices.clone().unwrap();
            if primary {
                let manager: ZwpPrimarySelectionDeviceManagerV1 = self.bind();
                let source = manager.create_source(&qh, content);
                mimes.into_iter().for_each(|m| source.offer(m));
                primary_device.set_selection(Some(&source), 0);
            } else {
                let manager: WlDataDeviceManager = self.bind();
                let source = manager.create_data_source(&qh, content);
                mimes.into_iter().for_each(|m| source.offer(m));
                clipboard.set_selection(Some(&source), 0);
            }
            self.idle(Duration::from_millis(50));
        }

        /// The Wayland client reads its last offer of the clipboard (or primary selection).
        fn wl_paste(&mut self, primary: bool, mime: &str) -> Vec<u8> {
            let (mut reader, writer) = std::io::pipe().unwrap();
            if primary {
                let offer = self.seen.primary.clone().unwrap();
                offer.receive(mime.into(), writer.as_fd());
            } else {
                let offer = self.seen.clipboard.clone().unwrap();
                offer.receive(mime.into(), writer.as_fd());
            }
            let _ = self.conn.flush();
            drop(writer);
            let (sender, reply) = mpsc::channel();
            thread::spawn(move || {
                let mut data = Vec::new();
                let _ = reader.read_to_end(&mut data);
                let _ = sender.send(data);
            });
            self.join("the Wayland read", reply)
        }

        /// Waits for a new offer of the clipboard (or primary selection) with `mime` among its
        /// types, after `before` selections were received.
        fn wl_offered(&mut self, primary: bool, before: usize, mime: &str) {
            self.wait("the offer to the Wayland client", |h| {
                let id = match primary {
                    true => h.seen.primary.as_ref().map(Proxy::id),
                    false => h.seen.clipboard.as_ref().map(Proxy::id),
                };
                let types = id.and_then(|id| h.seen.types.get(&id));
                (h.seen.selections > before && types.is_some_and(|t| t.iter().any(|m| m == mime)))
                    .then_some(())
            });
        }

        fn focus(&mut self, window: &Window) {
            self.state.focus(Some(window.clone()));
            self.idle(Duration::from_millis(20));
        }
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

    fn connect(display: u32) -> (RustConnection, XWindow) {
        let (conn, screen) = RustConnection::connect(Some(&format!(":{display}"))).unwrap();
        let root = conn.setup().roots[screen].root;
        (conn, root)
    }

    /// An X11 window shown until `stop` is dropped: with it, an X11 client may use the
    /// selections.
    fn x11_show(display: u32) -> mpsc::Sender<()> {
        let (stop, stopped) = mpsc::channel::<()>();
        thread::spawn(move || {
            let (conn, root) = connect(display);
            let window = conn.generate_id().unwrap();
            let white = conn.setup().roots[0].white_pixel;
            let aux = CreateWindowAux::new().background_pixel(white);
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
            conn.map_window(window).unwrap();
            conn.flush().unwrap();
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(Duration::from_millis(10))
            {
                while conn.poll_for_event().unwrap().is_some() {}
            }
        });
        stop
    }

    /// An INCR transfer under way: the requestor reads a piece and deletes the property, and
    /// the owner puts the next one.
    struct Incr {
        requestor: XWindow,
        property: Atom,
        target: Atom,
        data: Arc<Vec<u8>>,
        sent: usize,
    }

    /// An X11 client that owns a selection until `stop` is dropped. `asked` gets a message each
    /// time the window manager asks for its targets, which it does when it takes note of it.
    struct Owner {
        _stop: mpsc::Sender<()>,
        asked: mpsc::Receiver<()>,
    }

    /// Takes `selection` with what each target gives, as an X11 application does on a copy.
    fn x11_own(
        display: u32,
        selection: &'static str,
        offers: Vec<(&'static str, Arc<Vec<u8>>)>,
    ) -> Owner {
        let (stop, stopped) = mpsc::channel::<()>();
        let (ask, asked) = mpsc::channel();
        thread::spawn(move || {
            let (conn, root) = connect(display);
            let window = conn.generate_id().unwrap();
            let aux = CreateWindowAux::new();
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
                &aux,
            )
            .unwrap();
            let selection = atom(&conn, selection);
            let targets = atom(&conn, "TARGETS");
            let incr = atom(&conn, "INCR");
            let offers: Vec<(Atom, Arc<Vec<u8>>)> = offers
                .into_iter()
                .map(|(t, d)| (atom(&conn, t), d))
                .collect();
            conn.set_selection_owner(window, selection, CURRENT_TIME)
                .unwrap();
            conn.flush().unwrap();
            let mut transfers: Vec<Incr> = Vec::new();
            while let Err(mpsc::RecvTimeoutError::Timeout) =
                stopped.recv_timeout(Duration::from_millis(1))
            {
                while let Some(event) = conn.poll_for_event().unwrap() {
                    match event {
                        Event::SelectionRequest(req) => {
                            if req.target == targets {
                                let _ = ask.send(());
                            }
                            let all: Vec<Atom> = std::iter::once(targets)
                                .chain(offers.iter().map(|o| o.0))
                                .collect();
                            let data = offers.iter().find(|o| o.0 == req.target);
                            let started = answer(&conn, &req, (targets, &all), data, incr);
                            transfers.extend(started);
                        }
                        Event::PropertyNotify(n) if n.state == Property::DELETE => {
                            let Some(i) = transfers
                                .iter()
                                .position(|t| t.requestor == n.window && t.property == n.atom)
                            else {
                                continue;
                            };
                            let t = &mut transfers[i];
                            let end = (t.sent + X11_CHUNK).min(t.data.len());
                            let piece = &t.data[t.sent..end];
                            conn.change_property8(
                                PropMode::REPLACE,
                                t.requestor,
                                t.property,
                                t.target,
                                piece,
                            )
                            .unwrap();
                            if piece.is_empty() {
                                // The empty piece ends the transfer.
                                transfers.remove(i);
                            } else {
                                t.sent = end;
                            }
                            conn.flush().unwrap();
                        }
                        _ => {}
                    }
                }
            }
        });
        Owner { _stop: stop, asked }
    }

    /// Answers a selection request: the targets, the data, or INCR for large data.
    fn answer(
        conn: &RustConnection,
        req: &SelectionRequestEvent,
        (targets, all): (Atom, &[Atom]),
        data: Option<&(Atom, Arc<Vec<u8>>)>,
        incr: Atom,
    ) -> Option<Incr> {
        let property = if req.property == NONE {
            req.target
        } else {
            req.property
        };
        let mut started = None;
        let property = if req.target == targets {
            conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                property,
                AtomEnum::ATOM,
                all,
            )
            .unwrap();
            property
        } else if let Some((target, data)) = data {
            if data.len() > X11_CHUNK {
                let events =
                    ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
                conn.change_window_attributes(req.requestor, &events)
                    .unwrap();
                let len = [data.len() as u32];
                conn.change_property32(PropMode::REPLACE, req.requestor, property, incr, &len)
                    .unwrap();
                started = Some(Incr {
                    requestor: req.requestor,
                    property,
                    target: *target,
                    data: data.clone(),
                    sent: 0,
                });
            } else {
                conn.change_property8(PropMode::REPLACE, req.requestor, property, *target, data)
                    .unwrap();
            }
            property
        } else {
            NONE
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: req.time,
            requestor: req.requestor,
            selection: req.selection,
            target: req.target,
            property,
        };
        conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify)
            .unwrap();
        conn.flush().unwrap();
        started
    }

    /// Reads `selection` as `target`, as an X11 application does on a paste, INCR included.
    /// `None` if the owner refused, or gave no answer within `budget`.
    fn x11_read(display: u32, selection: &str, target: &str, budget: Duration) -> Option<Vec<u8>> {
        let (conn, root) = connect(display);
        let window = conn.generate_id().unwrap();
        let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
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
            &aux,
        )
        .unwrap();
        let (selection, target) = (atom(&conn, selection), atom(&conn, target));
        let (property, incr) = (atom(&conn, "D29_PASTE"), atom(&conn, "INCR"));
        conn.convert_selection(window, selection, target, property, CURRENT_TIME)
            .unwrap();
        conn.flush().unwrap();
        let deadline = Instant::now() + budget;
        let next = |what: &dyn Fn(&Event) -> bool| loop {
            match conn.poll_for_event().unwrap() {
                Some(event) if what(&event) => return Some(event),
                Some(_) => {}
                None if Instant::now() > deadline => return None,
                None => thread::sleep(Duration::from_millis(1)),
            }
        };
        let notified = next(&|e| matches!(e, Event::SelectionNotify(_)))?;
        let Event::SelectionNotify(notify) = notified else {
            return None;
        };
        if notify.property == NONE {
            return None;
        }
        let take = || {
            conn.get_property(true, window, property, AtomEnum::ANY, 0, u32::MAX / 4)
                .unwrap()
                .reply()
                .unwrap()
        };
        let reply = take();
        if reply.type_ != incr {
            return Some(reply.value);
        }
        let mut data = Vec::new();
        loop {
            next(&|e| {
                matches!(e, Event::PropertyNotify(n)
                    if n.atom == property && n.state == Property::NEW_VALUE)
            })?;
            let piece = take().value;
            if piece.is_empty() {
                return Some(data);
            }
            data.extend(piece);
        }
    }

    /// Reads in a thread, while the test runs the compositor, until the X11 client gets
    /// `expected`: the window manager may take the selection a moment after the Wayland side.
    fn x11_paste_until(
        display: u32,
        selection: &'static str,
        target: &'static str,
        expected: Arc<Vec<u8>>,
    ) -> mpsc::Receiver<bool> {
        let (sender, reply) = mpsc::channel();
        thread::spawn(move || {
            let deadline = Instant::now() + STEP_BUDGET;
            let budget = Duration::from_secs(5);
            let mut got = x11_read(display, selection, target, budget);
            while got.as_deref() != Some(&expected[..]) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(20));
                got = x11_read(display, selection, target, budget);
            }
            let _ = sender.send(got.as_deref() == Some(&expected[..]));
        });
        reply
    }

    /// Reads once, in a thread.
    fn x11_paste(
        display: u32,
        selection: &'static str,
        target: &'static str,
    ) -> mpsc::Receiver<Option<Vec<u8>>> {
        let (sender, reply) = mpsc::channel();
        thread::spawn(move || {
            let got = x11_read(display, selection, target, Duration::from_secs(5));
            let _ = sender.send(got);
        });
        reply
    }

    /// The preview of the newest item of the clipboard history.
    fn top(state: &State) -> Option<String> {
        let entries = state.clipboard_history().entries();
        entries.first().map(|e| e.content.preview.clone())
    }

    #[test]
    fn selections_pass_between_wayland_and_x11_but_not_while_locked() {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), 1.0);
        // No lock program: nothing is started, the compositor shows its rescue screen and the
        // session stays locked.
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
            devices: None,
            _file: None,
        };
        let mut startup = |_: &mut Harness| started.get();
        h.wait_within(XWAYLAND_STARTUP_BUDGET, "XWayland startup", &mut startup);
        let display = started.get().flatten().expect("XWayland did not start");
        h.wait("the globals", |h| {
            (!h.seen.globals.is_empty()).then_some(())
        });

        let _shown = x11_show(display);
        let x11 = h.wait("the X11 window", |h| {
            x11_window(&h.state).filter(|w| h.state.focused_window().as_ref() == Some(w))
        });
        h.show_window();
        let wayland = wayland_window(&h.state).unwrap();

        // Wayland to X11: text, a picture of 3 MB (INCR from the window manager), the primary
        // selection. smithay gives `UTF8_STRING` to X11 for `text/plain;charset=utf-8`.
        let text = Arc::new(b"copied in Wayland".to_vec());
        h.focus(&wayland);
        h.wl_copy(false, &[(TEXT, &text)]);
        h.focus(&x11);
        let got = x11_paste_until(display, "CLIPBOARD", "UTF8_STRING", text);
        assert!(h.join("the text read by X11", got));

        let picture = bytes(3 << 20, 1);
        h.focus(&wayland);
        h.wl_copy(
            false,
            &[(PNG, &picture), (TEXT, &Arc::new(b"alt".to_vec()))],
        );
        h.focus(&x11);
        let got = x11_paste_until(display, "CLIPBOARD", PNG, picture);
        assert!(h.join("the picture read by X11", got));

        let primary = Arc::new(b"selected in Wayland".to_vec());
        h.focus(&wayland);
        h.wl_copy(true, &[(TEXT, &primary)]);
        h.focus(&x11);
        let got = x11_paste_until(display, "PRIMARY", "UTF8_STRING", primary);
        assert!(h.join("the primary selection read by X11", got));

        // A screenshot, the compositor's own selection, reaches X11 too.
        let shot = bytes(200 << 10, 2);
        h.state.copy_png(Arc::from(&shot[..]));
        let got = x11_paste_until(display, "CLIPBOARD", PNG, shot);
        assert!(h.join("the screenshot read by X11", got));

        // X11 to Wayland: text and a picture of 4 MB sent by INCR, then the primary selection.
        let text = Arc::new(b"copied in X11".to_vec());
        let picture = bytes(4 << 20, 3);
        let offers = vec![("UTF8_STRING", text.clone()), (PNG, picture.clone())];
        let clipboard = x11_own(display, "CLIPBOARD", offers);
        h.join("the X11 targets asked", clipboard.asked);
        h.wait("the X11 clipboard on the Wayland side", |h| {
            let data = current_data_device_selection_userdata(&h.state.seat);
            data.is_some_and(|d| d.is_none()).then_some(())
        });
        let before = h.seen.selections;
        h.focus(&wayland);
        h.wl_offered(false, before, PNG);
        assert_eq!(h.wl_paste(false, TEXT), *text);
        assert_eq!(h.wl_paste(false, PNG), *picture);

        // The clipboard history keeps a text copied in X11 (lot D-42).
        h.focus(&x11);
        let kept = Arc::new(b"kept from X11".to_vec());
        let owner = x11_own(display, "CLIPBOARD", vec![("UTF8_STRING", kept)]);
        h.join("the X11 targets asked", owner.asked);
        h.wait("the X11 copy in the history", |h| {
            (top(&h.state).as_deref() == Some("kept from X11")).then_some(())
        });

        let primary = Arc::new(b"selected in X11".to_vec());
        h.focus(&x11);
        let owner = x11_own(display, "PRIMARY", vec![("UTF8_STRING", primary.clone())]);
        h.join("the X11 targets asked", owner.asked);
        h.wait("the X11 primary selection on the Wayland side", |h| {
            let data = current_primary_selection_userdata(&h.state.seat);
            data.is_some_and(|d| d.is_none()).then_some(())
        });
        let before = h.seen.selections;
        h.focus(&wayland);
        h.wl_offered(true, before, TEXT);
        assert_eq!(h.wl_paste(true, TEXT), *primary);

        // Locked: the clipboard comes from X11, the primary selection from Wayland.
        let mine = Arc::new(b"selected before the lock".to_vec());
        h.wl_copy(true, &[(TEXT, &mine)]);
        h.state.lock_session();
        assert!(h.state.is_locked());
        h.idle(Duration::from_millis(100));
        let before = h.seen.selections;
        // An offer received before the lock gives nothing.
        assert_eq!(h.wl_paste(false, TEXT), b"");
        // X11 reads nothing of Wayland's. Its window is not given the keyboard: the lock keeps
        // it.
        h.state.focus(Some(x11.clone()));
        assert!(h.state.focused_window().is_none());
        let got = x11_paste(display, "PRIMARY", "UTF8_STRING");
        assert_eq!(h.join("the refused X11 read", got), None);
        // An X11 selection does not reach Wayland.
        let owner = x11_own(display, "PRIMARY", vec![("UTF8_STRING", primary)]);
        h.join("the X11 targets asked", owner.asked);
        h.idle(Duration::from_millis(300));
        assert!(current_primary_selection_userdata(&h.state.seat).is_none());
        // No Wayland client is offered a selection, not even the clipboard set while locked.
        h.state.copy_png(Arc::from(&b"locked"[..]));
        h.idle(Duration::from_millis(100));
        assert_eq!(h.seen.selections, before);

        h.state.shutdown();
    }

    /// What a panel of the shell sees of the clipboard history protocol.
    #[derive(Default)]
    struct Panel {
        globals: Vec<(u32, String, u32)>,
        /// id, kind and preview of each item, by id.
        items: HashMap<u32, (u32, String)>,
        thumbnails: HashMap<u32, Vec<u8>>,
        enabled: Option<bool>,
        batches: usize,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Panel {
        fn event(
            panel: &mut Self,
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
                panel.globals.push((name, interface, version));
            }
        }
    }

    impl Dispatch<SherlockClipboardHistoryV1, ()> for Panel {
        fn event(
            panel: &mut Self,
            _: &SherlockClipboardHistoryV1,
            event: history::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                history::Event::State { enabled } => panel.enabled = Some(enabled != 0),
                history::Event::Item {
                    id, kind, preview, ..
                } => {
                    let kind = match kind {
                        wayland_client::WEnum::Value(kind) => kind as u32,
                        wayland_client::WEnum::Unknown(value) => value,
                    };
                    panel.items.insert(id, (kind, preview));
                }
                history::Event::Thumbnail { id, fd, size } => {
                    let mut png = vec![0; size as usize];
                    std::os::unix::fs::FileExt::read_exact_at(&File::from(fd), &mut png, 0)
                        .unwrap();
                    panel.thumbnails.insert(id, png);
                }
                history::Event::Removed { id } => {
                    panel.items.remove(&id);
                }
                history::Event::Done => panel.batches += 1,
                _ => {}
            }
        }
    }

    /// The globals a client of `socket` sees, and its queue, once the compositor answered.
    fn connect_panel(h: &mut Harness, shell: bool) -> (Connection, EventQueue<Panel>, Panel) {
        let (server_side, client_side) = UnixStream::pair().unwrap();
        if shell {
            h.state.insert_shell_client(server_side);
        } else {
            h.state.insert_client(server_side);
        }
        let conn = Connection::from_socket(client_side).unwrap();
        let mut queue = conn.new_event_queue();
        let _registry = conn.display().get_registry(&queue.handle(), ());
        let mut panel = Panel::default();
        pump_panel(h, &conn, &mut queue, &mut panel);
        (conn, queue, panel)
    }

    fn pump_panel(
        h: &mut Harness,
        conn: &Connection,
        queue: &mut EventQueue<Panel>,
        panel: &mut Panel,
    ) {
        for _ in 0..10 {
            let _ = conn.flush();
            h.pump();
            if let Some(guard) = queue.prepare_read() {
                let _ = guard.read();
            }
            queue.dispatch_pending(panel).unwrap();
        }
    }

    /// The clipboard history (lot D-42) between Wayland clients: copies kept, a copy already
    /// there moved to the top, pictures with a thumbnail, no secret, an item put back, nothing
    /// once the history is off.
    #[test]
    fn the_history_keeps_copies_but_no_secret() {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), 1.0);
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
            devices: None,
            _file: None,
        };
        h.wait("the globals", |h| {
            (!h.seen.globals.is_empty()).then_some(())
        });
        h.show_window();
        let window = wayland_window(&h.state).unwrap();
        h.focus(&window);
        let copy = |h: &mut Harness, text: &str| {
            h.wl_copy(false, &[(TEXT, &Arc::new(text.as_bytes().to_vec()))]);
            let text = text.to_owned();
            h.wait("the copy in the history", |h| {
                (top(&h.state) == Some(text.clone())).then_some(())
            });
        };
        copy(&mut h, "first");
        copy(&mut h, "second");
        copy(&mut h, "first");
        assert_eq!(h.state.clipboard_history().entries().len(), 2);

        // A password manager's copy is never kept.
        let secret = Arc::new(b"hunter2".to_vec());
        let hint = Arc::new(b"secret".to_vec());
        h.wl_copy(false, &[(TEXT, &secret), (compositor::SECRET_HINT, &hint)]);
        h.idle(Duration::from_millis(300));
        assert_eq!(top(&h.state).as_deref(), Some("first"));

        // A picture, with its thumbnail.
        let mut png = Vec::new();
        image::RgbaImage::new(64, 32)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        h.wl_copy(false, &[(PNG, &Arc::new(png))]);
        h.wait("the picture in the history", |h| {
            (top(&h.state).as_deref() == Some("64 × 32")).then_some(())
        });
        let entry = h.state.clipboard_history().entries()[0].clone();
        assert_eq!(
            entry.content.kind,
            compositor::clipboard_history::Kind::Image
        );
        assert!(entry.content.thumbnail.is_some());

        // Only a client of the shell socket sees the history.
        const GLOBAL: &str = "sherlock_clipboard_history_v1";
        let (_, _, app) = connect_panel(&mut h, false);
        assert!(app.globals.iter().all(|g| g.1 != GLOBAL));
        let (conn, mut queue, mut panel) = connect_panel(&mut h, true);
        let (name, _, _) = panel
            .globals
            .iter()
            .find(|g| g.1 == GLOBAL)
            .cloned()
            .unwrap();
        let registry = conn.display().get_registry(&queue.handle(), ());
        let manager: SherlockClipboardHistoryV1 = registry.bind(name, 1, &queue.handle(), ());
        pump_panel(&mut h, &conn, &mut queue, &mut panel);
        assert_eq!(panel.enabled, Some(true));
        assert_eq!(panel.items.len(), 3);
        let picture = entry.id;
        assert_eq!(panel.items[&picture], (3, "64 × 32".to_owned()));
        let thumbnail = image::load_from_memory(&panel.thumbnails[&picture]).unwrap();
        assert_eq!(thumbnail.width(), 64);
        // Removed from the panel: gone from the history, and the panel told.
        manager.remove(picture);
        pump_panel(&mut h, &conn, &mut queue, &mut panel);
        assert!(h.state.clipboard_history().get(picture).is_none());
        assert_eq!(panel.items.len(), 2);

        // An item put back is offered again, and goes to the top without being read twice.
        let second = h.state.clipboard_history().entries()[1].id;
        let before = h.seen.selections;
        manager.paste(second);
        pump_panel(&mut h, &conn, &mut queue, &mut panel);
        h.wl_offered(false, before, TEXT);
        assert_eq!(h.wl_paste(false, TEXT), b"second");
        assert_eq!(top(&h.state).as_deref(), Some("second"));
        assert_eq!(h.state.clipboard_history().entries().len(), 2);
        // Pinned, it stays when the rest is cleared.
        manager.set_pinned(second, 1);
        manager.clear();
        pump_panel(&mut h, &conn, &mut queue, &mut panel);
        assert_eq!(h.state.clipboard_history().entries().len(), 1);
        assert_eq!(panel.items.len(), 1);
        assert!(panel.items.contains_key(&second));

        // Kept on disk, for the user alone, and the file goes when the setting is off.
        let file = std::env::temp_dir().join(format!("clipboard-history-{}", std::process::id()));
        let _ = std::fs::remove_file(&file);
        h.state.set_clipboard_disk(Some(file.clone()));
        let mut settings = h.state.clipboard_settings();
        settings.keep_on_disk = true;
        h.state.set_clipboard_settings(settings);
        h.idle(Duration::from_millis(100));
        copy(&mut h, "on disk");
        h.wait("the history on disk", |_| {
            std::fs::read(&file)
                .ok()
                .filter(|bytes| bytes.windows(7).any(|w| w == b"on disk"))
        });
        settings.keep_on_disk = false;
        h.state.set_clipboard_settings(settings);
        h.wait("the file removed", |_| (!file.exists()).then_some(()));

        // Turned off: forgotten, and nothing more is kept.
        settings.history = false;
        h.state.set_clipboard_settings(settings);
        assert!(h.state.clipboard_history().entries().is_empty());
        pump_panel(&mut h, &conn, &mut queue, &mut panel);
        assert_eq!(panel.enabled, Some(false));
        assert!(panel.items.is_empty());
        h.wl_copy(false, &[(TEXT, &Arc::new(b"off".to_vec()))]);
        h.idle(Duration::from_millis(300));
        assert!(h.state.clipboard_history().entries().is_empty());
        h.state.shutdown();
    }
}
