//! The notification area against fake applications that put an item there with
//! `StatusNotifierItem`, on a private bus started by the test (its own `dbus-daemon`, never the
//! user's bus): the item appears in the popup, a click reaches it as `Activate`, the wheel as
//! `Scroll`, a change of status is heard, and it disappears when it leaves the bus. Then the bar
//! as a host of another watcher, and the captures of the open popup.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use bar::app::{Bar, Click, Message};
    use bar::source::battery::{Battery, Charge};
    use bar::source::clock::LocalTime;
    use bar::source::network::{Link, Network};
    use bar::source::sound::Sound;
    use bar::source::workspaces::Desktop;
    use bar::tray::{self, Event, Icon, Request, watcher};
    use iced::futures::StreamExt;
    use iced::window::Id;
    use iced_test::simulator::Simulator;
    use sherlock_ui::color::{self, Theme};
    use sherlock_ui::{metrics, theme};
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::OwnedObjectPath;

    /// A capture sets the active theme: one at a time.
    static THEME: Mutex<()> = Mutex::new(());

    /// A private bus: its own `dbus-daemon`, socket in a folder of its own, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
    }

    impl Bus {
        fn start(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("bar-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let config = dir.join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    "<busconfig><type>session</type><listen>unix:dir={}</listen>\
                     <auth>EXTERNAL</auth><policy context=\"default\">\
                     <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                     <allow own=\"*\"/></policy></busconfig>",
                    dir.display()
                ),
            )
            .unwrap();
            let mut daemon = Command::new("dbus-daemon")
                .arg(format!("--config-file={}", config.display()))
                .args(["--nofork", "--print-address"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("dbus-daemon (package dbus-daemon) is needed by these tests");
            let mut address = String::new();
            let stdout = daemon.stdout.take().unwrap();
            BufReader::new(stdout).read_line(&mut address).unwrap();
            Self {
                daemon,
                address: address.trim().to_owned(),
                dir,
            }
        }

        /// A connection with the bar's time limit on every call.
        fn connect(&self) -> zbus::Connection {
            zbus::block_on(async {
                zbus::connection::Builder::address(self.address.as_str())
                    .unwrap()
                    .method_timeout(bar::dbusmenu::TIMEOUT)
                    .build()
                    .await
                    .unwrap()
            })
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    type Pixels = Vec<(i32, i32, Vec<u8>)>;

    /// What an application was asked.
    type Seen = Arc<Mutex<Vec<String>>>;

    /// An item, as libappindicator exports one.
    struct FakeItem {
        seen: Seen,
        status: Arc<Mutex<String>>,
    }

    /// A 2 × 2 pixmap: opaque green.
    fn green() -> Pixels {
        vec![(2, 2, [0xFF, 0x85, 0xC5, 0x95].repeat(4))]
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl FakeItem {
        #[zbus(property)]
        fn id(&self) -> String {
            String::from("fake-chat")
        }
        #[zbus(property)]
        fn title(&self) -> String {
            String::from("Messagerie")
        }
        #[zbus(property)]
        fn status(&self) -> String {
            self.status.lock().unwrap().clone()
        }
        #[zbus(property)]
        fn icon_name(&self) -> String {
            String::from("fake-chat-not-installed")
        }
        #[zbus(property)]
        fn icon_pixmap(&self) -> Pixels {
            green()
        }
        #[zbus(property)]
        fn tool_tip(&self) -> (String, Pixels, String, String) {
            (
                String::new(),
                Vec::new(),
                String::from("Messagerie"),
                String::from("<b>2</b> messages"),
            )
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn menu(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from("/MenuBar").unwrap()
        }

        fn activate(&self, x: i32, y: i32) {
            self.seen.lock().unwrap().push(format!("Activate {x} {y}"));
        }
        fn secondary_activate(&self, _x: i32, _y: i32) {
            self.seen
                .lock()
                .unwrap()
                .push(String::from("SecondaryActivate"));
        }
        fn scroll(&self, delta: i32, orientation: &str) {
            self.seen
                .lock()
                .unwrap()
                .push(format!("Scroll {delta} {orientation}"));
        }

        #[zbus(signal)]
        async fn new_status(emitter: &SignalEmitter<'_>, status: &str) -> zbus::Result<()>;
    }

    /// The menu of the fake item, as libdbusmenu exports one.
    struct FakeMenu {
        seen: Seen,
    }

    fn entry(id: i32, label: &str) -> bar::dbusmenu::Node {
        let properties = [(
            String::from("label"),
            zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from(label)).unwrap(),
        )];
        (id, properties.into_iter().collect(), Vec::new())
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl FakeMenu {
        fn get_layout(
            &self,
            _parent: i32,
            _depth: i32,
            _names: Vec<String>,
        ) -> (u32, bar::dbusmenu::Node) {
            let children = [entry(1, "Ouvrir"), entry(2, "Quitter")]
                .into_iter()
                .map(|c| {
                    zbus::zvariant::OwnedValue::try_from(zbus::zvariant::Value::from(c)).unwrap()
                })
                .collect();
            (3, (0, Default::default(), children))
        }
        fn about_to_show(&self, _id: i32) -> bool {
            false
        }
        fn event(&self, id: i32, event: String, _data: zbus::zvariant::OwnedValue, _time: u32) {
            self.seen
                .lock()
                .unwrap()
                .push(format!("Event {id} {event}"));
        }
    }

    /// The fake application: its item at `/StatusNotifierItem` under a name of its own,
    /// registered with the watcher once there is one.
    struct Application {
        bus: zbus::Connection,
        name: String,
        seen: Seen,
        status: Arc<Mutex<String>>,
    }

    fn application(bus: &Bus, number: u32) -> Application {
        let connection = bus.connect();
        let seen = Seen::default();
        let status = Arc::new(Mutex::new(String::from("Active")));
        let item = FakeItem {
            seen: Arc::clone(&seen),
            status: Arc::clone(&status),
        };
        zbus::block_on(connection.object_server().at(tray::ITEM_PATH, item)).unwrap();
        let menu = FakeMenu {
            seen: Arc::clone(&seen),
        };
        zbus::block_on(connection.object_server().at("/MenuBar", menu)).unwrap();
        let name = format!("org.kde.StatusNotifierItem-{}-{number}", std::process::id());
        zbus::block_on(connection.request_name(name.as_str())).unwrap();
        // The watcher may come a little later than the application, as at login.
        let registered = (0..150).any(|_| {
            let called = zbus::block_on(connection.call_method(
                Some(watcher::NAME),
                watcher::PATH,
                Some(watcher::NAME),
                "RegisterStatusNotifierItem",
                &name,
            ));
            called.is_ok() || {
                std::thread::sleep(Duration::from_millis(100));
                false
            }
        });
        assert!(registered, "the watcher answers");
        Application {
            bus: connection,
            name,
            seen,
            status,
        }
    }

    /// The bar's host on `connection`, its events on a channel.
    fn host(connection: zbus::Connection) -> Receiver<Event> {
        let (sender, received) = mpsc::channel();
        std::thread::spawn(move || {
            let mut events = zbus::block_on(tray::host::follow(connection));
            while let Some(event) = zbus::block_on(events.next()) {
                if sender.send(event).is_err() {
                    return;
                }
            }
        });
        received
    }

    /// The next event, within 15 s: a loaded machine reads slowly, a lost event never comes.
    #[track_caller]
    fn next(events: &Receiver<Event>) -> Event {
        events
            .recv_timeout(Duration::from_secs(15))
            .expect("an event of the notification area within 15 s")
    }

    #[test]
    fn an_item_appears_in_the_popup_is_activated_and_leaves() {
        let bus = Bus::start("tray-item");
        let events = host(bus.connect());
        let app = application(&bus, 1);
        let Event::Changed(item) = next(&events) else {
            panic!("the item first")
        };
        assert_eq!(item.service, app.name);
        assert_eq!(item.properties.id, "fake-chat");
        assert_eq!(
            item.properties.label(),
            ("Messagerie".into(), "2 messages".into())
        );
        // No icon of that name on the machine: the pixels.
        assert!(matches!(item.icon, Some(Icon::Pixels(_))));
        let key = item.key.clone();

        // In the bar: the chevron, then the popup with the item in it.
        let mut state = Bar::default();
        let surface = Id::unique();
        let _ = state.update(Message::Surface(surface));
        let _ = state.update(Message::Resized(surface, 1896.0));
        let _ = state.update(Message::Tray(Event::Changed(item)));
        assert!(state.targets().contains(&bar::app::Target::Tray));
        let _ = state.update(Message::ToggleTray);
        assert!(state.tray_open());
        let (width, height) = state.tray.size;
        let mut ui = Simulator::with_size(
            bar::settings(),
            (width as f32, height as f32),
            state.view_tray(),
        );
        // The first cell, at the top left of the glass.
        let inset = sherlock_ui::tray::PADDING + metrics::BORDER_WIDTH;
        let cell = sherlock_ui::tray::CELL / 2.0;
        let margins = state.tray.margins;
        let at = iced::Point::new(margins.left + inset + cell, margins.top + inset + cell);
        ui.point_at(at);
        ui.simulate(iced_test::simulator::click());
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::TrayClick(k, Click::Primary) if *k == key)),
            "{messages:?}"
        );
        // What the click sends reaches the application, and the popup closes.
        let (service, path, request) = state.tray_request(&key, Click::Primary).unwrap();
        assert!(matches!(request, Request::Activate(..)));
        let _ = state.update(Message::TrayClick(key.clone(), Click::Primary));
        assert!(!state.tray_open());
        let client = bus.connect();
        zbus::block_on(request.send(&client, &service, &path)).unwrap();
        let wheel = iced::mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 };
        let scroll = Request::scroll(wheel).unwrap();
        zbus::block_on(scroll.send(&client, &service, &path)).unwrap();
        assert_eq!(app.seen.lock().unwrap()[1], "Scroll -120 vertical");
        assert!(app.seen.lock().unwrap()[0].starts_with("Activate "));

        // A right click opens its menu, read from the application, in the style of ours; a
        // click on « Quitter » reaches it.
        let _ = state.update(Message::ToggleTray);
        let _ = state.update(Message::TrayClick(key.clone(), Click::Context));
        let address = state.global.item.as_ref().unwrap().address.clone();
        assert_eq!(
            (address.service.as_str(), address.path.as_str()),
            (app.name.as_str(), "/MenuBar")
        );
        let root = zbus::block_on(bar::dbusmenu::layout(&client, &address, 0)).unwrap();
        let _ = state.update(Message::AppMenuRead(address.clone(), 0, Ok(root)));
        assert!(state.app_menu_open());
        let (menu, id) = state.picked(0, 1).unwrap();
        let _ = state.update(Message::PickEntry(0, 1));
        assert!(!state.app_menu_open() && !state.tray_open());
        zbus::block_on(bar::dbusmenu::clicked(&client, &menu, id)).unwrap();
        assert_eq!(
            app.seen.lock().unwrap().last().map(String::as_str),
            Some("Event 2 clicked")
        );

        // It needs attention: heard, read again, and the chevron carries the dot.
        *app.status.lock().unwrap() = String::from("NeedsAttention");
        let emitter = SignalEmitter::new(&app.bus, tray::ITEM_PATH).unwrap();
        zbus::block_on(FakeItem::new_status(&emitter, "NeedsAttention")).unwrap();
        let Event::Changed(changed) = next(&events) else {
            panic!("read again")
        };
        assert!(changed.needs_attention());
        let _ = state.update(Message::Tray(Event::Changed(changed)));
        assert!(state.tray.attention());

        // The application leaves the bus: its item goes.
        drop(emitter);
        zbus::block_on(app.bus.close()).unwrap();
        let Event::Gone(gone) = next(&events) else {
            panic!("gone")
        };
        assert_eq!(gone, key);
        let _ = state.update(Message::Tray(Event::Gone(gone)));
        assert_eq!(state.tray.shown().count(), 0);
        assert!(!state.targets().contains(&bar::app::Target::Tray));
    }

    /// Plasma's watcher owns the name: the bar registers there as a host, and gets its items.
    #[test]
    fn with_another_watcher_the_bar_is_a_host() {
        let bus = Bus::start("tray-host");
        let other = bus.connect();
        assert!(zbus::block_on(watcher::serve(&other)).unwrap());
        let events = host(bus.connect());
        let app = application(&bus, 2);
        let Event::Changed(item) = next(&events) else {
            panic!("the item")
        };
        assert_eq!(item.service, app.name);
        // The other watcher knows the bar as a host.
        let registered: bool = zbus::block_on(async {
            let properties = zbus::fdo::PropertiesProxy::builder(&other)
                .destination(watcher::NAME)
                .unwrap()
                .path(watcher::PATH)
                .unwrap()
                .build()
                .await
                .unwrap();
            let value = properties
                .get(
                    zbus::names::InterfaceName::from_static_str_unchecked(watcher::NAME),
                    "IsStatusNotifierHostRegistered",
                )
                .await
                .unwrap();
            bool::try_from(value).unwrap()
        });
        assert!(registered);
    }

    /// A line icon of ours as a symbolic icon of a theme would be: one color, `#bebebe`.
    fn symbolic(icon: sherlock_ui::icon::Icon) -> Icon {
        let document = icon.svg().replace("stroke=\"#000\"", "stroke=\"#bebebe\"");
        Icon::Drawing {
            handle: iced::widget::svg::Handle::from_memory(document.into_bytes()),
            symbolic: true,
        }
    }

    /// An application icon of ours, in full color.
    fn drawing(icon: sherlock_ui::app_icon::AppIcon) -> Icon {
        Icon::Drawing {
            handle: iced::widget::svg::Handle::from_memory(icon.svg().as_bytes()),
            symbolic: false,
        }
    }

    /// A round badge of pixels, as an application sends one: `side` pixels, ARGB.
    fn badge(side: i32, argb: [u8; 4]) -> Icon {
        let mut pixels = Vec::new();
        let middle = (side as f32 - 1.0) / 2.0;
        for y in 0..side {
            for x in 0..side {
                let distance = ((x as f32 - middle).powi(2) + (y as f32 - middle).powi(2)).sqrt();
                let inside = (middle + 0.5 - distance).clamp(0.0, 1.0);
                let alpha = (f32::from(argb[0]) * inside) as u8;
                pixels.extend([alpha, argb[1], argb[2], argb[3]]);
            }
        }
        let pixmap = tray::Pixmap {
            width: side,
            height: side,
            argb: pixels,
        };
        Icon::Pixels(iced::widget::image::Handle::from_rgba(
            side as u32,
            side as u32,
            pixmap.rgba().unwrap(),
        ))
    }

    /// An item for the captures.
    fn item(n: usize, title: &str, text: &str, icon: Icon, status: tray::Status) -> tray::Item {
        tray::Item {
            key: format!(":1.{n}/StatusNotifierItem"),
            service: format!(":1.{n}"),
            path: String::from(tray::ITEM_PATH),
            properties: tray::Properties {
                id: title.to_lowercase(),
                title: title.to_owned(),
                status,
                tooltip: Some((title.to_owned(), text.to_owned())),
                ..tray::Properties::default()
            },
            icon: Some(icon),
            attention: None,
            overlay: None,
        }
    }

    /// The bar of a `width` × `height` screen, the popup of the notification area open, the
    /// pointer on its second icon.
    fn bar_with_tray(width: f32, height: f32) -> Bar {
        use sherlock_ui::app_icon::AppIcon;
        use sherlock_ui::icon::Icon as Glyph;
        let mut state = Bar::default();
        let surface = Id::unique();
        let messages = [
            Message::Surface(surface),
            Message::Output(surface, None, Some((width, height))),
            Message::Resized(surface, width - 2.0 * metrics::TOP_BAR_MARGIN_SIDE),
            Message::Clock(LocalTime {
                year: 2026,
                weekday: 5,
                day: 27,
                month: 11,
                hour: 18,
                minute: 5,
            }),
            Message::Battery(Some(Battery {
                percent: 64,
                charge: Charge::Discharging,
                minutes: Some(150),
            })),
            Message::Network(Network::Connected {
                link: Link::Wifi,
                name: String::from("Maison"),
            }),
            Message::Sound(Sound {
                volume: 55,
                muted: false,
            }),
            Message::Desktop(Desktop::placeholder(Some("Code,Web,Jeu"), Some("Inquiry"))),
        ];
        for message in messages {
            let _ = state.update(message);
        }
        let active = tray::Status::Active;
        let items = [
            item(
                1,
                "Messagerie",
                "2 messages non lus",
                badge(44, [0xFF, 0x5B, 0xA3, 0xD0]),
                tray::Status::NeedsAttention,
            ),
            item(2, "VPN", "Connecté à Bureau", symbolic(Glyph::Lock), active),
            item(
                3,
                "Evidence",
                "Copie de 3 fichiers",
                drawing(AppIcon::Evidence),
                active,
            ),
            item(
                4,
                "Inquiry",
                "Compilation terminée",
                drawing(AppIcon::Inquiry),
                active,
            ),
            item(
                5,
                "Clé USB",
                "Prête à être retirée",
                symbolic(Glyph::UsbKey),
                active,
            ),
            item(
                6,
                "Sherlock",
                "Je veille",
                drawing(AppIcon::Sherlock),
                active,
            ),
        ];
        for mut item in items {
            item.properties.menu = Some(String::from("/MenuBar"));
            let _ = state.update(Message::Tray(Event::Changed(Box::new(item))));
        }
        // Sherlock kept in the bar (lot D-45a).
        let _ = state.update(Message::Pins(vec![String::from("sherlock")]));
        state
    }

    /// The menu of the VPN, as its application would export it.
    fn vpn_menu() -> bar::dbusmenu::Item {
        let entry = |id: i32, label: &str| bar::dbusmenu::Item {
            id,
            label: label.to_owned(),
            ..bar::dbusmenu::Item::default()
        };
        let children = vec![
            bar::dbusmenu::Item {
                toggle: bar::dbusmenu::Toggle::Check(true),
                ..entry(1, "Connecté")
            },
            entry(2, "Se déconnecter"),
            bar::dbusmenu::Item {
                separator: true,
                ..entry(3, "")
            },
            entry(4, "Réglages du VPN…"),
            entry(5, "Quitter"),
        ];
        bar::dbusmenu::Item {
            children,
            ..entry(0, "")
        }
    }

    /// Renders `view` at `size`, returns the image.
    fn render(view: iced::Element<'_, Message>, size: (u32, u32), name: &str) -> image::RgbaImage {
        let mut ui = Simulator::with_size(bar::settings(), (size.0 as f32, size.1 as f32), view);
        let snapshot = ui.snapshot(&theme::theme()).unwrap();
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
        let written = path.with_file_name(format!("{name}-{renderer}.png"));
        let _ = std::fs::remove_file(&written);
        let _ = snapshot.matches_image(&path).unwrap();
        image::open(&written).unwrap().to_rgba8()
    }

    /// Where the chevron is drawn: what the bar learns at its first redraw.
    fn locate_chevron(state: &mut Bar, bar_size: (u32, u32)) {
        let size = (bar_size.0 as f32, bar_size.1 as f32);
        let mut ui = Simulator::with_size(bar::settings(), size, state.view_bar());
        let redraw = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let _ = ui.simulate([redraw]);
        let located = ui.into_messages().find_map(|m| match m {
            Message::TrayChevron(bounds) => Some(bounds),
            _ => None,
        });
        let bounds = located.expect("the chevron says where it is");
        let _ = state.update(Message::TrayChevron(bounds));
    }

    /// Lays the bar and the open popup on the desktop of a `width` × `height` screen, at the
    /// scale the captures are rendered at, cut under the popup.
    fn screen(state: &mut Bar, (width, height): (u32, u32), name: &str) -> PathBuf {
        let bar_size = (
            width - 2 * metrics::TOP_BAR_MARGIN_SIDE as u32,
            metrics::TOP_BAR_HEIGHT as u32,
        );
        let bar = render(state.view_bar(), bar_size, &format!("{name}-bar"));
        let scale = i64::from(bar.width() / bar_size.0);
        let desktop = color::palette().desktop;
        let mut out = image::RgbaImage::from_pixel(
            width * scale as u32,
            height * scale as u32,
            image::Rgba([desktop.r, desktop.g, desktop.b, 255]),
        );
        let (bar_x, bar_y) = (
            metrics::TOP_BAR_MARGIN_SIDE as i64,
            metrics::TOP_BAR_MARGIN_TOP as i64,
        );
        image::imageops::overlay(&mut out, &bar, bar_x * scale, bar_y * scale);
        let popup = render(state.view_tray(), state.tray.size, &format!("{name}-popup"));
        let x = bar_x + state.tray_left() as i64;
        let y = bar_y + metrics::TOP_BAR_HEIGHT as i64;
        image::imageops::overlay(&mut out, &popup, x * scale, y * scale);
        let mut bottom = y + i64::from(state.tray.size.1) + 24;
        // An icon's menu, under its cell in the popup (software: the gap above it).
        let anchor = state.global.item.as_ref().map(|m| m.anchor);
        if let (Some(level), Some(bar::global_menu::Anchor::Popup(_, ax, ay))) =
            (state.global.levels.first(), anchor)
        {
            let menu = render(state.view(level.popup), level.size, &format!("{name}-menu"));
            let margins = bar::global_menu::margins(false, false);
            let (mx, my) = (
                x + (ax - margins.left) as i64,
                y + (ay - margins.top) as i64,
            );
            image::imageops::overlay(&mut out, &menu, mx * scale, my * scale);
            bottom = bottom.max(my + i64::from(level.size.1) + 24);
        }
        // The right half of the screen, down to under the popup.
        let left = (i64::from(width) / 2) * scale;
        let cut = (bottom * scale).min(i64::from(out.height())) as u32;
        let out = image::imageops::crop_imm(&out, left as u32, 0, out.width() - left as u32, cut)
            .to_image();
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        out.save(&path).unwrap();
        path
    }

    #[test]
    fn the_popup_renders_in_both_themes_at_two_sizes() {
        let _one = THEME.lock().unwrap_or_else(PoisonError::into_inner);
        for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
            color::set_active(which);
            for (width, height) in [(1920u32, 1080u32), (1366, 768)] {
                let mut state = bar_with_tray(width as f32, height as f32);
                let bar_size = (
                    width - 2 * metrics::TOP_BAR_MARGIN_SIDE as u32,
                    metrics::TOP_BAR_HEIGHT as u32,
                );
                locate_chevron(&mut state, bar_size);
                let chevron = state.tray.chevron.unwrap();
                let _ = state.update(Message::ToggleTray);
                assert!(state.tray_open());
                // The chevron has turned and the icons have breathed: at rest.
                let later = std::time::Instant::now() + Duration::from_secs(10);
                let _ = state.update(Message::Frame(later));
                assert!(!state.animating());
                // Under the chevron, within the bar.
                let (left, popup) = (state.tray_left(), sherlock_ui::tray::width(5));
                assert!(left <= chevron.center_x() && chevron.center_x() <= left + popup);
                assert!(left + popup <= bar_size.0 as f32);
                let _ = state.update(Message::TrayHover(Some(String::from(
                    ":1.2/StatusNotifierItem",
                ))));
                let path = screen(
                    &mut state,
                    (width, height),
                    &format!("tray-{width}{suffix}"),
                );
                assert!(std::fs::metadata(&path).unwrap().len() > 10_000);
                // The VPN's menu, on a right click (lot D-45a).
                let vpn = String::from(":1.2/StatusNotifierItem");
                let _ = state.update(Message::TrayClick(vpn, Click::Context));
                let address = state.global.item.as_ref().unwrap().address.clone();
                let _ = state.update(Message::AppMenuRead(address, 0, Ok(vpn_menu())));
                let _ = state.update(Message::HoverEntry(0, 1));
                let name = format!("tray-menu-{width}{suffix}");
                let path = screen(&mut state, (width, height), &name);
                assert!(std::fs::metadata(&path).unwrap().len() > 10_000);
            }
        }
        color::set_active(Theme::Dark);
    }
}
