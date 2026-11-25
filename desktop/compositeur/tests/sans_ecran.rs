//! Le compositeur démarre sans écran ni backend, un client de test se connecte, crée une fenêtre
//! et reçoit ses configurations. Tout tourne dans un seul fil : on alterne la boucle du
//! compositeur et la lecture du client, ce qui permet de regarder l'état des deux côtés.

// Module de test : clippy y autorise unwrap et expect, comme dans les tests unitaires.
#[cfg(test)]
mod tests {
    use std::{os::unix::net::UnixStream, time::Duration};

    use compositeur::State;
    use smithay::{reexports::calloop::EventLoop, utils::Transform};
    use wayland_client::{
        Connection, Dispatch, EventQueue, QueueHandle, delegate_noop,
        protocol::{wl_compositor::WlCompositor, wl_registry, wl_surface::WlSurface},
    };
    use wayland_protocols::xdg::shell::client::{
        xdg_surface::{self, XdgSurface},
        xdg_toplevel::{self, State as XdgState, XdgToplevel},
        xdg_wm_base::{self, XdgWmBase},
    };

    /// Ce que le client a reçu.
    #[derive(Default)]
    struct Client {
        globals: Vec<(u32, String, u32)>,
        /// Chaque configuration de la fenêtre : largeur, hauteur, états.
        configures: Vec<(i32, i32, Vec<XdgState>)>,
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
            state.add_output((800, 600).into(), Transform::Normal);
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
}
