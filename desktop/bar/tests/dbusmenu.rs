//! The bar against a fake application that exports its menus with `com.canonical.dbusmenu`, on
//! a private bus started by the test (its own `dbus-daemon`, never the user's bus): the tree
//! read, a click on an entry sent as `Event` « clicked », a change announced, and an
//! application that never answers, which leaves the bar free.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use bar::app::{Bar, Message};
    use bar::dbusmenu::{self, Toggle};
    use bar::source::workspaces::{AppMenu, Desktop};
    use iced::futures::StreamExt;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{OwnedValue, Value};

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
                    .method_timeout(dbusmenu::TIMEOUT)
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

    use bar::dbusmenu::Node;

    fn node(id: i32, properties: &[(&str, Value<'_>)], children: Vec<Node>) -> Node {
        let properties = properties
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.try_to_owned().unwrap()))
            .collect();
        let children = children
            .into_iter()
            .map(|c| OwnedValue::try_from(Value::from(c)).unwrap())
            .collect();
        (id, properties, children)
    }

    /// Kate's « Fichier » and « Affichage », as Kate 25.04 exports them (trimmed).
    fn kate_menus() -> Node {
        let file = node(
            1,
            &[
                ("label", Value::from("_Fichier")),
                ("children-display", Value::from("submenu")),
            ],
            vec![
                node(
                    2,
                    &[
                        ("label", Value::from("_Nouveau")),
                        ("shortcut", Value::from(vec![vec!["Control", "N"]])),
                    ],
                    vec![],
                ),
                node(4, &[("type", Value::from("separator"))], vec![]),
                node(
                    6,
                    &[
                        ("label", Value::from("_Récemment ouvert(s)")),
                        ("children-display", Value::from("submenu")),
                        ("enabled", Value::from(false)),
                    ],
                    vec![],
                ),
                node(
                    192,
                    &[
                        ("label", Value::from("Ouvrir un _dossier...")),
                        (
                            "shortcut",
                            Value::from(vec![vec!["Control", "T"], vec!["O"]]),
                        ),
                    ],
                    vec![],
                ),
            ],
        );
        let view = node(
            38,
            &[
                ("label", Value::from("Afficha_ge")),
                ("children-display", Value::from("submenu")),
            ],
            vec![node(
                40,
                &[
                    ("label", Value::from("Afficher la _barre latérale")),
                    ("toggle-type", Value::from("checkmark")),
                    ("toggle-state", Value::from(1i32)),
                ],
                vec![],
            )],
        );
        node(
            0,
            &[("children-display", Value::from("submenu"))],
            vec![file, view],
        )
    }

    /// What the fake application was told.
    #[derive(Default)]
    struct Seen {
        events: Vec<(i32, String)>,
        shown: Vec<i32>,
    }

    struct Menus {
        seen: Arc<Mutex<Seen>>,
        /// Never answers: the application hangs.
        hangs: bool,
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl Menus {
        async fn get_layout(
            &self,
            parent: i32,
            _depth: i32,
            _names: Vec<String>,
        ) -> zbus::fdo::Result<(u32, Node)> {
            if self.hangs {
                iced::futures::future::pending::<()>().await;
            }
            let root = kate_menus();
            let found = if parent == 0 {
                root
            } else {
                let children = root.2.into_iter().filter_map(|c| {
                    let c: Node = c.try_into().ok()?;
                    (c.0 == parent).then_some(c)
                });
                children.into_iter().next().unwrap()
            };
            Ok((7, found))
        }

        fn about_to_show(&self, id: i32) -> bool {
            self.seen.lock().unwrap().shown.push(id);
            false
        }

        fn event(&self, id: i32, event: String, _data: OwnedValue, _timestamp: u32) {
            self.seen.lock().unwrap().events.push((id, event));
        }

        #[zbus(signal)]
        async fn layout_updated(
            emitter: &SignalEmitter<'_>,
            revision: u32,
            parent: i32,
        ) -> zbus::Result<()>;
    }

    /// The fake application on `bus`, its menus at `/MenuBar/1`.
    fn application(bus: &Bus, hangs: bool) -> (zbus::Connection, AppMenu, Arc<Mutex<Seen>>) {
        let app = bus.connect();
        let seen = Arc::new(Mutex::new(Seen::default()));
        let menus = Menus {
            seen: Arc::clone(&seen),
            hangs,
        };
        zbus::block_on(app.object_server().at("/MenuBar/1", menus)).unwrap();
        let address = AppMenu {
            service: app.unique_name().unwrap().to_string(),
            path: String::from("/MenuBar/1"),
        };
        (app, address, seen)
    }

    #[test]
    fn the_menus_are_read_and_a_click_reaches_the_application() {
        let bus = Bus::start("dbusmenu-click");
        let (_app, address, seen) = application(&bus, false);
        let client = bus.connect();
        let root = zbus::block_on(dbusmenu::layout(&client, &address, 0)).unwrap();
        let labels: Vec<_> = root.children.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Fichier", "Affichage"]);
        let file = &root.children[0];
        assert!(file.opens());
        assert_eq!(file.children[0].keys.as_deref(), Some("Ctrl+N"));
        assert!(file.children[1].separator);
        assert!(!file.children[2].enabled && file.children[2].opens());
        assert_eq!(file.children[3].label, "Ouvrir un dossier...");
        assert_eq!(file.children[3].keys.as_deref(), Some("Ctrl+T, O"));
        assert_eq!(root.find(40).unwrap().toggle, Toggle::Check(true));

        // The bar shows them, then a click on « Nouveau » goes to Kate as « clicked ».
        let mut state = Bar::default();
        let _ = state.update(Message::Surface(iced::window::Id::unique()));
        let mut desktop = Desktop::placeholder(None, Some("Sans titre — Kate"));
        desktop.menu = Some(address.clone());
        let _ = state.update(Message::Desktop(desktop));
        assert_eq!(state.global.address.as_ref(), Some(&address));
        let _ = state.update(Message::AppMenuRead(address.clone(), 0, Ok(root.clone())));
        assert_eq!(state.global.tops().len(), 2);
        let _ = state.update(Message::OpenAppMenu(1));
        assert!(!state.app_menu_open(), "opens once Kate has filled it");
        let shown = zbus::block_on(dbusmenu::about_to_show(&client, &address, 1)).unwrap();
        assert!(!shown);
        let file = zbus::block_on(dbusmenu::layout(&client, &address, 1)).unwrap();
        let _ = state.update(Message::AppMenuRead(address.clone(), 1, Ok(file)));
        assert!(state.app_menu_open());
        // A disabled entry does nothing; « Nouveau » is sent, and the menu closes.
        let _ = state.update(Message::PickEntry(0, 2));
        assert!(state.app_menu_open());
        let picked = state.picked(0, 0);
        assert_eq!(picked, Some((address.clone(), 2)));
        let _ = state.update(Message::PickEntry(0, 0));
        assert!(!state.app_menu_open());
        // What the task of the click sends.
        let (menu, id) = picked.unwrap();
        zbus::block_on(dbusmenu::clicked(&client, &menu, id)).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.events, [(2, String::from("clicked"))]);
        assert_eq!(seen.shown, [1]);
    }

    #[test]
    fn a_change_of_the_menus_is_announced() {
        let bus = Bus::start("dbusmenu-change");
        let (app, address, _) = application(&bus, false);
        let client = bus.connect();
        let (sender, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut changes = dbusmenu::changes(client, address);
            while iced::futures::executor::block_on(changes.next()).is_some() {
                if sender.send(()).is_err() {
                    return;
                }
            }
        });
        // The stream listens once its match rule is in place: the change is announced until
        // it is heard, for 5 s at most.
        let emitter = SignalEmitter::new(&app, "/MenuBar/1").unwrap();
        let heard = (0..25).any(|_| {
            zbus::block_on(Menus::layout_updated(&emitter, 8, 0)).unwrap();
            received.recv_timeout(Duration::from_millis(200)).is_ok()
        });
        assert!(heard, "LayoutUpdated reaches the bar");
    }

    #[test]
    fn an_application_that_does_not_answer_freezes_nothing() {
        let bus = Bus::start("dbusmenu-hang");
        let (_app, address, _) = application(&bus, true);
        let client = bus.connect();
        let started = Instant::now();
        let read = zbus::block_on(dbusmenu::layout(&client, &address, 0));
        assert!(read.is_err(), "the call gives up");
        let waited = started.elapsed();
        assert!(
            waited < dbusmenu::TIMEOUT + Duration::from_secs(1),
            "{waited:?}"
        );
        // The bar shows the name alone, and goes on.
        let mut state = Bar::default();
        let mut desktop = Desktop::placeholder(None, Some("Kate"));
        desktop.menu = Some(address.clone());
        let _ = state.update(Message::Desktop(desktop));
        let error = read.unwrap_err().to_string();
        let _ = state.update(Message::AppMenuRead(address, 0, Err(error)));
        assert!(state.global.root.is_none() && !state.app_menu_open());
        assert_eq!(state.app_name().as_deref(), Some("Kate"));
    }
}
