//! Le compositeur démarre sans écran ni backend, lance XWayland, et un client X11 (x11rb) y
//! ouvre une fenêtre gérée et un menu (override-redirect). La boucle du compositeur tourne dans
//! le fil du test, le client X11 dans un autre fil.
//!
//! Il faut `Xwayland` sur le poste, comme pour lancer le compositeur (`docs/compositeur.md`).

// Module de test : clippy y autorise unwrap et expect, comme dans les tests unitaires.
#[cfg(test)]
mod tests {
    use std::{path::Path, sync::mpsc, thread, time::Duration};

    use compositeur::State;
    use smithay::{
        backend::input::{ButtonState, KeyState},
        desktop::Window,
        input::keyboard::Keycode,
        reexports::calloop::EventLoop,
        utils::{Logical, Rectangle, Transform},
        wayland::seat::WaylandFocus,
    };
    use x11rb::{
        connection::Connection,
        protocol::xproto::{
            AtomEnum, ConnectionExt, CreateWindowAux, PropMode, Window as XWindow, WindowClass,
        },
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    const BTN_LEFT: u32 = 0x110;
    // Codes xkb de Super gauche et de la flèche bas. Sans `--imbrique`, la touche des gestes
    // est Super.
    const KEY_SUPER: u32 = 133;
    const KEY_DOWN: u32 = 116;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Ouvre une fenêtre blanche : avec un fond, le serveur la peint, et XWayland envoie un
    /// contenu au compositeur.
    fn open(conn: &RustConnection, root: XWindow, geometry: (i16, i16, u16, u16), menu: bool) {
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
            if menu { b"menu" } else { b"essai x11" },
        )
        .unwrap();
        conn.map_window(window).unwrap();
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        time: u32,
    }

    impl Harness {
        fn run_until(&mut self, what: &str, done: impl Fn(&State) -> bool) {
            for _ in 0..2000 {
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if done(&self.state) {
                    return;
                }
            }
            panic!("délai dépassé en attendant : {what}");
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
    }

    /// Fenêtre X11 gérée ou menu.
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

    /// Place de la fenêtre dans l'espace, et celle que le serveur X lui connaît.
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
    fn une_application_x11_se_gere_comme_une_fenetre_wayland() {
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), Transform::Normal);
        state.start_xwayland(&event_loop, |_, _| {}).unwrap();
        let mut h = Harness {
            event_loop,
            state,
            time: 0,
        };
        h.run_until("XWayland prêt", |s| s.x_display.is_some());
        let display = h.state.x_display.unwrap();
        let socket = format!("/tmp/.X11-unix/X{display}");
        assert!(Path::new(&socket).exists());

        // Le client X11 ouvre une fenêtre de 200 × 100 et un menu en (50, 60), puis attend
        // le signal de partir.
        let (quit, wait) = mpsc::channel::<()>();
        let client = thread::spawn(move || {
            let (conn, screen) = RustConnection::connect(Some(&format!(":{display}"))).unwrap();
            let root = conn.setup().roots[screen].root;
            open(&conn, root, (0, 0, 200, 100), false);
            open(&conn, root, (50, 60, 80, 40), true);
            conn.flush().unwrap();
            wait.recv().unwrap();
        });

        // Affichage et titre : la fenêtre est centrée, et le serveur X connaît sa place.
        h.run_until("la fenêtre X11", |s| x11_window(s, false).is_some());
        let window = x11_window(&h.state, false).unwrap();
        assert_eq!(State::title(&window).as_deref(), Some("essai x11"));
        assert_eq!(
            places(&h.state, &window),
            (rect(300, 250, 200, 100), rect(300, 250, 200, 100))
        );

        // Focus : elle a le clavier dès que XWayland lui a donné sa surface.
        h.run_until("le clavier à la fenêtre X11", |s| {
            s.focused_window().as_ref() == Some(&window)
        });
        assert!(h.state.window_visual(&window, Duration::ZERO).is_some());

        // Le menu est à la place qu'il a choisie, sans prendre le clavier.
        h.run_until("le menu X11", |s| {
            x11_window(s, true).is_some_and(|m| m.wl_surface().is_some())
        });
        let menu = x11_window(&h.state, true).unwrap();
        assert_eq!(
            h.state.space.element_geometry(&menu),
            Some(rect(50, 60, 80, 40))
        );
        let visual = h.state.window_visual(&menu, Duration::ZERO).unwrap();
        assert_eq!(visual.rect, rect(50, 60, 80, 40).to_f64());
        assert_eq!(h.state.focused_window().as_ref(), Some(&window));

        // Super + glisser jusqu'au bord haut : agrandie, pour le serveur X aussi.
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
            (rect(0, 0, 800, 600), rect(0, 0, 800, 600))
        );

        // Super + bas : elle reprend sa place d'avant le glisser.
        h.key(KEY_SUPER, KeyState::Pressed);
        h.key(KEY_DOWN, KeyState::Pressed);
        h.key(KEY_DOWN, KeyState::Released);
        h.key(KEY_SUPER, KeyState::Released);
        assert!(!surface.is_maximized());
        assert_eq!(
            places(&h.state, &window),
            (rect(300, 250, 200, 100), rect(300, 250, 200, 100))
        );

        // Bande de saisie : le coin bas droit suit le pointeur, le coin haut gauche reste.
        h.run_until("le contenu de la fenêtre", |s| {
            s.window_visual(&window, Duration::ZERO).is_some()
        });
        h.motion(503.0, 353.0);
        h.button(ButtonState::Pressed);
        h.motion(553.0, 383.0);
        h.button(ButtonState::Released);
        assert_eq!(
            places(&h.state, &window),
            (rect(300, 250, 250, 130), rect(300, 250, 250, 130))
        );

        // Fermeture : le client part, ses fenêtres disparaissent et personne n'a le clavier.
        quit.send(()).unwrap();
        client.join().unwrap();
        h.run_until("la fermeture des fenêtres X11", |s| {
            s.space.elements().next().is_none() && s.focused_window().is_none()
        });

        // Arrêt propre : XWayland s'en va avec sa socket.
        h.state.stop_xwayland();
        h.run_until("la socket retirée", |_| !Path::new(&socket).exists());
    }
}
