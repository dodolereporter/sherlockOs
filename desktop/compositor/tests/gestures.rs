//! Touchpad gestures (lot D-31), from simulated libinput events: a fake input backend hands
//! swipes, pinches and holds to `State::process_input_event`, as libinput's backend does in the
//! real session. Three finger swipes move the workspaces with the fingers; gestures of fewer
//! fingers reach the client under the pointer through `zwp_pointer_gestures_v1`.

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
    use desktop_config::touchpad::{Config, SwipeDown};
    use smithay::{
        backend::input::{self as backend, InputBackend, InputEvent, UnusedEvent},
        reexports::calloop::EventLoop,
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_pointer::WlPointer, wl_registry,
            wl_seat::WlSeat, wl_shm::Format, wl_shm::WlShm, wl_shm_pool::WlShmPool,
            wl_surface::WlSurface,
        },
    };
    use wayland_protocols::wp::pointer_gestures::zv1::client::{
        zwp_pointer_gesture_hold_v1::{self, ZwpPointerGestureHoldV1},
        zwp_pointer_gesture_pinch_v1::{self, ZwpPointerGesturePinchV1},
        zwp_pointer_gesture_swipe_v1::{self, ZwpPointerGestureSwipeV1},
        zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::XdgToplevel,
        xdg_wm_base::{self, XdgWmBase},
    };

    // A fake libinput: one device, one event type for every gesture.

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct Pad;

    impl backend::Device for Pad {
        fn id(&self) -> String {
            "pad".into()
        }
        fn name(&self) -> String {
            "Fake touchpad".into()
        }
        fn has_capability(&self, capability: backend::DeviceCapability) -> bool {
            capability == backend::DeviceCapability::Gesture
        }
        fn usb_id(&self) -> Option<(u32, u32)> {
            None
        }
        fn syspath(&self) -> Option<std::path::PathBuf> {
            None
        }
    }

    /// A gesture event: time in milliseconds, fingers at its start, movement, pinch scale and
    /// rotation, cancelled at its end.
    #[derive(Debug, Clone, Copy, Default)]
    struct Gesture {
        time: u32,
        fingers: u32,
        delta: (f64, f64),
        scale: f64,
        rotation: f64,
        cancelled: bool,
    }

    struct Fake;

    impl backend::Event<Fake> for Gesture {
        fn time(&self) -> u64 {
            u64::from(self.time) * 1000
        }
        fn device(&self) -> Pad {
            Pad
        }
    }
    impl backend::GestureBeginEvent<Fake> for Gesture {
        fn fingers(&self) -> u32 {
            self.fingers
        }
    }
    impl backend::GestureEndEvent<Fake> for Gesture {
        fn cancelled(&self) -> bool {
            self.cancelled
        }
    }
    impl backend::GestureSwipeBeginEvent<Fake> for Gesture {}
    impl backend::GestureSwipeEndEvent<Fake> for Gesture {}
    impl backend::GesturePinchBeginEvent<Fake> for Gesture {}
    impl backend::GesturePinchEndEvent<Fake> for Gesture {}
    impl backend::GestureHoldBeginEvent<Fake> for Gesture {}
    impl backend::GestureHoldEndEvent<Fake> for Gesture {}
    impl backend::GestureSwipeUpdateEvent<Fake> for Gesture {
        fn delta_x(&self) -> f64 {
            self.delta.0
        }
        fn delta_y(&self) -> f64 {
            self.delta.1
        }
    }
    impl backend::GesturePinchUpdateEvent<Fake> for Gesture {
        fn delta_x(&self) -> f64 {
            self.delta.0
        }
        fn delta_y(&self) -> f64 {
            self.delta.1
        }
        fn scale(&self) -> f64 {
            self.scale
        }
        fn rotation(&self) -> f64 {
            self.rotation
        }
    }

    impl InputBackend for Fake {
        type Device = Pad;
        type KeyboardKeyEvent = UnusedEvent;
        type PointerAxisEvent = UnusedEvent;
        type PointerButtonEvent = UnusedEvent;
        type PointerMotionEvent = UnusedEvent;
        type PointerMotionAbsoluteEvent = UnusedEvent;
        type GestureSwipeBeginEvent = Gesture;
        type GestureSwipeUpdateEvent = Gesture;
        type GestureSwipeEndEvent = Gesture;
        type GesturePinchBeginEvent = Gesture;
        type GesturePinchUpdateEvent = Gesture;
        type GesturePinchEndEvent = Gesture;
        type GestureHoldBeginEvent = Gesture;
        type GestureHoldEndEvent = Gesture;
        type TouchDownEvent = UnusedEvent;
        type TouchUpEvent = UnusedEvent;
        type TouchMotionEvent = UnusedEvent;
        type TouchCancelEvent = UnusedEvent;
        type TouchFrameEvent = UnusedEvent;
        type TabletToolAxisEvent = UnusedEvent;
        type TabletToolProximityEvent = UnusedEvent;
        type TabletToolTipEvent = UnusedEvent;
        type TabletToolButtonEvent = UnusedEvent;
        type SwitchToggleEvent = UnusedEvent;
        type SpecialEvent = ();
    }

    /// A swipe of `fingers`, moved by `steps` of `delta` every 10 ms, ended at the last one.
    fn swipe(state: &mut State, fingers: u32, steps: u32, delta: (f64, f64), cancelled: bool) {
        let begin = Gesture {
            fingers,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeBegin { event: begin });
        for i in 1..=steps {
            let event = Gesture {
                time: i * 10,
                delta,
                ..Gesture::default()
            };
            state.process_input_event(InputEvent::<Fake>::GestureSwipeUpdate { event });
        }
        let end = Gesture {
            time: steps * 10,
            cancelled,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeEnd { event: end });
    }

    fn desktop() -> (EventLoop<'static, State>, State) {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), 1.0);
        (event_loop, state)
    }

    /// Long enough after `state.now()` for any slide to have settled.
    fn later(state: &State) -> Duration {
        state.now() + Duration::from_secs(5)
    }

    #[test]
    fn three_fingers_move_the_workspaces_with_the_fingers() {
        let (_event_loop, mut state) = desktop();
        assert_eq!(state.workspaces().active(), "1");
        let begin = Gesture {
            fingers: 3,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeBegin { event: begin });
        // Fingers to the left, 80 units: the row follows them, a fifth of a workspace.
        for i in 1..=4 {
            let event = Gesture {
                time: i * 10,
                delta: (-20.0, 0.0),
                ..Gesture::default()
            };
            state.process_input_event(InputEvent::<Fake>::GestureSwipeUpdate { event });
            let expected = f64::from(i) * 20.0 / compositor::gestures::WORKSPACE_DISTANCE;
            assert!((state.workspace_view(state.now()) - expected).abs() < 1e-9);
        }
        // Held where the fingers are: no animation, nothing changes with time.
        assert!(!state.workspaces_sliding(later(&state)));
        assert_eq!(state.workspace_view(later(&state)), 0.2);
        assert_eq!(state.workspaces().active(), "1");
        // Lifted while moving fast: the throw lands on the next workspace, from where the
        // fingers left the row, at their speed.
        let end = Gesture {
            time: 40,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeEnd { event: end });
        assert_eq!(state.workspaces().active(), "2");
        let now = state.now();
        assert!(state.workspaces_sliding(now));
        assert!((state.workspace_view(now) - 0.2).abs() < 0.05);
        assert!(state.workspace_view(now + Duration::from_millis(5)) > state.workspace_view(now));
        assert_eq!(state.workspace_view(later(&state)), 1.0);
    }

    #[test]
    fn a_slow_or_cancelled_swipe_comes_back() {
        let (_event_loop, mut state) = desktop();
        // A quarter of the way, slowly: back to the first.
        swipe(&mut state, 3, 50, (-2.0, 0.0), false);
        assert_eq!(state.workspaces().active(), "1");
        assert!(state.workspace_view(state.now()) > 0.0);
        assert_eq!(state.workspace_view(later(&state)), 0.0);
        // Past the middle, but cancelled by libinput: back as well.
        swipe(&mut state, 3, 30, (-10.0, 0.0), true);
        assert_eq!(state.workspaces().active(), "1");
        // Past the middle: the next one.
        swipe(&mut state, 3, 30, (-10.0, 0.0), false);
        assert_eq!(state.workspaces().active(), "2");
        // Below the lock distance, nothing moves.
        swipe(&mut state, 3, 1, (-10.0, 0.0), false);
        assert_eq!(state.workspaces().active(), "2");
    }

    #[test]
    fn the_ends_of_the_row_resist_and_spring_back() {
        let (_event_loop, mut state) = desktop();
        let begin = Gesture {
            fingers: 3,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeBegin { event: begin });
        // Fingers to the right on the first workspace: half a workspace of travel, much less
        // of movement.
        let event = Gesture {
            time: 10,
            delta: (200.0, 0.0),
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeUpdate { event });
        let view = state.workspace_view(state.now());
        assert!(view < 0.0 && view > -0.3, "{view}");
        let end = Gesture {
            time: 10,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeEnd { event: end });
        assert_eq!(state.workspaces().active(), "1");
        assert_eq!(state.workspace_view(later(&state)), 0.0);
        // At the other end, one workspace at a time.
        swipe(&mut state, 3, 30, (-10.0, 0.0), false);
        swipe(&mut state, 3, 30, (-10.0, 0.0), false);
        swipe(&mut state, 3, 30, (-10.0, 0.0), false);
        assert_eq!(state.workspaces().active(), "3");
        assert_eq!(state.workspace_view(later(&state)), 2.0);
    }

    #[test]
    fn a_swipe_cut_short_by_a_console_switch_comes_back() {
        let (_event_loop, mut state) = desktop();
        let begin = Gesture {
            fingers: 3,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeBegin { event: begin });
        let event = Gesture {
            time: 10,
            delta: (-300.0, 0.0),
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureSwipeUpdate { event });
        assert!(state.workspace_view(later(&state)) > 0.5);
        // The session pauses: libinput says no more, the row is let go and comes back.
        state.cancel_gestures();
        assert_eq!(state.workspaces().active(), "1");
        assert_eq!(state.workspace_view(later(&state)), 0.0);
    }

    #[test]
    fn four_fingers_and_vertical_swipes_do_not_change_workspace() {
        let (_event_loop, mut state) = desktop();
        swipe(&mut state, 4, 30, (-10.0, 0.0), false);
        assert_eq!(state.workspace_view(state.now()), 0.0);
        // Up: the overview, reserved; down without a window: nothing.
        swipe(&mut state, 3, 20, (0.0, -10.0), false);
        swipe(&mut state, 3, 20, (0.0, 10.0), false);
        assert_eq!(state.workspaces().active(), "1");
        assert_eq!(state.workspace_view(state.now()), 0.0);
    }

    // A Wayland client with a window and the pointer gestures of its pointer.

    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        configured: bool,
        /// Gesture events received, by name.
        gestures: Vec<String>,
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

    impl Dispatch<ZwpPointerGestureSwipeV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwpPointerGestureSwipeV1,
            event: zwp_pointer_gesture_swipe_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            client.gestures.push(match event {
                zwp_pointer_gesture_swipe_v1::Event::Begin { fingers, .. } => {
                    format!("swipe begin {fingers}")
                }
                zwp_pointer_gesture_swipe_v1::Event::Update { dx, .. } => {
                    format!("swipe update {dx}")
                }
                zwp_pointer_gesture_swipe_v1::Event::End { cancelled, .. } => {
                    format!("swipe end {cancelled}")
                }
                _ => "swipe ?".into(),
            });
        }
    }

    impl Dispatch<ZwpPointerGesturePinchV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwpPointerGesturePinchV1,
            event: zwp_pointer_gesture_pinch_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            client.gestures.push(match event {
                zwp_pointer_gesture_pinch_v1::Event::Begin { fingers, .. } => {
                    format!("pinch begin {fingers}")
                }
                zwp_pointer_gesture_pinch_v1::Event::Update { scale, .. } => {
                    format!("pinch update {scale}")
                }
                zwp_pointer_gesture_pinch_v1::Event::End { cancelled, .. } => {
                    format!("pinch end {cancelled}")
                }
                _ => "pinch ?".into(),
            });
        }
    }

    impl Dispatch<ZwpPointerGestureHoldV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwpPointerGestureHoldV1,
            event: zwp_pointer_gesture_hold_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            client.gestures.push(match event {
                zwp_pointer_gesture_hold_v1::Event::Begin { fingers, .. } => {
                    format!("hold begin {fingers}")
                }
                zwp_pointer_gesture_hold_v1::Event::End { cancelled, .. } => {
                    format!("hold end {cancelled}")
                }
                _ => "hold ?".into(),
            });
        }
    }

    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlSeat);
    delegate_noop!(Client: ignore WlPointer);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);
    delegate_noop!(Client: ignore XdgToplevel);
    delegate_noop!(Client: ZwpPointerGesturesV1);

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        registry: wl_registry::WlRegistry,
        /// Buffer files, alive as long as their buffers.
        files: Vec<File>,
    }

    impl Harness {
        /// One screen of 800 × 600 and a client with a window of 100 × 80 in its center, the
        /// pointer over it, and the three gestures of its pointer.
        fn start() -> (Self, XdgToplevel) {
            let (event_loop, mut state) = desktop();
            let (server, stream) = UnixStream::pair().unwrap();
            state.insert_client(server);
            let conn = Connection::from_socket(stream).unwrap();
            let queue = conn.new_event_queue();
            let registry = conn.display().get_registry(&queue.handle(), ());
            let mut h = Self {
                event_loop,
                state,
                conn,
                queue,
                client: Client::default(),
                registry,
                files: Vec::new(),
            };
            h.run_until("the globals", |h| {
                h.sees("wl_shm") && h.sees("zwp_pointer_gestures_v1")
            });
            let qh = h.queue.handle();
            let seat: WlSeat = h.bind();
            let pointer = seat.get_pointer(&qh, ());
            let gestures: ZwpPointerGesturesV1 = h.bind();
            gestures.get_swipe_gesture(&pointer, &qh, ());
            gestures.get_pinch_gesture(&pointer, &qh, ());
            gestures.get_hold_gesture(&pointer, &qh, ());
            let compositor: WlCompositor = h.bind();
            let wm_base: XdgWmBase = h.bind();
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            surface.commit();
            h.run_until("the first configure", |h| h.client.configured);
            h.draw(&surface, 100, 80);
            h.run_until("the window shown", |h| h.state.focused_window().is_some());
            h.state.pointer_motion((400.0, 300.0).into(), 1);
            h.turns();
            (h, toplevel)
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
            let version = (*version).min(I::interface().version);
            self.registry.bind(*id, version, &self.queue.handle(), ())
        }

        /// A buffer of `w` × `h` attached and committed on `surface`.
        fn draw(&mut self, surface: &WlSurface, w: i32, h: i32) {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "compositor-gestures-test-{}-{n}",
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
            file.set_len((w * h * 4) as u64).unwrap();
            let qh = self.queue.handle();
            let shm: WlShm = self.bind();
            let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            self.files.push(file);
        }

        fn turn(&mut self) {
            self.conn.flush().ok();
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .unwrap();
            self.state.refresh();
            let _ = self.conn.flush();
            if let Some(guard) = self.queue.prepare_read() {
                // Nothing to read is not an error: try again on the next turn.
                let _ = guard.read();
            }
            let _ = self.queue.dispatch_pending(&mut self.client);
        }

        fn turns(&mut self) {
            for _ in 0..5 {
                self.turn();
            }
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

        /// The gestures received since the last call.
        fn received(&mut self) -> Vec<String> {
            self.turns();
            std::mem::take(&mut self.client.gestures)
        }
    }

    fn pinch(state: &mut State, fingers: u32, scale: f64) {
        let begin = Gesture {
            fingers,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GesturePinchBegin { event: begin });
        let update = Gesture {
            time: 10,
            scale,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GesturePinchUpdate { event: update });
        let end = Gesture {
            time: 20,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GesturePinchEnd { event: end });
    }

    fn hold(state: &mut State, fingers: u32, cancelled: bool) {
        let begin = Gesture {
            fingers,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureHoldBegin { event: begin });
        let end = Gesture {
            time: 300,
            cancelled,
            ..Gesture::default()
        };
        state.process_input_event(InputEvent::<Fake>::GestureHoldEnd { event: end });
    }

    #[test]
    fn two_finger_gestures_reach_the_client_under_the_pointer() {
        let (mut h, _toplevel) = Harness::start();
        pinch(&mut h.state, 2, 1.5);
        assert_eq!(
            h.received(),
            ["pinch begin 2", "pinch update 1.5", "pinch end 0"]
        );
        hold(&mut h.state, 2, true);
        assert_eq!(h.received(), ["hold begin 2", "hold end 1"]);
        // libinput gives no two finger swipe on a touchpad (it scrolls), but a device that
        // does gets it through.
        swipe(&mut h.state, 2, 1, (5.0, 0.0), false);
        assert_eq!(
            h.received(),
            ["swipe begin 2", "swipe update 5", "swipe end 0"]
        );
    }

    #[test]
    fn three_fingers_and_more_stay_with_the_compositor() {
        let (mut h, _toplevel) = Harness::start();
        swipe(&mut h.state, 3, 30, (-10.0, 0.0), false);
        swipe(&mut h.state, 4, 30, (-10.0, 0.0), false);
        pinch(&mut h.state, 3, 0.5);
        hold(&mut h.state, 3, false);
        assert_eq!(h.received(), Vec::<String>::new());
        assert_eq!(h.state.workspaces().active(), "2");
    }

    #[test]
    fn three_fingers_down_minimize_the_active_window_if_set() {
        let (mut h, _toplevel) = Harness::start();
        let window = h.state.focused_window().unwrap();
        // Not far enough: nothing.
        swipe(&mut h.state, 3, 5, (0.0, 10.0), false);
        assert!(!h.state.is_minimized(&window));
        // Off in the setting: nothing.
        h.state.set_touchpad(Config {
            swipe_down: SwipeDown::Nothing,
            ..Config::default()
        });
        swipe(&mut h.state, 3, 20, (0.0, 10.0), false);
        assert!(!h.state.is_minimized(&window));
        // By default: minimized.
        h.state.set_touchpad(Config::default());
        swipe(&mut h.state, 3, 20, (0.0, 10.0), false);
        assert!(h.state.is_minimized(&window));
        assert_eq!(h.received(), Vec::<String>::new());
    }
}
