//! Le compositeur démarre sans écran ni backend, un client de test se connecte, crée une fenêtre
//! et reçoit ses configurations. Tout tourne dans un seul fil : on alterne la boucle du
//! compositeur et la lecture du client, ce qui permet de regarder l'état des deux côtés.

// Module de test : clippy y autorise unwrap et expect, comme dans les tests unitaires.
#[cfg(test)]
mod tests {
    use std::{
        fs::{File, OpenOptions},
        os::{fd::AsFd, unix::net::UnixStream},
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use compositeur::State;
    use smithay::{
        backend::input::{ButtonState, KeyState},
        input::{keyboard::Keycode, pointer::CursorIcon},
        reexports::calloop::EventLoop,
        utils::{Logical, Point, Rectangle},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop,
        protocol::{
            wl_buffer::WlBuffer,
            wl_callback::{self, WlCallback},
            wl_compositor::WlCompositor,
            wl_keyboard::{self, WlKeyboard},
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
        xdg_toplevel::{self, ResizeEdge, State as XdgState, XdgToplevel},
        xdg_wm_base::{self, XdgWmBase},
    };

    /// Ce que le client a reçu.
    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        /// Chaque configuration de la fenêtre : largeur, hauteur, états.
        configures: Vec<(i32, i32, Vec<XdgState>)>,
        /// Serial du dernier appui de bouton reçu.
        button_serial: Option<u32>,
        /// Touches reçues : code evdev, appuyée ou relâchée.
        keys: Vec<(u32, bool)>,
        /// Le compositeur a répondu au dernier `wl_display.sync`.
        synced: bool,
    }

    impl Dispatch<WlPointer, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WlPointer,
            event: wl_pointer::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_pointer::Event::Button { serial, .. } = event {
                client.button_serial = Some(serial);
            }
        }
    }

    impl Dispatch<WlKeyboard, ()> for Client {
        fn event(
            client: &mut Self,
            _: &WlKeyboard,
            event: wl_keyboard::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_keyboard::Event::Key { key, state, .. } = event {
                let pressed = state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                client.keys.push((key, pressed));
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
            _: &mut Self,
            surface: &XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
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
            if let xdg_toplevel::Event::Configure {
                width,
                height,
                states,
            } = event
            {
                let states = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .filter_map(|b| XdgState::try_from(u32::from_ne_bytes(*b)).ok())
                    .collect();
                client.configures.push((width, height, states));
            }
        }
    }

    delegate_noop!(Client: WlCompositor);
    delegate_noop!(Client: ignore WlSurface);
    delegate_noop!(Client: ignore WlSeat);
    delegate_noop!(Client: ignore WlShm);
    delegate_noop!(Client: WlShmPool);
    delegate_noop!(Client: ignore WlBuffer);

    /// Mémoire partagée pour un tampon de `size` octets, sans fichier qui reste sur le disque.
    /// Les tests tournent en parallèle : chacun a son nom de fichier.
    fn shm_file(size: u64) -> File {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let name = format!("compositeur-test-{}-{n}", std::process::id());
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

    const BTN_LEFT: u32 = 0x110;
    const BTN_RIGHT: u32 = 0x111;
    // Codes xkb (evdev + 8) de Super gauche et des flèches. Sans `--imbrique`, la touche des
    // gestes est Super.
    const KEY_SUPER: u32 = 133;
    const KEY_UP: u32 = 111;
    const KEY_LEFT: u32 = 113;
    const KEY_DOWN: u32 = 116;
    const KEY_SHIFT: u32 = 50;

    /// Fenêtre affichée par `Harness::show_window` : 100 × 80, centrée en (350, 260) dans l'écran
    /// de 800 × 600. Le fichier du tampon vit aussi longtemps qu'elle.
    struct Shown {
        seat: WlSeat,
        toplevel: XdgToplevel,
        surface: WlSurface,
        shm: WlShm,
        _file: File,
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
    }

    impl Harness {
        fn start() -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            let (server_side, client_side) = UnixStream::pair().unwrap();
            state.insert_client(server_side);
            let conn = Connection::from_socket(client_side).unwrap();
            let queue = conn.new_event_queue();
            Self {
                event_loop,
                state,
                conn,
                queue,
                client: Client::default(),
            }
        }

        /// Fait tourner compositeur et client jusqu'à ce que `done` soit vrai.
        fn run_until(&mut self, what: &str, done: impl Fn(&Client, &State) -> bool) {
            for _ in 0..200 {
                self.conn.flush().unwrap();
                self.event_loop
                    .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                    .unwrap();
                self.state.refresh();
                if let Some(guard) = self.queue.prepare_read() {
                    // Rien à lire n'est pas une erreur : on retente au tour suivant.
                    let _ = guard.read();
                }
                self.queue.dispatch_pending(&mut self.client).unwrap();
                if done(&self.client, &self.state) {
                    return;
                }
            }
            panic!("délai dépassé en attendant : {what}");
        }

        fn bind<I: wayland_client::Proxy + 'static>(&self, registry: &wl_registry::WlRegistry) -> I
        where
            Client: Dispatch<I, ()>,
        {
            let name = I::interface().name;
            let (id, _, version) = self.client.globals.iter().find(|g| g.1 == name).unwrap();
            registry.bind(
                *id,
                (*version).min(I::interface().version),
                &self.queue.handle(),
                (),
            )
        }

        fn last_configure(&self) -> &(i32, i32, Vec<XdgState>) {
            self.client.configures.last().unwrap()
        }

        fn show_window(&mut self) -> Shown {
            self.show_window_with(false)
        }

        /// Un client crée une fenêtre, l'affiche avec un tampon de 100 × 80 et écoute le pointeur
        /// et le clavier. `maximized` : il demande l'agrandissement avant son premier contenu.
        fn show_window_with(&mut self, maximized: bool) -> Shown {
            let qh = self.queue.handle();
            let registry = self.conn.display().get_registry(&qh, ());
            self.run_until("les globales", |c, _| {
                c.globals.iter().any(|g| g.1 == "wl_shm")
            });
            let compositor: WlCompositor = self.bind(&registry);
            let wm_base: XdgWmBase = self.bind(&registry);
            let shm: WlShm = self.bind(&registry);
            let seat: WlSeat = self.bind(&registry);
            let _pointer = seat.get_pointer(&qh, ());
            let _keyboard = seat.get_keyboard(&qh, ());
            let surface = compositor.create_surface(&qh, ());
            let toplevel = wm_base
                .get_xdg_surface(&surface, &qh, ())
                .get_toplevel(&qh, ());
            if maximized {
                toplevel.set_maximized();
            }
            surface.commit();
            self.run_until("la première configuration", |c, _| {
                !c.configures.is_empty()
            });
            let file = shm_file(100 * 80 * 4);
            let pool = shm.create_pool(file.as_fd(), 100 * 80 * 4, &qh, ());
            let buffer = pool.create_buffer(0, 100, 80, 400, Format::Argb8888, &qh, ());
            surface.attach(Some(&buffer), 0, 0);
            surface.commit();
            let placed = if maximized { (0, 0) } else { (350, 260) };
            self.run_until("le placement", |_, s| location(s) == Some(placed.into()));
            Shown {
                seat,
                toplevel,
                surface,
                shm,
                _file: file,
            }
        }

        /// Le client redessine sa fenêtre à `w` × `h` et l'envoie. Le fichier du tampon doit
        /// vivre aussi longtemps que lui.
        fn redraw(&mut self, shown: &Shown, w: i32, h: i32) -> File {
            let qh = self.queue.handle();
            let file = shm_file((w * h * 4) as u64);
            let pool = shown.shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
            let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, &qh, ());
            shown.surface.attach(Some(&buffer), 0, 0);
            shown.surface.commit();
            self.run_until("le nouveau contenu", |_, s| {
                s.space
                    .elements()
                    .next()
                    .is_some_and(|w_| w_.geometry().size == (w, h).into())
            });
            file
        }

        /// Ce qui est dessiné de la fenêtre à `t` ms, et sa géométrie réelle.
        fn drawn(&self, t: u64) -> (Rectangle<f64, Logical>, Rectangle<f64, Logical>) {
            let window = self.state.space.elements().next().unwrap();
            let visual = self
                .state
                .window_visual(window, Duration::from_millis(t))
                .unwrap();
            let real = self.state.space.element_geometry(window).unwrap();
            (visual.rect, real.to_f64())
        }

        /// Dessine la fenêtre à `t` ms et vérifie qu'elle l'est exactement à sa place.
        fn assert_follows(&self, t: u64, what: &str) {
            let (drawn, real) = self.drawn(t);
            assert_eq!(drawn, real, "{what} : dessinée ailleurs que sa géométrie");
        }

        /// Appui du bouton gauche en (`x`, `y`), sur une surface du client : rend son serial.
        fn press(&mut self, x: f64, y: f64) -> u32 {
            self.client.button_serial = None;
            self.state.pointer_motion((x, y).into(), 1);
            self.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
            self.run_until("l'appui", |c, _| c.button_serial.is_some());
            self.client.button_serial.unwrap()
        }

        fn release(&mut self) {
            self.state
                .pointer_button(BTN_LEFT, ButtonState::Released, 3);
        }

        /// Attend une configuration de taille `w` × `h` et rend ses états.
        fn wait_size(&mut self, what: &str, w: i32, h: i32) -> Vec<XdgState> {
            self.run_until(what, |c, _| {
                c.configures.last().is_some_and(|c| (c.0, c.1) == (w, h))
            });
            self.last_configure().2.clone()
        }

        /// Touche des gestes et une touche, appuyées puis relâchées.
        fn shortcut(&mut self, key: u32) {
            self.chord(&[KEY_SUPER], key);
        }

        /// Les touches `mods` tenues pendant l'appui et le relâchement de `key`.
        fn chord(&mut self, mods: &[u32], key: u32) {
            let press = mods.iter().chain([&key]).map(|k| (*k, KeyState::Pressed));
            let release = [&key]
                .into_iter()
                .chain(mods)
                .map(|k| (*k, KeyState::Released));
            for (code, state) in press.chain(release).collect::<Vec<_>>() {
                self.state.keyboard_key(Keycode::new(code), state, 4);
            }
        }
    }

    fn location(s: &State) -> Option<Point<i32, Logical>> {
        let window = s.space.elements().next()?;
        s.space.element_location(window)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn un_client_cree_une_fenetre_et_recoit_sa_configuration() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let registry = h.conn.display().get_registry(&qh, ());
        h.run_until("les globales", |c, _| {
            c.globals.iter().any(|g| g.1 == "xdg_wm_base")
        });

        let compositor: WlCompositor = h.bind(&registry);
        let wm_base: XdgWmBase = h.bind(&registry);
        let surface = compositor.create_surface(&qh, ());
        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        toplevel.set_title("essai".into());
        surface.commit();

        h.run_until("la première configuration", |c, _| {
            !c.configures.is_empty()
        });
        assert_eq!(
            h.last_configure(),
            &(0, 0, vec![]),
            "le client choisit sa taille"
        );
        let windows: Vec<_> = h.state.space.elements().collect();
        assert_eq!(windows.len(), 1);
        assert_eq!(State::title(windows[0]).as_deref(), Some("essai"));

        toplevel.set_maximized();
        h.run_until("l'agrandissement", |c, _| c.configures.len() >= 2);
        let (w, h_, states) = h.last_configure().clone();
        assert_eq!(
            (w, h_),
            (800, 600),
            "agrandie, la fenêtre prend tout l'écran"
        );
        assert!(states.contains(&XdgState::Maximized));

        toplevel.set_fullscreen(None);
        h.run_until("le plein écran", |c, _| c.configures.len() >= 3);
        assert!(h.last_configure().2.contains(&XdgState::Fullscreen));

        toplevel.unset_fullscreen();
        toplevel.unset_maximized();
        h.run_until("le retour à la normale", |c, _| {
            c.configures.last().is_some_and(|(_, _, s)| {
                !s.contains(&XdgState::Maximized) && !s.contains(&XdgState::Fullscreen)
            })
        });

        toplevel.destroy();
        h.run_until("la fermeture de la fenêtre", |_, s| {
            s.space.elements().count() == 0
        });
    }

    #[test]
    fn un_client_deplace_sa_fenetre_depuis_sa_barre_de_titre() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let Shown { seat, toplevel, .. } = h.show_window();

        // Appui sur la barre de titre, que le client dessine lui-même.
        let serial = h.press(360.0, 270.0);

        // Un serial qui ne vient pas de cet appui est refusé.
        toplevel._move(&seat, serial + 1000);
        h.conn.display().sync(&qh, ());
        h.run_until("la réponse au sync", |c, _| c.synced);
        assert!(!h.state.gesture_active());

        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Move));
        h.state.pointer_motion((410.0, 300.0).into(), 3);
        assert_eq!(
            location(&h.state),
            Some((400, 290).into()),
            "la fenêtre suit le pointeur"
        );
        assert_eq!(h.state.snap_preview(), None, "loin des bords, rien à caler");
        h.release();
        assert!(!h.state.gesture_active());
        h.state.pointer_motion((500.0, 400.0).into(), 5);
        assert_eq!(
            location(&h.state),
            Some((400, 290).into()),
            "le geste est fini"
        );
    }

    #[test]
    fn une_fenetre_lachee_en_haut_est_agrandie_puis_rendue_en_la_deplacant() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();

        let serial = h.press(360.0, 270.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        h.state.pointer_motion((400.0, 2.0).into(), 3);
        assert_eq!(
            h.state.snap_preview(),
            Some(rect(0, 0, 800, 600)),
            "l'aperçu montre tout l'écran"
        );
        h.release();
        let states = h.wait_size("l'agrandissement", 800, 600);
        assert!(states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((0, 0).into()));

        // Déplacée, elle quitte l'agrandissement et reprend sa taille d'avant.
        let serial = h.press(50.0, 10.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        // Sans mouvement, rien ne change : un simple clic ne détache pas la fenêtre.
        h.state.pointer_motion((51.0, 11.0).into(), 3);
        assert_eq!(h.last_configure().0, 800);
        h.state.pointer_motion((300.0, 300.0).into(), 3);
        let states = h.wait_size("la taille d'avant", 100, 80);
        assert!(!states.contains(&XdgState::Maximized));
        h.release();
        assert!(!h.state.gesture_active());
    }

    #[test]
    fn une_fenetre_lachee_dans_un_coin_ou_sur_un_cote_se_cale() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();

        let serial = h.press(360.0, 270.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        h.state.pointer_motion((795.0, 590.0).into(), 3);
        assert_eq!(h.state.snap_preview(), Some(rect(400, 300, 400, 300)));
        h.release();
        let states = h.wait_size("le quart bas droit", 400, 300);
        assert!(!states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((400, 300).into()));

        // Le contenu reste de 100 × 80 tant que le client ne l'a pas redessiné.
        let serial = h.press(410.0, 310.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        h.state.pointer_motion((1.0, 300.0).into(), 3);
        assert_eq!(h.state.snap_preview(), Some(rect(0, 0, 400, 600)));
        h.release();
        h.wait_size("la moitié gauche", 400, 600);

        // Rendue, elle retrouve sa place d'avant le glisser, pas celle du lâcher.
        h.shortcut(KEY_DOWN);
        h.wait_size("la taille d'avant", 100, 80);
        assert_eq!(location(&h.state), Some((400, 300).into()));
    }

    #[test]
    fn la_bande_autour_de_la_fenetre_la_redimensionne() {
        let mut h = Harness::start();
        let _shown = h.show_window();

        // 5 pixels à droite de la fenêtre, qui va de x = 350 à x = 450.
        h.state.pointer_motion((455.0, 300.0).into(), 1);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::EResize));
        h.state.pointer_motion((455.0, 345.0).into(), 1);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::SeResize));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        assert!(h.state.gesture_active());
        h.state.pointer_motion((485.0, 365.0).into(), 3);
        let states = h.wait_size("le redimensionnement", 130, 100);
        assert!(states.contains(&XdgState::Resizing));
        // Un autre bouton ne termine pas le geste : seul le relâchement du gauche le fait.
        h.state.pointer_button(BTN_RIGHT, ButtonState::Pressed, 3);
        h.state.pointer_button(BTN_RIGHT, ButtonState::Released, 3);
        assert!(h.state.gesture_active());
        h.release();
        assert!(!h.state.gesture_active());
        h.run_until("la fin du redimensionnement", |c, _| {
            c.configures
                .last()
                .is_some_and(|c| !c.2.contains(&XdgState::Resizing))
        });
    }

    #[test]
    fn un_client_redimensionne_sa_fenetre_par_un_bord() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();

        // Dans le coin bas droit, à l'intérieur : là où le client dessine son bord.
        let serial = h.press(448.0, 338.0);
        toplevel.resize(&seat, serial, ResizeEdge::BottomRight);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::SeResize));
        h.state.pointer_motion((478.0, 358.0).into(), 3);
        let states = h.wait_size("le redimensionnement", 130, 100);
        assert!(states.contains(&XdgState::Resizing));
        h.release();
        assert!(!h.state.gesture_active());
    }

    #[test]
    fn une_fenetre_en_plein_ecran_ne_bouge_pas() {
        let mut h = Harness::start();
        let qh = h.queue.handle();
        let Shown { seat, toplevel, .. } = h.show_window();
        toplevel.set_fullscreen(None);
        let states = h.wait_size("le plein écran", 800, 600);
        assert!(states.contains(&XdgState::Fullscreen));

        // Le contenu reste de 100 × 80, posé en (0, 0).
        let serial = h.press(50.0, 40.0);
        toplevel._move(&seat, serial);
        toplevel.resize(&seat, serial, ResizeEdge::BottomRight);
        h.conn.display().sync(&qh, ());
        h.run_until("la réponse au sync", |c, _| c.synced);
        assert!(!h.state.gesture_active());
        // Pas de bande de saisie non plus.
        h.release();
        h.state.pointer_motion((103.0, 40.0).into(), 3);
        assert_eq!(h.state.cursor_icon(), Some(CursorIcon::Default));
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 4);
        assert!(!h.state.gesture_active());
    }

    #[test]
    fn la_touche_des_gestes_et_les_fleches_calent_la_fenetre() {
        let mut h = Harness::start();
        let _shown = h.show_window();

        h.shortcut(KEY_LEFT);
        let states = h.wait_size("la moitié gauche", 400, 600);
        assert!(!states.contains(&XdgState::Maximized));
        h.shortcut(KEY_UP);
        let states = h.wait_size("l'agrandissement", 800, 600);
        assert!(states.contains(&XdgState::Maximized));
        h.shortcut(KEY_DOWN);
        let states = h.wait_size("la taille d'avant", 100, 80);
        assert!(!states.contains(&XdgState::Maximized));
        assert_eq!(location(&h.state), Some((350, 260).into()));
        h.shortcut(KEY_UP);
        h.wait_size("l'agrandissement", 800, 600);
        h.shortcut(KEY_UP);
        h.wait_size("le retour", 100, 80);

        // Le client voit Super, jamais les flèches interceptées, ni leur appui ni leur relâchement.
        h.conn.display().sync(&h.queue.handle(), ());
        h.run_until("la réponse au sync", |c, _| c.synced);
        let evdev = |xkb: u32| xkb - 8;
        assert!(h.client.keys.contains(&(evdev(KEY_SUPER), true)));
        let arrows = [KEY_UP, KEY_LEFT, KEY_DOWN].map(evdev);
        assert!(!h.client.keys.iter().any(|(k, _)| arrows.contains(k)));

        // Avec une autre touche en plus de celle des gestes, la flèche va au client.
        h.chord(&[KEY_SUPER, KEY_SHIFT], KEY_LEFT);
        h.run_until("la flèche", |c, _| {
            c.keys.contains(&(evdev(KEY_LEFT), false))
        });
        assert!(h.client.keys.contains(&(evdev(KEY_LEFT), true)));
        assert_eq!(h.last_configure().0, 100, "la fenêtre n'est pas calée");
    }

    #[test]
    fn une_fenetre_agrandie_avant_son_contenu_se_deplace_a_la_taille_du_client() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window_with(true);
        let states = h.wait_size("l'agrandissement", 800, 600);
        assert!(states.contains(&XdgState::Maximized));

        // Sa place d'avant est vide : le client choisit sa taille, 0 × 0.
        let serial = h.press(50.0, 10.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        h.state.pointer_motion((300.0, 300.0).into(), 3);
        let states = h.wait_size("la taille du client", 0, 0);
        assert!(!states.contains(&XdgState::Maximized));

        // Calée puis rendue, elle ne garde pas la taille de l'écran comme taille d'avant.
        h.state.pointer_motion((1.0, 300.0).into(), 4);
        h.release();
        h.wait_size("la moitié gauche", 400, 600);
        h.shortcut(KEY_DOWN);
        let states = h.wait_size("la taille du client", 0, 0);
        assert!(!states.contains(&XdgState::Maximized));
    }

    #[test]
    fn un_calage_interrompu_ne_rend_pas_le_glisser_suivant_anime() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();
        h.drawn(0);
        h.assert_follows(1000, "ouverte");

        // Agrandie, puis rendue avant la fin de l'animation.
        h.shortcut(KEY_UP);
        let (moving, real) = h.drawn(1010);
        assert_ne!(moving, real, "le calage est animé");
        h.shortcut(KEY_DOWN);
        h.drawn(1050);
        h.assert_follows(3000, "rendue");

        // Le glisser suivant suit le pointeur, à chaque image.
        let serial = h.press(360.0, 270.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        h.state.pointer_motion((330.0, 280.0).into(), 3);
        h.assert_follows(3010, "glissée");
        h.state.pointer_motion((300.0, 290.0).into(), 3);
        h.assert_follows(3020, "glissée plus loin");
        h.release();
    }

    #[test]
    fn un_calage_sans_effet_ne_rend_pas_le_redimensionnement_suivant_anime() {
        let mut h = Harness::start();
        let shown = h.show_window();
        h.drawn(0);
        h.shortcut(KEY_LEFT);
        h.wait_size("la moitié gauche", 400, 600);
        h.drawn(1000);
        h.assert_follows(3000, "calée");

        // Déjà à gauche : le calage ne change rien.
        h.shortcut(KEY_LEFT);
        h.assert_follows(3010, "calée de nouveau");

        // Redimensionnée par la bande de droite, elle est dessinée à sa nouvelle taille.
        h.state.pointer_motion((104.0, 40.0).into(), 1);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        assert!(h.state.gesture_active());
        h.state.pointer_motion((134.0, 40.0).into(), 3);
        h.wait_size("le redimensionnement", 130, 80);
        let _file = h.redraw(&shown, 130, 80);
        h.assert_follows(3020, "redimensionnée");
        assert_eq!(
            h.drawn(3030).0,
            Rectangle::new((0.0, 0.0).into(), (130.0, 80.0).into())
        );
        h.release();
    }

    #[test]
    fn une_fenetre_calee_detachee_suit_le_pointeur() {
        let mut h = Harness::start();
        let Shown { seat, toplevel, .. } = h.show_window();
        h.drawn(0);
        h.shortcut(KEY_LEFT);
        h.wait_size("la moitié gauche", 400, 600);
        h.drawn(1000);
        h.assert_follows(3000, "calée");

        // Détachée en la glissant, puis glissée vite : elle reste sous le pointeur.
        let serial = h.press(10.0, 10.0);
        toplevel._move(&seat, serial);
        h.run_until("le début du geste", |_, s| s.gesture_active());
        for (i, x) in [100.0, 200.0, 300.0, 400.0].into_iter().enumerate() {
            h.state.pointer_motion((x, 300.0).into(), 3);
            h.assert_follows(3001 + i as u64, "détachée");
        }
        h.release();
    }
}
