//! Evidence's menus in the bar (lot D-36c), on a private bus started by the test (its own
//! `dbus-daemon`, never the user's bus), with a fake registrar: the menus registered, read as
//! the bar reads them, a click coming back as an action, and a change announced.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use evidence::app::Command as Run;
    use evidence::menubar::{self, Action, MenuBar, State};
    use iced::futures::StreamExt;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
    }

    impl Bus {
        fn start(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("evidence-{name}-{}", std::process::id()));
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

    /// A registrar that keeps what it is told.
    struct Registrar(Arc<Mutex<Vec<(u32, String)>>>);

    #[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
    impl Registrar {
        fn register_window(&self, window: u32, path: OwnedObjectPath) {
            self.0.lock().unwrap().push((window, path.to_string()));
        }
    }

    type Node = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

    fn label(node: &Node) -> String {
        node.1
            .get("label")
            .and_then(|l| String::try_from(l.try_clone().unwrap()).ok())
            .unwrap_or_default()
    }

    fn layout(client: &zbus::Connection, service: &str, parent: i32) -> Node {
        zbus::block_on(async {
            let names: Vec<&str> = Vec::new();
            let reply = client
                .call_method(
                    Some(service),
                    menubar::PATH,
                    Some("com.canonical.dbusmenu"),
                    "GetLayout",
                    &(parent, -1i32, names),
                )
                .await
                .unwrap();
            let (_, node): (u32, Node) = reply.body().deserialize().unwrap();
            node
        })
    }

    fn places() -> State {
        State {
            places: vec![("Accueil", PathBuf::from("/home/camille"))],
            ..State::default()
        }
    }

    #[test]
    fn the_menus_are_registered_read_and_clicked() {
        let bus = Bus::start("menubar");
        let registrar = bus.connect();
        let seen = Arc::new(Mutex::new(Vec::new()));
        zbus::block_on(async {
            let object = Registrar(Arc::clone(&seen));
            registrar
                .object_server()
                .at("/com/canonical/AppMenu/Registrar", object)
                .await
                .unwrap();
            registrar
                .request_name("com.canonical.AppMenu.Registrar")
                .await
                .unwrap();
        });
        let app = bus.connect();
        let service = app.unique_name().unwrap().to_string();
        let (menus, mut clicks) = zbus::block_on(MenuBar::serve(app, places())).unwrap();
        assert_eq!(*seen.lock().unwrap(), [(0, String::from("/MenuBar"))]);

        // Read as the bar reads it.
        let client = bus.connect();
        let root = layout(&client, &service, 0);
        let tops: Vec<Node> = root
            .2
            .iter()
            .map(|c| c.try_clone().unwrap().try_into().unwrap())
            .collect();
        let labels: Vec<String> = tops.iter().map(label).collect();
        assert_eq!(
            labels,
            ["Fichier", "Édition", "Présentation", "Aller", "Fenêtre"]
        );
        let go = layout(&client, &service, 4);
        let entries: Vec<Node> =
            go.2.iter()
                .map(|c| c.try_clone().unwrap().try_into().unwrap())
                .collect();
        assert!(entries.iter().any(|e| label(e) == "Accueil"));

        // « Nouveau dossier » clicked in the bar comes back as its command; a disabled
        // « Copier » (nothing selected) does nothing.
        let click = |id: i32| {
            zbus::block_on(client.call_method(
                Some(service.as_str()),
                menubar::PATH,
                Some("com.canonical.dbusmenu"),
                "Event",
                &(id, "clicked", Value::from(0i32), 0u32),
            ))
            .unwrap();
        };
        click(31);
        click(10);
        let action = iced::futures::executor::block_on(clicks.next());
        assert_eq!(action, Some(Action::Run(Run::NewFolder)));

        // A selection enables « Copier »: the bar is told the menus changed.
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .member("LayoutUpdated")
            .unwrap()
            .build();
        let mut signals =
            zbus::block_on(zbus::MessageStream::for_match_rule(rule, &client, None)).unwrap();
        menus.update(State {
            selection: true,
            ..places()
        });
        let (sender, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let signal = iced::futures::executor::block_on(signals.next());
            let _ = sender.send(signal.is_some());
        });
        assert_eq!(received.recv_timeout(Duration::from_secs(5)), Ok(true));
        click(31);
        let action = iced::futures::executor::block_on(clicks.next());
        assert_eq!(action, Some(Action::Run(Run::Copy)));
    }

    #[test]
    fn without_a_registrar_the_menus_are_only_exported() {
        let bus = Bus::start("menubar-alone");
        let app = bus.connect();
        let service = app.unique_name().unwrap().to_string();
        let served = zbus::block_on(MenuBar::serve(app, places()));
        assert!(served.is_ok());
        let root = layout(&bus.connect(), &service, 0);
        assert_eq!(root.2.len(), 5);
    }
}
