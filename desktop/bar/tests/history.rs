//! The notification centre against the real notification daemon, on a private bus started by
//! the test (its own `dbus-daemon`, nothing of the user's session): the history listed and
//! grouped, `Changed`, an action run from the centre, « Tout effacer », and a bus without the
//! daemon.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use bar::history;
    use iced::futures::StreamExt;
    use notifications::core::{Core, Sink};
    use notifications::server;
    use zbus::zvariant::Value;

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

        fn connect(&self) -> zbus::Connection {
            zbus::block_on(async {
                zbus::connection::Builder::address(self.address.as_str())
                    .unwrap()
                    .build()
                    .await
                    .unwrap()
            })
        }

        /// The notification daemon on this bus, without bubbles.
        fn serve(&self) -> Arc<Core> {
            let sink: Sink = Box::new(|_| {});
            let core = Core::new(sink, None);
            let builder =
                zbus::blocking::connection::Builder::address(self.address.as_str()).unwrap();
            let served = server::start(builder, &core).unwrap();
            std::mem::forget(served.expect("the name is free on a private bus"));
            core
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// `Notify` from an application named `app`, which never expires.
    fn notify(bus: &zbus::Connection, app: &str, summary: &str, actions: &[&str]) -> u32 {
        let hints: HashMap<&str, Value<'_>> = HashMap::new();
        let body = &(
            app,
            0u32,
            "",
            summary,
            "Le <b>corps</b>",
            actions,
            hints,
            0i32,
        );
        zbus::block_on(async {
            let reply = bus
                .call_method(
                    Some("org.freedesktop.Notifications"),
                    "/org/freedesktop/Notifications",
                    Some("org.freedesktop.Notifications"),
                    "Notify",
                    body,
                )
                .await
                .unwrap();
            reply.body().deserialize().unwrap()
        })
    }

    /// The `Changed` signals the centre sees on `client`, through a channel.
    fn changes(client: &zbus::Connection) -> Receiver<()> {
        let stream = zbus::block_on(history::changes(client)).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            zbus::block_on(async move {
                let mut stream = Box::pin(stream);
                while stream.next().await.is_some() {
                    if sender.send(()).is_err() {
                        return;
                    }
                }
            });
        });
        receiver
    }

    /// Waits for the next `Changed`, 5 s at most.
    fn changed(changes: &Receiver<()>) {
        let got = changes.recv_timeout(Duration::from_secs(5));
        assert!(got.is_ok(), "no Changed within 5 s");
    }

    #[test]
    fn the_centre_lists_groups_runs_actions_and_clears() {
        let bus = Bus::start("history");
        let core = bus.serve();
        let client = bus.connect();
        let changes = changes(&client);

        let build = notify(&client, "Sherlock", "Build terminé", &[]);
        changed(&changes);
        let actions = ["restart", "Relancer", "logs", "Voir les logs"];
        let stopped = notify(&client, "Conteneurs", "front-next s'est arrêté", &actions);
        changed(&changes);
        let again = notify(&client, "Sherlock", "Tests passés", &[]);
        changed(&changes);

        let entries = zbus::block_on(history::list(&client)).unwrap();
        let ids: Vec<_> = entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, [again, stopped, build]);
        assert_eq!(entries[1].body, "Le corps");
        assert!(entries[1].open);
        let groups = history::group(entries);
        let apps: Vec<_> = groups.iter().map(|g| g.app.as_str()).collect();
        assert_eq!(apps, ["Sherlock", "Conteneurs"]);
        assert_eq!(groups[0].entries.len(), 2);

        // « Relancer » from the centre: the action goes to the application, the notification
        // closes and stays in the history without its buttons.
        zbus::block_on(history::invoke(&client, stopped, "restart", None)).unwrap();
        changed(&changes);
        assert!(core.open(stopped).is_none());
        let entries = zbus::block_on(history::list(&client)).unwrap();
        assert!(!entries.iter().any(|e| e.id == stopped && e.open));
        // With the click's activation token (lot Q-52d): the action runs the same.
        let open = notify(&client, "Sherlock", "Rapport prêt", &["open", "Ouvrir"]);
        changed(&changes);
        zbus::block_on(history::invoke(&client, open, "open", Some("centre-token"))).unwrap();
        changed(&changes);
        assert!(core.open(open).is_none());
        // Again: it is no longer open.
        assert!(zbus::block_on(history::invoke(&client, stopped, "restart", None)).is_err());

        zbus::block_on(history::clear(&client)).unwrap();
        changed(&changes);
        assert!(zbus::block_on(history::list(&client)).unwrap().is_empty());
    }

    #[test]
    fn without_the_daemon_the_centre_gets_an_error() {
        let bus = Bus::start("absent");
        let client = bus.connect();
        assert!(zbus::block_on(history::list(&client)).is_err());
        assert!(zbus::block_on(history::clear(&client)).is_err());
        assert!(zbus::block_on(history::invoke(&client, 1, "default", None)).is_err());
    }
}
