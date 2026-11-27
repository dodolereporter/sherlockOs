//! The screens changed through `wlr-output-management` (ADR 0025, lot D-26b): a client reads
//! the heads and their modes, tests and applies a configuration, and the compositor saves it
//! per screen and takes it again when the screen comes back. Two virtual screens without a
//! host window, which can take several modes and turn.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{os::unix::net::UnixStream, time::Duration};

    use compositor::{
        State,
        displays::{Refused, Setting, Wanted},
    };
    use desktop_config::displays::{Config, Identity};
    use smithay::{
        output::Mode,
        reexports::calloop::EventLoop,
        utils::{Point, Transform},
    };
    use wayland_client::{
        Connection, Dispatch, EventQueue, Proxy, QueueHandle, event_created_child,
        protocol::wl_registry,
    };
    use wayland_protocols_wlr::output_management::v1::client::{
        zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
        zwlr_output_configuration_v1::{self as configuration, ZwlrOutputConfigurationV1},
        zwlr_output_head_v1::{self as head, ZwlrOutputHeadV1},
        zwlr_output_manager_v1::{self as manager, ZwlrOutputManagerV1},
        zwlr_output_mode_v1::{self as mode, ZwlrOutputModeV1},
    };

    /// A head as the client knows it.
    #[derive(Default, Debug)]
    struct Head {
        proxy: Option<ZwlrOutputHeadV1>,
        name: String,
        make: String,
        modes: Vec<(ZwlrOutputModeV1, (i32, i32, i32))>,
        enabled: bool,
        current: Option<(i32, i32)>,
        position: (i32, i32),
        scale: f64,
        finished: bool,
    }

    #[derive(Default)]
    struct Client {
        manager_name: Option<(u32, u32)>,
        heads: Vec<Head>,
        serial: Option<u32>,
        /// `succeeded`, `failed` or `cancelled` of the last configuration.
        answer: Option<&'static str>,
    }

    impl Client {
        fn head(&mut self, proxy: &ZwlrOutputHeadV1) -> &mut Head {
            let i = self
                .heads
                .iter()
                .position(|h| h.proxy.as_ref() == Some(proxy));
            &mut self.heads[i.unwrap()]
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
                && interface == ZwlrOutputManagerV1::interface().name
            {
                client.manager_name = Some((name, version));
            }
        }
    }

    impl Dispatch<ZwlrOutputManagerV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwlrOutputManagerV1,
            event: manager::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            match event {
                manager::Event::Head { head } => client.heads.push(Head {
                    proxy: Some(head),
                    ..Head::default()
                }),
                manager::Event::Done { serial } => client.serial = Some(serial),
                _ => {}
            }
        }

        event_created_child!(Client, ZwlrOutputManagerV1, [
            manager::EVT_HEAD_OPCODE => (ZwlrOutputHeadV1, ()),
        ]);
    }

    impl Dispatch<ZwlrOutputHeadV1, ()> for Client {
        fn event(
            client: &mut Self,
            proxy: &ZwlrOutputHeadV1,
            event: head::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            let head = client.head(proxy);
            match event {
                head::Event::Name { name } => head.name = name,
                head::Event::Make { make } => head.make = make,
                head::Event::Mode { mode } => head.modes.push((mode, (0, 0, 0))),
                head::Event::Enabled { enabled } => head.enabled = enabled != 0,
                head::Event::CurrentMode { mode } => {
                    let size = head.modes.iter().find(|(m, _)| *m == mode).map(|(_, s)| s);
                    head.current = size.map(|s| (s.0, s.1));
                }
                head::Event::Position { x, y } => head.position = (x, y),
                head::Event::Scale { scale } => head.scale = scale,
                head::Event::Finished => head.finished = true,
                _ => {}
            }
        }

        event_created_child!(Client, ZwlrOutputHeadV1, [
            head::EVT_MODE_OPCODE => (ZwlrOutputModeV1, ()),
        ]);
    }

    impl Dispatch<ZwlrOutputModeV1, ()> for Client {
        fn event(
            client: &mut Self,
            proxy: &ZwlrOutputModeV1,
            event: mode::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            for head in &mut client.heads {
                for (m, size) in &mut head.modes {
                    if m != proxy {
                        continue;
                    }
                    match event {
                        mode::Event::Size { width, height } => (size.0, size.1) = (width, height),
                        mode::Event::Refresh { refresh } => size.2 = refresh,
                        _ => {}
                    }
                    return;
                }
            }
        }
    }

    impl Dispatch<ZwlrOutputConfigurationV1, ()> for Client {
        fn event(
            client: &mut Self,
            _: &ZwlrOutputConfigurationV1,
            event: configuration::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            client.answer = Some(match event {
                configuration::Event::Succeeded => "succeeded",
                configuration::Event::Failed => "failed",
                _ => "cancelled",
            });
        }
    }

    impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for Client {
        fn event(
            _: &mut Self,
            _: &ZwlrOutputConfigurationHeadV1,
            _: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        conn: Connection,
        queue: EventQueue<Client>,
        client: Client,
        manager: Option<ZwlrOutputManagerV1>,
    }

    impl Harness {
        /// Two virtual screens of 1440 × 900 and a client bound to the manager.
        fn start() -> Self {
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((1440, 900).into(), 1.0);
            state.add_output((1440, 900).into(), 1.0);
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
                manager: None,
            };
            h.run_until("the global", |h| h.client.manager_name.is_some());
            let (name, version) = h.client.manager_name.unwrap();
            assert_eq!(version, 3);
            h.manager = Some(registry.bind(name, version, &h.queue.handle(), ()));
            h.run_until("the heads", |h| h.client.serial.is_some());
            h
        }

        fn turn(&mut self) {
            self.conn.flush().ok();
            self.event_loop
                .dispatch(Some(Duration::from_millis(5)), &mut self.state)
                .unwrap();
            self.state.refresh();
            self.conn.flush().ok();
            if let Some(guard) = self.queue.prepare_read() {
                let _ = guard.read();
            }
            let _ = self.queue.dispatch_pending(&mut self.client);
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

        /// Sends a configuration: the first head at `mode` of its list, placed at `position`,
        /// the second off or as it is; tested or applied. The answer.
        fn configure(
            &mut self,
            mode: usize,
            position: (i32, i32),
            second_off: bool,
            apply: bool,
        ) -> &'static str {
            let qh = self.queue.handle();
            let serial = self.client.serial.unwrap();
            let config = self
                .manager
                .as_ref()
                .unwrap()
                .create_configuration(serial, &qh, ());
            let first = &self.client.heads[0];
            let head = config.enable_head(first.proxy.as_ref().unwrap(), &qh, ());
            head.set_mode(&first.modes[mode].0);
            head.set_position(position.0, position.1);
            head.set_scale(1.25);
            let second = self.client.heads[1].proxy.as_ref().unwrap();
            if second_off {
                config.disable_head(second);
            } else {
                config.enable_head(second, &qh, ());
            }
            match apply {
                true => config.apply(),
                false => config.test(),
            }
            self.client.answer = None;
            self.run_until("the answer", |h| h.client.answer.is_some());
            config.destroy();
            self.client.answer.unwrap()
        }
    }

    #[test]
    fn the_heads_come_with_their_modes_and_state() {
        let h = Harness::start();
        assert_eq!(h.client.heads.len(), 2);
        let first = &h.client.heads[0];
        assert_eq!(first.name, "virtual-1");
        assert_eq!(first.make, "compositor");
        assert!(first.enabled);
        assert_eq!(first.current, Some((1440, 900)));
        assert_eq!(first.modes[0].1, (1440, 900, 60_000));
        assert!(first.modes.len() >= 4, "{:?}", first.modes);
        assert_eq!((first.position, first.scale), ((0, 0), 1.0));
        assert_eq!(h.client.heads[1].position, (1440, 0));
    }

    #[test]
    fn a_test_changes_nothing_and_an_apply_changes_the_screen() {
        let mut h = Harness::start();
        let i = h.client.heads[0]
            .modes
            .iter()
            .position(|(_, s)| (s.0, s.1) == (1920, 1080))
            .unwrap();
        assert_eq!(h.configure(i, (1440, 0), false, false), "succeeded");
        let first = h.state.outputs()[0].clone();
        assert_eq!(first.current_mode().unwrap().size, (1440, 900).into());

        // The first screen goes to the right of the second, at 1920 × 1080 and 125 %.
        let serial = h.client.serial;
        assert_eq!(h.configure(i, (1440, 0), false, true), "succeeded");
        assert_eq!(first.current_mode().unwrap().size, (1920, 1080).into());
        assert_eq!(first.current_scale().fractional_scale(), 1.25);
        let area = h.state.space.output_geometry(&first).unwrap();
        assert_eq!(area.loc, (1440, 0).into());
        assert_eq!(area.size, (1536, 864).into());
        // The client hears of it, with a new serial.
        h.run_until("the new state", |h| h.client.serial != serial);
        let head = &h.client.heads[0];
        assert_eq!(
            (head.current, head.position),
            (Some((1920, 1080)), (1440, 0))
        );
        assert_eq!(head.scale, 1.25);
    }

    #[test]
    fn an_old_serial_is_cancelled() {
        let mut h = Harness::start();
        h.state.apply_screens(&[]).unwrap();
        let mut stale = h.client.serial;
        h.state.add_output((800, 600).into(), 1.0);
        h.turn();
        std::mem::swap(&mut h.client.serial, &mut stale);
        assert_eq!(h.configure(0, (0, 0), false, true), "cancelled");
    }

    #[test]
    fn a_screen_switched_off_stays_listed_and_comes_back() {
        let mut h = Harness::start();
        assert_eq!(h.configure(0, (0, 0), true, true), "succeeded");
        assert_eq!(h.state.outputs().len(), 1);
        h.run_until("the screen off", |h| !h.client.heads[1].enabled);
        assert!(!h.client.heads[1].finished);
        assert_eq!(h.configure(0, (0, 0), false, true), "succeeded");
        assert_eq!(h.state.outputs().len(), 2);
    }

    #[test]
    fn a_wrong_configuration_is_refused_and_changes_nothing() {
        let mut h = Harness::start();
        let [first, second] = [0, 1].map(|i| h.state.outputs()[i].clone());
        let on = |output: &smithay::output::Output, setting: Setting| Wanted {
            output: output.clone(),
            on: Some(setting),
        };
        let off = |output: &smithay::output::Output| Wanted {
            output: output.clone(),
            on: None,
        };
        let odd = Setting {
            mode: Some(Mode {
                size: (1234, 567).into(),
                refresh: 60_000,
            }),
            ..Setting::default()
        };
        assert!(matches!(
            h.state.apply_screens(&[on(&first, odd)]),
            Err(Refused::Mode(_))
        ));
        assert_eq!(
            h.state.apply_screens(&[off(&first), off(&second)]),
            Err(Refused::AllOff)
        );
        let huge = Setting {
            scale: Some(9.0),
            ..Setting::default()
        };
        assert_eq!(
            h.state.apply_screens(&[on(&first, huge)]),
            Err(Refused::Scale(9.0))
        );
        assert_eq!(first.current_scale().fractional_scale(), 1.0);
        assert_eq!(h.state.outputs().len(), 2);
    }

    #[test]
    fn a_turned_screen_swaps_its_sides() {
        let mut h = Harness::start();
        let first = h.state.outputs()[0].clone();
        let turned = Wanted {
            output: first.clone(),
            on: Some(Setting {
                transform: Some(Transform::_90),
                position: Some(Point::from((0, 0))),
                ..Setting::default()
            }),
        };
        h.state.apply_screens(&[turned]).unwrap();
        let area = h.state.space.output_geometry(&first).unwrap();
        assert_eq!(area.size, (900, 1440).into());
        // The other screen did not move: it was placed by its first configuration.
        let second = h.state.outputs()[1].clone();
        assert_eq!(h.state.space.output_geometry(&second).unwrap().loc.x, 1440);
    }

    #[test]
    fn the_setting_is_saved_per_screen_and_taken_back() {
        let dir = std::env::temp_dir().join(format!("displays-{}", std::process::id()));
        let path = dir.join("displays.conf");
        let mut h = Harness::start();
        h.state.remember_screens(Some(path.clone()));
        let first = h.state.outputs()[0].clone();
        let wanted = Wanted {
            output: first.clone(),
            on: Some(Setting {
                scale: Some(1.5),
                position: Some(Point::from((100, 50))),
                ..Setting::default()
            }),
        };
        h.state.apply_screens(&[wanted]).unwrap();
        h.state.save_screens();
        // Written off the loop.
        let mut saved = None;
        for _ in 0..200 {
            if let Ok((config, _)) = Config::read(&path)
                && !config.screens.is_empty()
            {
                saved = Some(config);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let saved = saved.expect("displays.conf written");
        let identity = Identity::new("compositor", "virtual", "0");
        let screen = saved.get(&identity).unwrap();
        assert_eq!((screen.scale, screen.position), (1.5, (100, 50)));

        // A new session finds it for the same screen.
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.remember_screens(Some(path));
        let output = state.add_output((1440, 900).into(), 1.0);
        assert_eq!(output.current_scale().fractional_scale(), 1.5);
        let area = state.space.output_geometry(&output).unwrap();
        assert_eq!(area.loc, (100, 50).into());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
