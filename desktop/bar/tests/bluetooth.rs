//! The Bluetooth devices against a fake BlueZ on a private bus started by the test (its own
//! `dbus-daemon`, never the machine's system bus): the list, connecting, disconnecting, pairing
//! with the agent's confirmation, forgetting, a scan, BlueZ absent, and a device that does not
//! answer. Every device is fictitious.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::{Duration, Instant};

    use bar::system::Problem;
    use bar::system::bluetooth::{self, Asked, Kind, Pairing};
    use iced::futures::StreamExt;
    use zbus::fdo;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{ObjectPath, OwnedObjectPath};

    /// A private bus: its own `dbus-daemon`, socket in a folder of its own, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
        served: Vec<zbus::blocking::Connection>,
    }

    impl Bus {
        fn start(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("bar-bluez-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let config = dir.join("bus.conf");
            std::fs::write(
                &config,
                format!(
                    "<busconfig><type>system</type><listen>unix:dir={}</listen>\
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
                served: Vec::new(),
            }
        }

        /// The bar's side, with a short time limit: a device that does not answer fails fast.
        fn connect(&self) -> zbus::Connection {
            zbus::block_on(async {
                zbus::connection::Builder::address(self.address.as_str())
                    .unwrap()
                    .method_timeout(Duration::from_millis(800))
                    .build()
                    .await
                    .unwrap()
            })
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            self.served.clear();
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// What the fake BlueZ holds and was asked, shared with the test.
    #[derive(Debug, Default)]
    struct State {
        /// The agent registered: its owner on the bus and its path.
        agent: Option<(String, OwnedObjectPath)>,
        unregistered: usize,
        scanning: Vec<bool>,
        removed: Vec<String>,
    }

    type Shared = Arc<Mutex<State>>;

    fn lock(state: &Shared) -> MutexGuard<'_, State> {
        state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    struct AgentManager(Shared);

    #[zbus::interface(name = "org.bluez.AgentManager1")]
    impl AgentManager {
        fn register_agent(
            &self,
            agent: OwnedObjectPath,
            _capability: String,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) {
            let owner = header.sender().map(|s| s.to_string()).unwrap_or_default();
            lock(&self.0).agent = Some((owner, agent));
        }

        fn unregister_agent(&self, _agent: OwnedObjectPath) {
            let mut state = lock(&self.0);
            state.agent = None;
            state.unregistered += 1;
        }
    }

    struct Adapter(Shared);

    #[zbus::interface(name = "org.bluez.Adapter1")]
    impl Adapter {
        #[zbus(property)]
        fn powered(&self) -> bool {
            true
        }

        fn start_discovery(&self) {
            lock(&self.0).scanning.push(true);
        }

        fn stop_discovery(&self) {
            lock(&self.0).scanning.push(false);
        }

        async fn remove_device(
            &self,
            device: OwnedObjectPath,
            #[zbus(object_server)] server: &zbus::ObjectServer,
        ) -> fdo::Result<()> {
            let _ = server.remove::<Gadget, _>(device.as_str()).await;
            lock(&self.0).removed.push(device.to_string());
            Ok(())
        }
    }

    /// A fictitious device. `answers`: whether it ever answers `Pair`.
    struct Gadget {
        state: Shared,
        name: &'static str,
        icon: &'static str,
        paired: bool,
        connected: bool,
        trusted: bool,
        answers: bool,
    }

    impl Gadget {
        fn new(state: &Shared, name: &'static str, icon: &'static str, paired: bool) -> Self {
            Self {
                state: Arc::clone(state),
                name,
                icon,
                paired,
                connected: false,
                trusted: paired,
                answers: true,
            }
        }
    }

    #[zbus::interface(name = "org.bluez.Device1")]
    impl Gadget {
        #[zbus(property)]
        fn name(&self) -> String {
            self.name.to_owned()
        }

        #[zbus(property)]
        fn icon(&self) -> String {
            self.icon.to_owned()
        }

        #[zbus(property)]
        fn paired(&self) -> bool {
            self.paired
        }

        #[zbus(property)]
        fn connected(&self) -> bool {
            self.connected
        }

        #[zbus(property)]
        fn trusted(&self) -> bool {
            self.trusted
        }

        #[zbus(property)]
        fn set_trusted(&mut self, trusted: bool) {
            self.trusted = trusted;
        }

        async fn connect(&mut self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
            self.connected = true;
            let _ = self.connected_changed(&emitter).await;
        }

        async fn disconnect(&mut self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) {
            self.connected = false;
            let _ = self.connected_changed(&emitter).await;
        }

        /// Asks the registered agent to confirm 123456, as a device with a screen does.
        async fn pair(
            &mut self,
            #[zbus(connection)] connection: &zbus::Connection,
            #[zbus(header)] header: zbus::message::Header<'_>,
        ) -> fdo::Result<()> {
            if !self.answers {
                iced::futures::future::pending::<()>().await;
            }
            let device = header.path().map(|p| p.to_owned()).unwrap();
            let agent = lock(&self.state).agent.clone();
            let Some((owner, path)) = agent else {
                return Err(fdo::Error::Failed(String::from("no agent")));
            };
            let confirmed = connection
                .call_method(
                    Some(owner.as_str()),
                    path.as_str(),
                    Some("org.bluez.Agent1"),
                    "RequestConfirmation",
                    &(device, 123_456u32),
                )
                .await;
            match confirmed {
                Ok(_) => {
                    self.paired = true;
                    Ok(())
                }
                Err(error) => Err(fdo::Error::AccessDenied(error.to_string())),
            }
        }
    }

    /// A device found without a name: BlueZ only gives its address as its alias.
    struct Nameless;

    #[zbus::interface(name = "org.bluez.Device1")]
    impl Nameless {
        #[zbus(property)]
        fn alias(&self) -> String {
            String::from("00-11-22-33-44-55")
        }

        #[zbus(property)]
        fn paired(&self) -> bool {
            false
        }
    }

    struct Battery(u8);

    #[zbus::interface(name = "org.bluez.Battery1")]
    impl Battery {
        #[zbus(property)]
        fn percentage(&self) -> u8 {
            self.0
        }
    }

    const ADAPTER: &str = "/org/bluez/hci0";
    const HEADSET: &str = "/org/bluez/hci0/dev_01";
    const MOUSE: &str = "/org/bluez/hci0/dev_02";
    const KEYBOARD: &str = "/org/bluez/hci0/dev_03";
    const WATCH: &str = "/org/bluez/hci0/dev_04";
    const NAMELESS: &str = "/org/bluez/hci0/dev_05";

    /// BlueZ with one adapter: « Casque » paired and connected at 80 %, « Souris » paired,
    /// « Clavier » and « Montre » found (the watch never answers), and an unnamed device.
    fn bluez(bus: &mut Bus, state: &Shared) {
        let mut headset = Gadget::new(state, "Casque", "audio-headset", true);
        headset.connected = true;
        let mut watch = Gadget::new(state, "Montre", "", false);
        watch.answers = false;
        let connection = zbus::blocking::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .serve_at("/", zbus::fdo::ObjectManager)
            .unwrap()
            .serve_at("/org/bluez", AgentManager(Arc::clone(state)))
            .unwrap()
            .serve_at(ADAPTER, Adapter(Arc::clone(state)))
            .unwrap()
            .serve_at(HEADSET, headset)
            .unwrap()
            .serve_at(HEADSET, Battery(80))
            .unwrap()
            .serve_at(MOUSE, Gadget::new(state, "Souris", "input-mouse", true))
            .unwrap()
            .serve_at(
                KEYBOARD,
                Gadget::new(state, "Clavier", "input-keyboard", false),
            )
            .unwrap()
            .serve_at(WATCH, watch)
            .unwrap()
            .serve_at(NAMELESS, Nameless)
            .unwrap()
            .build()
            .unwrap();
        connection.request_name(bluetooth::SERVICE).unwrap();
        bus.served.push(connection);
    }

    fn read(client: &zbus::Connection) -> bluetooth::Bluetooth {
        zbus::block_on(bluetooth::read(client)).unwrap()
    }

    fn device<'a>(read: &'a bluetooth::Bluetooth, path: &str) -> &'a bluetooth::Device {
        read.devices.iter().find(|d| d.path == path).unwrap()
    }

    /// Pairs `path`, answering each question with `yes`. Returns the questions and the end.
    fn pair(client: &zbus::Connection, path: &str, yes: bool) -> (Vec<Asked>, Result<(), Problem>) {
        zbus::block_on(async {
            let mut events = bluetooth::pair(client.clone(), path.to_owned());
            let mut asked = Vec::new();
            while let Some(event) = events.next().await {
                match event {
                    Pairing::Ask(question, reply) => {
                        asked.push(question);
                        reply.send(yes);
                    }
                    Pairing::Canceled => {}
                    Pairing::Done(result) => return (asked, result),
                }
            }
            panic!("the pairing ended without its result");
        })
    }

    #[test]
    fn the_devices_are_listed_paired_first_with_their_kind_and_battery() {
        let mut bus = Bus::start("list");
        let state = Shared::default();
        bluez(&mut bus, &state);
        let read = read(&bus.connect());
        let names: Vec<_> = read.devices.iter().map(|d| d.name.as_str()).collect();
        // Connected, then paired, then found; the unnamed device is not shown.
        assert_eq!(names, ["Casque", "Souris", "Clavier", "Montre"]);
        let headset = device(&read, HEADSET);
        assert_eq!((headset.kind, headset.connected), (Kind::Audio, true));
        assert_eq!(headset.battery, Some(80));
        let mouse = device(&read, MOUSE);
        assert_eq!(
            (mouse.kind, mouse.paired, mouse.battery),
            (Kind::Mouse, true, None)
        );
        assert!(!device(&read, KEYBOARD).paired);
        assert_eq!(read.connected, 1);
    }

    #[test]
    fn a_click_connects_or_disconnects_a_paired_device_and_a_scan_starts_and_stops() {
        let mut bus = Bus::start("connect");
        let state = Shared::default();
        bluez(&mut bus, &state);
        let client = bus.connect();
        zbus::block_on(bluetooth::connect(&client, MOUSE)).unwrap();
        zbus::block_on(bluetooth::disconnect(&client, HEADSET)).unwrap();
        let read = read(&client);
        assert!(device(&read, MOUSE).connected);
        assert!(!device(&read, HEADSET).connected);
        zbus::block_on(bluetooth::discover(&client, ADAPTER, true)).unwrap();
        zbus::block_on(bluetooth::discover(&client, ADAPTER, false)).unwrap();
        assert_eq!(lock(&state).scanning, [true, false]);
    }

    #[test]
    fn a_device_found_pairs_once_its_code_is_confirmed_then_is_trusted_and_connected() {
        let mut bus = Bus::start("pair");
        let state = Shared::default();
        bluez(&mut bus, &state);
        let client = bus.connect();
        // « Annuler »: not paired, the agent gone.
        let (asked, result) = pair(&client, KEYBOARD, false);
        assert_eq!(asked, [Asked::Confirm(String::from("123456"))]);
        assert!(result.is_err(), "{result:?}");
        assert!(!device(&read(&client), KEYBOARD).paired);
        assert!(lock(&state).agent.is_none());
        // « Confirmer »: paired, trusted, connected; the agent registered for this pairing only.
        let (asked, result) = pair(&client, KEYBOARD, true);
        assert_eq!(asked, [Asked::Confirm(String::from("123456"))]);
        assert_eq!(result, Ok(()));
        let read = read(&client);
        let keyboard = device(&read, KEYBOARD);
        assert!(keyboard.paired && keyboard.connected);
        assert_eq!(lock(&state).unregistered, 2);
        assert!(lock(&state).agent.is_none());
        let trusted: bool = zbus::block_on(bar::system::get(
            &client,
            bluetooth::SERVICE,
            KEYBOARD,
            bluetooth::DEVICE,
            "Trusted",
        ))
        .unwrap();
        assert!(trusted);
    }

    #[test]
    fn a_device_that_does_not_answer_fails_within_the_time_limit() {
        let mut bus = Bus::start("silent");
        let state = Shared::default();
        bluez(&mut bus, &state);
        let start = Instant::now();
        let (asked, result) = pair(&bus.connect(), WATCH, true);
        assert!(asked.is_empty());
        assert!(result.is_err(), "{result:?}");
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(
            lock(&state).unregistered,
            1,
            "the agent goes with the failure"
        );
    }

    #[test]
    fn a_forgotten_device_leaves_the_list() {
        let mut bus = Bus::start("forget");
        let state = Shared::default();
        bluez(&mut bus, &state);
        let client = bus.connect();
        zbus::block_on(bluetooth::forget(&client, ADAPTER, MOUSE)).unwrap();
        assert_eq!(lock(&state).removed, [MOUSE]);
        let read = read(&client);
        assert!(read.devices.iter().all(|d| d.path != MOUSE));
        let path = ObjectPath::try_from(HEADSET).unwrap();
        assert_eq!(path.as_str(), HEADSET);
    }

    #[test]
    fn without_bluez_the_list_is_unavailable() {
        let bus = Bus::start("absent");
        let read = zbus::block_on(bluetooth::read(&bus.connect()));
        assert!(matches!(read, Err(Problem::Absent(_))), "{read:?}");
        let result = zbus::block_on(bluetooth::connect(&bus.connect(), HEADSET));
        assert!(matches!(result, Err(Problem::Absent(_))), "{result:?}");
    }
}
