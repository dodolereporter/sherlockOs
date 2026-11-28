//! A left click on an icon of the notification area reaches its application (Q-48), against
//! fake applications that put their item there the way the real ones do, on a private bus (its
//! own `dbus-daemon`, never the user's):
//!
//! - Chromium and Electron (Discord): the item at `/StatusNotifierItem`, registered by the
//!   unique name of its connection, `ItemIsMenu` false, `Activate` answered;
//! - libayatana-appindicator (Steam): registered by an object path alone, no `ItemIsMenu`, and
//!   `Activate` refused with `UnknownMethod` (« No handler for Activate »): its menu opens;
//! - `xembedsniproxy` (AnyDesk): a connection of its own per icon, registered by its unique
//!   name, `Activate (x, y)` turned into an X11 click at that point of the screen.
//!
//! The bar runs as in the session: the messages in the order `iced_exwlshell` gives them (the
//! button's click before the subscription hears the release), and the tasks it returns run,
//! their messages fed back, on the session bus the bar uses (`dbusmenu::bus`), here the private
//! one. In the popup, then once the icons are kept in the bar.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use bar::app::{Bar, Message};
    use bar::tray::{self, Event, watcher};
    use iced::advanced::clipboard::Null;
    use iced::advanced::renderer::Headless;
    use iced::futures::StreamExt;
    use iced::mouse::{self, Button};
    use iced::window::Id;
    use iced_runtime::user_interface::{self, UserInterface};
    use iced_runtime::{Action, Task};
    use zbus::zvariant::OwnedObjectPath;

    /// A private bus: its own `dbus-daemon`, socket in a folder of its own, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
    }

    impl Bus {
        fn start() -> Self {
            let dir = std::env::temp_dir().join(format!("bar-tray-click-{}", std::process::id()));
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
                .expect("dbus-daemon (package dbus-daemon) is needed by this test");
            let mut address = String::new();
            let stdout = daemon.stdout.take().unwrap();
            BufReader::new(stdout).read_line(&mut address).unwrap();
            Self {
                daemon,
                address: address.trim().to_owned(),
                dir,
            }
        }

        fn connect(&self) -> zbus::Connection {
            zbus::block_on(async {
                zbus::connection::Builder::address(self.address.as_str())
                    .unwrap()
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

    /// What an application was asked.
    type Seen = Arc<Mutex<Vec<String>>>;
    type Pixels = Vec<(i32, i32, Vec<u8>)>;

    fn pixels() -> Pixels {
        vec![(2, 2, [0xFF, 0x85, 0xC5, 0x95].repeat(4))]
    }

    fn note(seen: &Seen, what: String) {
        seen.lock().unwrap().push(what);
    }

    /// Chromium's `StatusIconLinuxDbus`, as Electron applications have it.
    struct Chromium {
        seen: Seen,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Chromium {
        #[zbus(property)]
        fn category(&self) -> String {
            String::from("ApplicationStatus")
        }
        #[zbus(property)]
        fn id(&self) -> String {
            String::from("chrome_status_icon_1")
        }
        #[zbus(property)]
        fn title(&self) -> String {
            String::from("Messagerie")
        }
        #[zbus(property)]
        fn status(&self) -> String {
            String::from("Active")
        }
        #[zbus(property)]
        fn icon_pixmap(&self) -> Pixels {
            pixels()
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn menu(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from("/com/canonical/dbusmenu").unwrap()
        }
        fn activate(&self, x: i32, y: i32) {
            note(&self.seen, format!("Activate {x} {y}"));
        }
        fn secondary_activate(&self, x: i32, y: i32) {
            note(&self.seen, format!("SecondaryActivate {x} {y}"));
        }
        fn context_menu(&self, x: i32, y: i32) {
            note(&self.seen, format!("ContextMenu {x} {y}"));
        }
        fn scroll(&self, delta: i32, orientation: &str) {
            note(&self.seen, format!("Scroll {delta} {orientation}"));
        }
        fn provide_xdg_activation_token(&self, _token: &str) {}
    }

    /// libayatana-appindicator: no `ItemIsMenu`, and `Activate` refused when the application
    /// connected no handler, so that the host shows the menu.
    struct Ayatana {
        seen: Seen,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Ayatana {
        #[zbus(property)]
        fn id(&self) -> String {
            String::from("games-store")
        }
        #[zbus(property)]
        fn category(&self) -> String {
            String::from("ApplicationStatus")
        }
        #[zbus(property)]
        fn status(&self) -> String {
            String::from("Active")
        }
        #[zbus(property)]
        fn icon_name(&self) -> String {
            String::from("games-store-tray-not-installed")
        }
        #[zbus(property)]
        fn title(&self) -> String {
            String::from("Jeux")
        }
        #[zbus(property)]
        fn menu(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from(AYATANA_MENU).unwrap()
        }
        fn activate(&self, _x: i32, _y: i32) -> zbus::fdo::Result<()> {
            note(&self.seen, String::from("Activate refused"));
            Err(zbus::fdo::Error::UnknownMethod(String::from(
                "No handler for Activate",
            )))
        }
        fn secondary_activate(&self, _x: i32, _y: i32) {}
        fn scroll(&self, _delta: i32, _orientation: &str) {}
        #[zbus(name = "XAyatanaSecondaryActivate")]
        fn x_ayatana_secondary_activate(&self, _timestamp: u32) {}
    }

    const AYATANA_PATH: &str = "/org/ayatana/NotificationItem/games_store";
    const AYATANA_MENU: &str = "/org/ayatana/NotificationItem/games_store/Menu";

    /// Its menu, as libdbusmenu exports one.
    struct Menu {
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
    impl Menu {
        fn get_layout(
            &self,
            _parent: i32,
            _depth: i32,
            _names: Vec<String>,
        ) -> (u32, bar::dbusmenu::Node) {
            note(&self.seen, String::from("GetLayout"));
            let children = [entry(1, "Bibliothèque"), entry(2, "Quitter")]
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
        fn event(&self, _id: i32, _event: String, _data: zbus::zvariant::OwnedValue, _time: u32) {}
    }

    /// `xembedsniproxy`: one connection per X11 icon, its item at `/StatusNotifierItem`, no
    /// D-Bus menu; `Activate` clicks the X11 icon at that point of the screen.
    struct Xembed {
        seen: Seen,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl Xembed {
        #[zbus(property)]
        fn id(&self) -> String {
            String::from("remote-desk")
        }
        #[zbus(property)]
        fn title(&self) -> String {
            String::from("Bureau à distance")
        }
        #[zbus(property)]
        fn category(&self) -> String {
            String::from("ApplicationStatus")
        }
        #[zbus(property)]
        fn status(&self) -> String {
            String::from("Active")
        }
        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }
        #[zbus(property)]
        fn icon_pixmap(&self) -> Pixels {
            pixels()
        }
        fn activate(&self, x: i32, y: i32) {
            note(&self.seen, format!("Activate {x} {y}"));
        }
        fn secondary_activate(&self, _x: i32, _y: i32) {}
        fn context_menu(&self, x: i32, y: i32) {
            note(&self.seen, format!("ContextMenu {x} {y}"));
        }
        fn scroll(&self, _delta: i32, _orientation: &str) {}
    }

    /// Registers `registered` with the watcher from `connection`, once there is one.
    fn register(connection: &zbus::Connection, registered: &str) {
        let done = (0..150).any(|_| {
            let called = zbus::block_on(connection.call_method(
                Some(watcher::NAME),
                watcher::PATH,
                Some(watcher::NAME),
                "RegisterStatusNotifierItem",
                &registered,
            ));
            called.is_ok() || {
                std::thread::sleep(Duration::from_millis(100));
                false
            }
        });
        assert!(done, "the watcher answers");
    }

    /// The bar's host, on the connection the bar uses, its events on a channel.
    fn host() -> Receiver<Event> {
        let (sender, received) = mpsc::channel();
        std::thread::spawn(move || {
            let mut events = tray::watch();
            while let Some(event) = zbus::block_on(events.next()) {
                if sender.send(event).is_err() {
                    return;
                }
            }
        });
        received
    }

    /// Runs `task` as the runtime would, and what its messages return in turn.
    fn run(bar: &mut Bar, task: Task<Message>) {
        let mut pending = vec![task];
        while let Some(task) = pending.pop() {
            let Some(mut stream) = iced_runtime::task::into_stream(task) else {
                continue;
            };
            while let Some(action) = zbus::block_on(stream.next()) {
                if let Action::Output(
                    message @ (Message::TraySent(..) | Message::AppMenuRead(..)),
                ) = action
                {
                    pending.push(bar.update(message));
                }
            }
        }
    }

    /// `message`, and the tasks it returns run.
    fn update(bar: &mut Bar, message: Message) {
        let task = bar.update(message);
        run(bar, task);
    }

    /// The bar's widgets for one of its surfaces, kept from one event to the next as the
    /// runtime keeps them: the button pressed stays pressed until the release.
    struct Surface {
        id: Id,
        size: iced::Size,
        cache: Option<user_interface::Cache>,
        renderer: iced::Renderer,
    }

    impl Surface {
        fn new(id: Id, size: iced::Size) -> Self {
            let renderer = zbus::block_on(<iced::Renderer as Headless>::new(
                iced::Font::DEFAULT,
                iced::Pixels(14.0),
                Some("tiny-skia"),
            ))
            .unwrap();
            Self {
                id,
                size,
                cache: None,
                renderer,
            }
        }

        /// `events` at `at`, through the widgets built from the bar as it is now; what they
        /// say goes to the bar, its tasks run.
        fn deliver(&mut self, bar: &mut Bar, at: iced::Point, events: &[iced::Event]) {
            let cache = self.cache.take().unwrap_or_default();
            let mut ui =
                UserInterface::build(bar.view(self.id), self.size, cache, &mut self.renderer);
            let mut messages = Vec::new();
            let cursor = mouse::Cursor::Available(at);
            let _ = ui.update(events, cursor, &mut self.renderer, &mut Null, &mut messages);
            self.cache = Some(ui.into_cache());
            for message in messages {
                update(bar, message);
            }
        }
    }

    /// A left click at `at` of `surface`, in the order the bar gets it in the session: the
    /// widgets get each event first, then the subscription hears it (`iced_exwlshell`
    /// broadcasts it after updating the widgets of the surface).
    fn click(bar: &mut Bar, surface: &mut Surface, at: iced::Point) {
        let redraw = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        let moved = mouse::Event::CursorMoved { position: at };
        surface.deliver(bar, at, &[iced::Event::Mouse(moved), redraw.clone()]);
        for event in [
            mouse::Event::ButtonPressed(Button::Left),
            mouse::Event::ButtonReleased(Button::Left),
        ] {
            surface.deliver(bar, at, &[iced::Event::Mouse(event), redraw.clone()]);
            update(bar, Message::TrayPointer(surface.id, event));
        }
    }

    /// The widgets of the open popup.
    fn popup_surface(bar: &Bar) -> Surface {
        let (width, height) = bar.tray.size;
        Surface::new(
            bar.tray.popup.unwrap(),
            iced::Size::new(width as f32, height as f32),
        )
    }

    /// The middle of the cell of `key` in the popup.
    fn cell(bar: &Bar, key: &str) -> iced::Point {
        use sherlock_ui::tray as look;
        let index = bar.tray.in_popup().position(|i| i.key == key).unwrap();
        let inset = look::PADDING + sherlock_ui::metrics::BORDER_WIDTH;
        let step = look::CELL + look::GAP;
        let margins = bar.tray.margins;
        iced::Point::new(
            margins.left + inset + (index % look::COLUMNS) as f32 * step + look::CELL / 2.0,
            margins.top + inset + (index / look::COLUMNS) as f32 * step + look::CELL / 2.0,
        )
    }

    /// The middle of the icon of `key` kept in the bar, once the bar is drawn and says where.
    fn pinned(bar: &mut Bar, strip: &mut Surface, key: &str) -> iced::Point {
        let redraw = iced::Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ));
        strip.deliver(bar, iced::Point::ORIGIN, &[redraw]);
        let row = bar.tray.pinned_row.unwrap();
        let index = bar.tray.pinned().position(|i| i.key == key).unwrap();
        let slot = row.width / bar.tray.pinned().count() as f32;
        iced::Point::new(row.x + (index as f32 + 0.5) * slot, row.center_y())
    }

    #[test]
    fn a_left_click_reaches_chromium_ayatana_and_xembedsniproxy_items() {
        let bus = Bus::start();
        // The bar reaches the session bus through `dbusmenu::bus`: here, the private one.
        // SAFETY: first statement after the bus starts, before any thread of this test binary
        // reads the environment; this binary holds this one test.
        unsafe { std::env::set_var("DBUS_SESSION_BUS_ADDRESS", &bus.address) };
        let events = host();

        let chromium_seen = Seen::default();
        let chromium = bus.connect();
        let item = Chromium {
            seen: Arc::clone(&chromium_seen),
        };
        zbus::block_on(chromium.object_server().at(tray::ITEM_PATH, item)).unwrap();
        let unique = chromium.unique_name().unwrap().to_string();
        register(&chromium, &unique);

        let ayatana_seen = Seen::default();
        let ayatana = bus.connect();
        let item = Ayatana {
            seen: Arc::clone(&ayatana_seen),
        };
        zbus::block_on(ayatana.object_server().at(AYATANA_PATH, item)).unwrap();
        let menu = Menu {
            seen: Arc::clone(&ayatana_seen),
        };
        zbus::block_on(ayatana.object_server().at(AYATANA_MENU, menu)).unwrap();
        register(&ayatana, AYATANA_PATH);

        let xembed_seen = Seen::default();
        let xembed = bus.connect();
        let item = Xembed {
            seen: Arc::clone(&xembed_seen),
        };
        zbus::block_on(xembed.object_server().at(tray::ITEM_PATH, item)).unwrap();
        let unique = xembed.unique_name().unwrap().to_string();
        register(&xembed, &unique);

        let mut bar = Bar::default();
        let surface = Id::unique();
        let _ = bar.update(Message::Surface(surface));
        let _ = bar.update(Message::Resized(surface, 1896.0));
        let chevron =
            iced::Rectangle::new(iced::Point::new(1500.0, 3.0), iced::Size::new(32.0, 30.0));
        let _ = bar.update(Message::TrayChevron(chevron));
        let mut keys = Vec::new();
        while keys.len() < 3 {
            let Event::Changed(item) = events
                .recv_timeout(Duration::from_secs(15))
                .expect("the three items within 15 s")
            else {
                continue;
            };
            keys.push((item.properties.id.clone(), item.key.clone()));
            let _ = bar.update(Message::Tray(Event::Changed(item)));
        }
        let key = |id: &str| keys.iter().find(|(i, _)| i == id).unwrap().1.clone();
        let (chromium_key, ayatana_key, xembed_key) = (
            key("chrome_status_icon_1"),
            key("games-store"),
            key("remote-desk"),
        );
        assert_eq!(
            chromium_key,
            format!("{}{}", chromium.unique_name().unwrap(), tray::ITEM_PATH)
        );
        assert_eq!(
            ayatana_key,
            format!("{}{AYATANA_PATH}", ayatana.unique_name().unwrap())
        );

        // In the popup.
        for (key, seen) in [(&chromium_key, &chromium_seen), (&xembed_key, &xembed_seen)] {
            update(&mut bar, Message::ToggleTray);
            let mut popup = popup_surface(&bar);
            let at = cell(&bar, key);
            click(&mut bar, &mut popup, at);
            let seen = seen.lock().unwrap().clone();
            assert!(
                seen.iter().any(|s| s.starts_with("Activate ")),
                "{key}: {seen:?}"
            );
            assert!(!bar.tray_open(), "the popup folds back");
            let _ = bar.update(Message::Frame(
                std::time::Instant::now() + Duration::from_secs(2),
            ));
        }
        update(&mut bar, Message::ToggleTray);
        let mut popup = popup_surface(&bar);
        let at = cell(&bar, &ayatana_key);
        click(&mut bar, &mut popup, at);
        assert_eq!(
            ayatana_seen.lock().unwrap().as_slice(),
            ["Activate refused", "GetLayout"]
        );
        assert!(bar.app_menu_open(), "the menu of an item without Activate");
        let _ = bar.update(Message::MenuKey(bar::menu::Key::Close));
        let _ = bar.update(Message::Frame(
            std::time::Instant::now() + Duration::from_secs(2),
        ));
        chromium_seen.lock().unwrap().clear();
        xembed_seen.lock().unwrap().clear();
        ayatana_seen.lock().unwrap().clear();

        // Kept in the bar, all three: the same clicks there.
        let pins = vec![
            String::from("chrome_status_icon_1"),
            String::from("games-store"),
            String::from("remote-desk"),
        ];
        update(&mut bar, Message::Pins(pins));
        assert_eq!(bar.tray.pinned().count(), 3);
        let mut strip = Surface::new(
            surface,
            iced::Size::new(1896.0, sherlock_ui::metrics::TOP_BAR_HEIGHT),
        );
        for (key, seen) in [(&chromium_key, &chromium_seen), (&xembed_key, &xembed_seen)] {
            let at = pinned(&mut bar, &mut strip, key);
            click(&mut bar, &mut strip, at);
            let seen = seen.lock().unwrap().clone();
            assert!(
                seen.iter().any(|s| s.starts_with("Activate ")),
                "{key} in the bar: {seen:?}"
            );
        }
        let at = pinned(&mut bar, &mut strip, &ayatana_key);
        click(&mut bar, &mut strip, at);
        assert_eq!(
            ayatana_seen.lock().unwrap().as_slice(),
            ["Activate refused", "GetLayout"]
        );
        assert!(bar.app_menu_open());
    }
}
