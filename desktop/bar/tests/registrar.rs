//! `com.canonical.AppMenu.Registrar` on a private bus started by the test (its own
//! `dbus-daemon`, never the user's bus): it answers at once, passes each registration on with
//! the caller's process, leaves the name to a registrar already there, and withdraws the menus
//! that are gone.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::Arc;
    use std::sync::mpsc::{self, Receiver};
    use std::time::{Duration, Instant};

    use bar::registrar::{self, Change};
    use zbus::zvariant::ObjectPath;

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

        /// The registrar on this bus, its changes on the receiver.
        fn registrar(&self) -> (zbus::Connection, Receiver<Change>, bool) {
            let bus = self.connect();
            let (sender, changes) = mpsc::channel();
            let sender = std::sync::Mutex::new(sender);
            let sink: registrar::Sink = Arc::new(move |change| {
                let _ = sender.lock().unwrap().send(change);
            });
            let served = zbus::block_on(registrar::serve(&bus, sink)).unwrap();
            (bus, changes, served)
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The smallest menu an application exports: its version, as introspection shows it.
    struct Menu;

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl Menu {
        #[zbus(property)]
        fn version(&self) -> u32 {
            3
        }
    }

    fn register(app: &zbus::Connection, window: u32, path: &'static str) -> zbus::Result<()> {
        let path = ObjectPath::from_static_str_unchecked(path);
        let body = (window, path);
        let method = "RegisterWindow";
        zbus::block_on(app.call_method(
            Some(registrar::NAME),
            registrar::PATH,
            Some(registrar::NAME),
            method,
            &body,
        ))?;
        Ok(())
    }

    fn unregister(app: &zbus::Connection, window: u32) -> zbus::Result<()> {
        let method = "UnregisterWindow";
        zbus::block_on(app.call_method(
            Some(registrar::NAME),
            registrar::PATH,
            Some(registrar::NAME),
            method,
            &window,
        ))?;
        Ok(())
    }

    fn next(changes: &Receiver<Change>) -> Change {
        changes
            .recv_timeout(Duration::from_secs(5))
            .expect("a change within 5 s")
    }

    #[test]
    fn a_registration_is_answered_at_once_and_passed_on_with_its_process() {
        let bus = Bus::start("registrar-answer");
        let (_registrar, changes, served) = bus.registrar();
        assert!(served, "the name is free on a private bus");
        let app = bus.connect();
        let service = app.unique_name().unwrap().to_string();
        // The best of five: a loaded machine can pause the test during one call, which says
        // nothing of the registrar.
        let mut best = Duration::MAX;
        for window in 1..=5 {
            let started = Instant::now();
            register(&app, window, "/MenuBar/1").unwrap();
            best = best.min(started.elapsed());
        }
        assert!(best < Duration::from_millis(50), "answered in {best:?}");
        assert_eq!(
            next(&changes),
            Change::Register {
                window: 1,
                pid: std::process::id(),
                service: service.clone(),
                path: String::from("/MenuBar/1"),
            }
        );
        let menu: (String, zbus::zvariant::OwnedObjectPath) = zbus::block_on(async {
            let reply = app
                .call_method(
                    Some(registrar::NAME),
                    registrar::PATH,
                    Some(registrar::NAME),
                    "GetMenuForWindow",
                    &3u32,
                )
                .await
                .unwrap();
            reply.body().deserialize().unwrap()
        });
        assert_eq!(
            (menu.0.as_str(), menu.1.as_str()),
            (service.as_str(), "/MenuBar/1")
        );
    }

    #[test]
    fn a_registrar_already_there_keeps_the_name() {
        let bus = Bus::start("registrar-taken");
        let (_first, _, served) = bus.registrar();
        assert!(served);
        let (_second, changes, served) = bus.registrar();
        assert!(!served, "the second one stays out of the way");
        let app = bus.connect();
        register(&app, 1, "/MenuBar/1").unwrap();
        assert!(changes.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn only_the_menus_that_are_gone_are_withdrawn() {
        let bus = Bus::start("registrar-withdraw");
        let (_registrar, changes, _) = bus.registrar();
        // Qt under Wayland: two windows, the same id 1; the second one's menu stays exported.
        // Served from the builder, which returns once zbus listens for method calls (see
        // `dbusmenu.rs`): the registrar's question about it cannot be lost.
        let app = zbus::block_on(async {
            zbus::connection::Builder::address(bus.address.as_str())
                .unwrap()
                .serve_at("/MenuBar/2", Menu)
                .unwrap()
                .build()
                .await
                .unwrap()
        });
        let service = app.unique_name().unwrap().to_string();
        register(&app, 1, "/MenuBar/1").unwrap();
        register(&app, 1, "/MenuBar/2").unwrap();
        let _ = (next(&changes), next(&changes));
        unregister(&app, 1).unwrap();
        assert_eq!(
            next(&changes),
            Change::Unregister {
                service: service.clone(),
                path: String::from("/MenuBar/1"),
            }
        );
        assert!(changes.recv_timeout(Duration::from_millis(300)).is_err());
        // The application leaves the bus: its last menu goes with it.
        drop(app);
        assert_eq!(
            next(&changes),
            Change::Unregister {
                service,
                path: String::from("/MenuBar/2"),
            }
        );
    }
}
