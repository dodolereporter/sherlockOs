//! The compositor starts without a screen or backend, launches XWayland, and an X11 client
//! (x11rb) opens a managed window and a menu (override-redirect) on it. The compositor loop runs
//! in the test thread, the X11 client in another thread.
//!
//! It needs `Xwayland` on the machine, as launching the compositor does (`docs/compositor.md`).

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        path::{Path, PathBuf},
        rc::Rc,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use compositor::{State, decorations};
    use smithay::{
        backend::input::{ButtonState, KeyState},
        desktop::Window,
        input::keyboard::Keycode,
        reexports::{calloop::EventLoop, wayland_server::Resource as _},
        utils::{Logical, Rectangle},
        wayland::{
            seat::WaylandFocus,
            selection::{
                data_device::current_data_device_selection_userdata,
                primary_selection::current_primary_selection_userdata,
            },
        },
    };
    use x11rb::{
        CURRENT_TIME, NONE,
        connection::Connection,
        protocol::{
            Event,
            xproto::{
                Atom, AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask,
                PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent,
                Window as XWindow, WindowClass,
            },
        },
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    const BTN_LEFT: u32 = 0x110;
    // xkb codes of left Super and of the down arrow. Without `--nested`, the gesture key is
    // Super.
    const KEY_SUPER: u32 = 133;
    const KEY_DOWN: u32 = 116;
    // Time given to each step of the test. XWayland startup gets more: it starts a whole X
    // server, which took 9.9 s on a loaded machine, and went past 30 s with the whole
    // workspace's tests running beside it. The wait ends on XWayland's own signal (ready, or
    // failed if the server exits): the budget only stops a startup that hangs, so a long one
    // costs nothing when the machine is idle.
    const STEP_BUDGET: Duration = Duration::from_secs(10);
    const XWAYLAND_STARTUP_BUDGET: Duration = Duration::from_secs(120);

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Opens a white window: with a background, the server paints it, and XWayland sends content
    /// to the compositor.
    fn open(
        conn: &RustConnection,
        root: XWindow,
        geometry: (i16, i16, u16, u16),
        menu: bool,
    ) -> XWindow {
        let window = conn.generate_id().unwrap();
        let screen = &conn.setup().roots[0];
        let aux = CreateWindowAux::new()
            .background_pixel(screen.white_pixel)
            .override_redirect(u32::from(menu));
        let (x, y, w, h) = geometry;
        conn.create_window(
            0,
            window,
            root,
            x,
            y,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &aux,
        )
        .unwrap();
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            if menu { b"menu" } else { b"test x11" },
        )
        .unwrap();
        conn.map_window(window).unwrap();
        window
    }

    /// What the test asks the X11 client thread to do.
    enum Command {
        /// Take ownership of CLIPBOARD (`true`) or PRIMARY, and answer the window manager's
        /// TARGETS request.
        Own(bool),
        /// Ask for an interactive move of the managed window (`_NET_WM_MOVERESIZE`).
        Move,
        /// Rename the managed window. The window manager sees the new title after everything
        /// the client did before: the test waits for it to know those requests were handled.
        Sync(u32),
        /// Withdraw the managed window (unmap it), as an application hiding in the tray does.
        Withdraw,
        /// Map the managed window again.
        Map,
        /// Iconify the managed window, as `XIconifyWindow` does (`WM_CHANGE_STATE`).
        Iconify,
        /// Ask for no decoration on the managed window (`_MOTIF_WM_HINTS`).
        Undecorate,
        Quit,
    }

    fn atom(conn: &RustConnection, name: &str) -> Atom {
        conn.intern_atom(false, name.as_bytes())
            .unwrap()
            .reply()
            .unwrap()
            .atom
    }

    /// Answers a selection request: the list of targets, or the text itself.
    fn answer(conn: &RustConnection, req: &SelectionRequestEvent, targets: Atom, utf8: Atom) {
        let property = if req.property == NONE {
            req.target
        } else {
            req.property
        };
        let property = if req.target == targets {
            conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                property,
                AtomEnum::ATOM,
                &[targets, utf8],
            )
            .unwrap();
            property
        } else if req.target == utf8 {
            conn.change_property8(PropMode::REPLACE, req.requestor, property, utf8, b"x11")
                .unwrap();
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
    }

    /// The X11 client: a 200 × 100 window and a menu at (50, 60), then the test's commands.
    fn x11_client(display: u32, commands: mpsc::Receiver<Command>) {
        let (conn, screen) = RustConnection::connect(Some(&format!(":{display}"))).unwrap();
        let root = conn.setup().roots[screen].root;
        let window = open(&conn, root, (0, 0, 200, 100), false);
        open(&conn, root, (50, 60, 80, 40), true);
        conn.flush().unwrap();
        let clipboard = atom(&conn, "CLIPBOARD");
        let targets = atom(&conn, "TARGETS");
        let utf8 = atom(&conn, "UTF8_STRING");
        let moveresize = atom(&conn, "_NET_WM_MOVERESIZE");
        let change_state = atom(&conn, "WM_CHANGE_STATE");
        let motif = atom(&conn, "_MOTIF_WM_HINTS");
        loop {
            while let Some(event) = conn.poll_for_event().unwrap() {
                if let Event::SelectionRequest(req) = event {
                    answer(&conn, &req, targets, utf8);
                }
            }
            match commands.recv_timeout(Duration::from_millis(5)) {
                Ok(Command::Own(clip)) => {
                    let selection = if clip {
                        clipboard
                    } else {
                        AtomEnum::PRIMARY.into()
                    };
                    conn.set_selection_owner(window, selection, CURRENT_TIME)
                        .unwrap();
                    conn.flush().unwrap();
                    // The window manager reads the targets before it reports the selection.
                    loop {
                        if let Event::SelectionRequest(req) = conn.wait_for_event().unwrap() {
                            answer(&conn, &req, targets, utf8);
                            if req.target == targets {
                                break;
                            }
                        }
                    }
                }
                Ok(Command::Move) => {
                    // Direction 8 is _NET_WM_MOVERESIZE_MOVE, button 1, source 1 (application).
                    let event = ClientMessageEvent::new(32, window, moveresize, [0, 0, 8, 1, 1]);
                    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
                    conn.send_event(false, root, mask, event).unwrap();
                    conn.flush().unwrap();
                }
                Ok(Command::Sync(n)) => {
                    let title = format!("sync {n}");
                    conn.change_property8(
                        PropMode::REPLACE,
                        window,
                        AtomEnum::WM_NAME,
                        AtomEnum::STRING,
                        title.as_bytes(),
                    )
                    .unwrap();
                    conn.flush().unwrap();
                }
                Ok(Command::Withdraw) => {
                    conn.unmap_window(window).unwrap();
                    conn.flush().unwrap();
                }
                Ok(Command::Map) => {
                    conn.map_window(window).unwrap();
                    conn.flush().unwrap();
                }
                Ok(Command::Iconify) => {
                    // 3 is IconicState (ICCCM 4.1.4).
                    let event = ClientMessageEvent::new(32, window, change_state, [3, 0, 0, 0, 0]);
                    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
                    conn.send_event(false, root, mask, event).unwrap();
                    conn.flush().unwrap();
                }
                Ok(Command::Undecorate) => {
                    // Flags: decorations given (2); decorations: none.
                    let hints = [2, 0, 0, 0, 0];
                    conn.change_property32(PropMode::REPLACE, window, motif, motif, &hints)
                        .unwrap();
                    conn.flush().unwrap();
                }
                Ok(Command::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        time: u32,
    }

    impl Harness {
        fn run_until(&mut self, what: &str, done: impl Fn(&State) -> bool) {
            self.run_within(STEP_BUDGET, what, done);
        }

        fn run_within(&mut self, budget: Duration, what: &str, done: impl Fn(&State) -> bool) {
            let deadline = Instant::now() + budget;
            while Instant::now() < deadline {
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if done(&self.state) {
                    return;
                }
            }
            panic!("timed out waiting for: {what}");
        }

        fn tick(&mut self) -> u32 {
            self.time += 10;
            self.time
        }

        fn key(&mut self, code: u32, state: KeyState) {
            let time = self.tick();
            self.state.keyboard_key(Keycode::new(code), state, time);
        }

        fn motion(&mut self, x: f64, y: f64) {
            let time = self.tick();
            self.state.pointer_motion((x, y).into(), time);
        }

        fn button(&mut self, state: ButtonState) {
            let time = self.tick();
            self.state.pointer_button(BTN_LEFT, state, time);
        }

        /// Waits until the window manager has handled everything the client sent so far.
        fn sync(&mut self, client: &mpsc::Sender<Command>, window: &Window, n: u32) {
            client.send(Command::Sync(n)).unwrap();
            let title = format!("sync {n}");
            self.run_until(&title, |_| State::title(window).as_deref() == Some(&title));
        }
    }

    /// Wayland socket and its lock, in the temporary directory: the tests may run without
    /// `XDG_RUNTIME_DIR`.
    fn wayland_socket() -> (PathBuf, PathBuf) {
        let path = std::env::temp_dir().join(format!("compositor-xwayland-{}", std::process::id()));
        (path.clone(), path.with_extension("lock"))
    }

    /// Managed X11 window or menu.
    fn x11_window(state: &State, menu: bool) -> Option<Window> {
        state
            .space
            .elements()
            .find(|w| {
                w.x11_surface()
                    .is_some_and(|s| s.is_override_redirect() == menu)
            })
            .cloned()
    }

    /// Place of the window in the space, and the one the X server knows for it.
    fn places(
        state: &State,
        window: &Window,
    ) -> (Rectangle<i32, Logical>, Rectangle<i32, Logical>) {
        let surface = window.x11_surface().unwrap();
        (
            state.space.element_geometry(window).unwrap(),
            surface.geometry(),
        )
    }

    #[test]
    fn an_x11_application_is_managed_like_a_wayland_window() {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), 1.0);
        let (wayland, wayland_lock) = wayland_socket();
        state.listen_at(wayland.clone(), &event_loop).unwrap();
        assert!(wayland.exists() && wayland_lock.exists());
        let shell = shell_protocol::socket_path(&wayland);
        assert!(shell.exists(), "no shell socket");
        // The startup result: `Some(None)` if XWayland failed. Its messages are on the test's
        // standard error.
        let started = Rc::new(Cell::new(None));
        let done = started.clone();
        state
            .start_xwayland(&event_loop, move |_, display| done.set(Some(display)))
            .unwrap();
        let mut h = Harness {
            event_loop,
            state,
            time: 0,
        };
        h.run_within(XWAYLAND_STARTUP_BUDGET, "XWayland startup", |_| {
            started.get().is_some()
        });
        let display = started.get().flatten().expect("XWayland did not start");
        assert_eq!(h.state.x_display, Some(display));
        let socket = format!("/tmp/.X11-unix/X{display}");
        assert!(Path::new(&socket).exists());

        let (commands, receiver) = mpsc::channel();
        let client = thread::spawn(move || x11_client(display, receiver));

        // Mapping and title: the window is centered with the title bar of the compositor above
        // it (lot D-08): 42 more, and the X server knows the place of its content.
        h.run_until("the X11 window", |s| x11_window(s, false).is_some());
        let window = x11_window(&h.state, false).unwrap();
        assert_eq!(State::title(&window).as_deref(), Some("test x11"));
        assert_eq!(
            places(&h.state, &window),
            (rect(300, 271, 200, 100), rect(300, 271, 200, 100))
        );

        // Focus: it has the keyboard as soon as XWayland has given it its surface.
        h.run_until("the keyboard on the X11 window", |s| {
            s.focused_window().as_ref() == Some(&window)
        });
        assert!(h.state.window_visual(&window, Duration::ZERO).is_some());
        assert!(decorations::server_side(&window));
        // XWayland is an ordinary client: the shell's globals are not offered to it (ADR 0021).
        let xwayland = window.wl_surface().unwrap().client().unwrap();
        assert!(!State::is_shell_client(&xwayland));

        // The menu is at the place it chose, without taking the keyboard.
        h.run_until("the X11 menu", |s| {
            x11_window(s, true).is_some_and(|m| m.wl_surface().is_some())
        });
        let menu = x11_window(&h.state, true).unwrap();
        assert_eq!(
            h.state.space.element_geometry(&menu),
            Some(rect(50, 60, 80, 40))
        );
        assert!(!decorations::server_side(&menu), "a menu has no title bar");
        let visual = h.state.window_visual(&menu, Duration::ZERO).unwrap();
        assert_eq!(visual.rect, rect(50, 60, 80, 40).to_f64());
        assert_eq!(h.state.focused_window().as_ref(), Some(&window));

        // A move requested by the X11 window is refused while the button is held on another
        // X11 window (the menu), even though both belong to the same Wayland client.
        h.motion(60.0, 70.0);
        h.button(ButtonState::Pressed);
        commands.send(Command::Move).unwrap();
        h.sync(&commands, &window, 1);
        assert!(!h.state.gesture_active());
        h.button(ButtonState::Released);
        // Held on the window itself, it is granted, and ends with the button.
        h.motion(400.0, 300.0);
        h.button(ButtonState::Pressed);
        commands.send(Command::Move).unwrap();
        h.run_until("the gesture requested by X11", State::gesture_active);
        h.button(ButtonState::Released);
        assert!(!h.state.gesture_active());

        // Selections: an X11 client without the keyboard sets neither CLIPBOARD nor PRIMARY
        // on the Wayland side.
        h.state.focus(None);
        commands.send(Command::Own(true)).unwrap();
        commands.send(Command::Own(false)).unwrap();
        h.sync(&commands, &window, 2);
        assert!(current_data_device_selection_userdata(&h.state.seat).is_none());
        assert!(current_primary_selection_userdata(&h.state.seat).is_none());
        // With the keyboard, both are passed on.
        h.state.focus(Some(window.clone()));
        commands.send(Command::Own(true)).unwrap();
        commands.send(Command::Own(false)).unwrap();
        h.sync(&commands, &window, 3);
        assert!(current_data_device_selection_userdata(&h.state.seat).is_some());
        assert!(current_primary_selection_userdata(&h.state.seat).is_some());

        // Super + drag up to the top edge: maximized, for the X server too, its content under
        // its title bar.
        h.motion(400.0, 300.0);
        h.key(KEY_SUPER, KeyState::Pressed);
        h.button(ButtonState::Pressed);
        h.motion(400.0, 200.0);
        h.motion(400.0, 2.0);
        h.button(ButtonState::Released);
        h.key(KEY_SUPER, KeyState::Released);
        let surface = window.x11_surface().unwrap().clone();
        assert!(surface.is_maximized());
        assert_eq!(
            places(&h.state, &window),
            (rect(0, 42, 800, 558), rect(0, 42, 800, 558))
        );

        // Super + down: it returns to its place from before the drag.
        h.key(KEY_SUPER, KeyState::Pressed);
        h.key(KEY_DOWN, KeyState::Pressed);
        h.key(KEY_DOWN, KeyState::Released);
        h.key(KEY_SUPER, KeyState::Released);
        assert!(!surface.is_maximized());
        assert_eq!(
            places(&h.state, &window),
            (rect(300, 271, 200, 100), rect(300, 271, 200, 100))
        );

        // Grab band: the bottom right corner follows the pointer, the top left corner stays.
        h.run_until("the window content", |s| {
            s.window_visual(&window, Duration::ZERO).is_some()
        });
        h.motion(503.0, 374.0);
        h.button(ButtonState::Pressed);
        h.motion(553.0, 404.0);
        h.button(ButtonState::Released);
        assert_eq!(
            places(&h.state, &window),
            (rect(300, 271, 250, 130), rect(300, 271, 250, 130))
        );

        // Minimized by the shell, then withdrawn by its application and mapped again: it is no
        // longer minimized, and no longer hidden for the X server (_NET_WM_STATE_HIDDEN).
        let window = x11_window(&h.state, false).unwrap();
        let surface = window.x11_surface().unwrap().clone();
        h.state.minimize(&window);
        assert!(h.state.is_minimized(&window) && surface.is_minimized());
        assert!(!h.state.is_visible(&window));
        commands.send(Command::Withdraw).unwrap();
        h.run_until("the minimized window withdrawn", |s| {
            !s.is_minimized(&window) && !surface.is_mapped()
        });
        assert!(
            !surface.is_minimized(),
            "the withdrawn window is still hidden"
        );
        commands.send(Command::Map).unwrap();
        h.run_until("the window mapped again", |s| {
            x11_window(s, false).is_some_and(|w| w.wl_surface().is_some())
        });
        assert!(!surface.is_minimized(), "the window mapped again is hidden");

        // The application iconifies its window: minimized like by the shell, hidden for the X
        // server, and the dock brings it back.
        let window = x11_window(&h.state, false).unwrap();
        commands.send(Command::Iconify).unwrap();
        h.run_until("the window iconified by its application", |s| {
            s.is_minimized(&window) && surface.is_minimized()
        });
        assert!(!h.state.is_visible(&window));
        h.state.activate(&window);
        assert!(!h.state.is_minimized(&window) && !surface.is_minimized());
        assert_eq!(h.state.focused_window().as_ref(), Some(&window));

        // A window that asks for no decoration (`_MOTIF_WM_HINTS`) loses its title bar.
        commands.send(Command::Undecorate).unwrap();
        h.run_until("the title bar gone", |_| !decorations::server_side(&window));

        // Closing: the client leaves, its windows go away and nobody has the keyboard.
        commands.send(Command::Quit).unwrap();
        client.join().unwrap();
        h.run_until("the X11 windows closing", |s| {
            s.space.elements().next().is_none() && s.focused_window().is_none()
        });

        // Clean shutdown: XWayland goes away with its socket, and the Wayland socket and its
        // lock go away too, although the X11Wm source keeps the event loop alive.
        h.state.shutdown();
        h.run_until("the socket removed", |_| !Path::new(&socket).exists());
        assert!(!wayland.exists() && !wayland_lock.exists() && !shell.exists());
    }
}
