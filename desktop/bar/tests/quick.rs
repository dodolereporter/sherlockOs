//! The quick settings against fake system services on a private bus started by the test (its
//! own `dbus-daemon`, never the machine's system bus): NetworkManager, BlueZ, logind and
//! power-profiles-daemon, each able to refuse. The tiles read them, the actions reach them, an
//! absent or refusing service gives its error, and their signals reach the bar. Sound goes
//! through a fake `pactl`.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::Duration;

    use bar::quick::{self, Service};
    use bar::system::audio::Pactl;
    use bar::system::network::{self, Security};
    use bar::system::power::{self, Profile};
    use bar::system::{Problem, backlight, bluetooth};
    use iced::futures::StreamExt;
    use zbus::fdo;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    /// A private bus: its own `dbus-daemon`, socket in a folder of its own, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
        /// The fake services' connections, open for the test.
        served: Vec<zbus::blocking::Connection>,
    }

    impl Bus {
        fn start(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("bar-quick-{name}-{}", std::process::id()));
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

        fn connect(&self) -> zbus::Connection {
            zbus::block_on(async {
                zbus::connection::Builder::address(self.address.as_str())
                    .unwrap()
                    .build()
                    .await
                    .unwrap()
            })
        }

        /// A fake service: its objects served, then its name taken.
        fn service(
            &mut self,
            name: &str,
            serve: impl FnOnce(
                zbus::blocking::connection::Builder<'static>,
            ) -> zbus::blocking::connection::Builder<'static>,
        ) {
            let builder =
                zbus::blocking::connection::Builder::address(self.address.as_str()).unwrap();
            let connection = serve(builder).build().unwrap();
            connection.request_name(name.to_owned()).unwrap();
            self.served.push(connection);
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

    /// What the fake services hold and were asked, shared with the test.
    #[derive(Debug, Default)]
    struct State {
        refuse: bool,
        wifi: bool,
        activated: Vec<(String, String)>,
        added: Vec<HashMap<String, HashMap<String, OwnedValue>>>,
        powered: bool,
        brightness: Vec<(String, String, u32)>,
        profile: String,
    }

    type Shared = Arc<Mutex<State>>;

    fn lock(state: &Shared) -> MutexGuard<'_, State> {
        state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn denied() -> fdo::Error {
        fdo::Error::AccessDenied(String::from("not authorized"))
    }

    fn path(p: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(p).unwrap()
    }

    // NetworkManager: a manager, one Ethernet device then one Wi-Fi device, three access
    // points (two of the same network), one saved connection.

    struct Manager(Shared);

    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl Manager {
        #[zbus(property)]
        fn wireless_enabled(&self) -> bool {
            lock(&self.0).wifi
        }

        #[zbus(property)]
        fn set_wireless_enabled(&mut self, on: bool) -> fdo::Result<()> {
            let mut state = lock(&self.0);
            if state.refuse {
                return Err(denied());
            }
            state.wifi = on;
            Ok(())
        }

        #[zbus(property)]
        fn wireless_hardware_enabled(&self) -> bool {
            true
        }

        fn get_devices(&self) -> Vec<OwnedObjectPath> {
            vec![path("/nm/Devices/1"), path("/nm/Devices/2")]
        }

        fn activate_connection(
            &self,
            connection: OwnedObjectPath,
            _device: OwnedObjectPath,
            point: OwnedObjectPath,
        ) -> fdo::Result<OwnedObjectPath> {
            let mut state = lock(&self.0);
            if state.refuse {
                return Err(denied());
            }
            let pair = (connection.to_string(), point.to_string());
            state.activated.push(pair);
            Ok(path("/nm/ActiveConnection/1"))
        }

        fn add_and_activate_connection(
            &self,
            settings: HashMap<String, HashMap<String, OwnedValue>>,
            _device: OwnedObjectPath,
            _point: OwnedObjectPath,
        ) -> fdo::Result<(OwnedObjectPath, OwnedObjectPath)> {
            let mut state = lock(&self.0);
            if state.refuse {
                return Err(denied());
            }
            state.added.push(settings);
            Ok((path("/nm/Settings/9"), path("/nm/ActiveConnection/2")))
        }
    }

    struct Device(u32);

    #[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
    impl Device {
        #[zbus(property)]
        fn device_type(&self) -> u32 {
            self.0
        }

        #[zbus(signal)]
        async fn state_changed(
            emitter: &SignalEmitter<'_>,
            new: u32,
            old: u32,
            reason: u32,
        ) -> zbus::Result<()>;
    }

    struct Wireless;

    #[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
    impl Wireless {
        #[zbus(property)]
        fn active_access_point(&self) -> OwnedObjectPath {
            path("/nm/AccessPoint/1")
        }

        #[zbus(property)]
        fn access_points(&self) -> Vec<OwnedObjectPath> {
            vec![
                path("/nm/AccessPoint/1"),
                path("/nm/AccessPoint/2"),
                path("/nm/AccessPoint/3"),
            ]
        }

        fn request_scan(&self, _options: HashMap<String, OwnedValue>) {}

        #[zbus(signal)]
        async fn access_point_added(
            emitter: &SignalEmitter<'_>,
            point: OwnedObjectPath,
        ) -> zbus::Result<()>;
    }

    struct Point {
        ssid: &'static str,
        strength: u8,
        rsn: u32,
    }

    #[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
    impl Point {
        #[zbus(property)]
        fn ssid(&self) -> Vec<u8> {
            self.ssid.as_bytes().to_vec()
        }

        #[zbus(property)]
        fn strength(&self) -> u8 {
            self.strength
        }

        #[zbus(property)]
        fn flags(&self) -> u32 {
            u32::from(self.rsn != 0)
        }

        #[zbus(property)]
        fn wpa_flags(&self) -> u32 {
            0
        }

        #[zbus(property)]
        fn rsn_flags(&self) -> u32 {
            self.rsn
        }
    }

    struct Settings;

    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
    impl Settings {
        fn list_connections(&self) -> Vec<OwnedObjectPath> {
            vec![path("/nm/Settings/1")]
        }
    }

    struct Saved;

    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
    impl Saved {
        fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
            let ssid = OwnedValue::try_from(Value::from(b"Maison".to_vec())).unwrap();
            HashMap::from([(
                String::from("802-11-wireless"),
                HashMap::from([(String::from("ssid"), ssid)]),
            )])
        }
    }

    fn serve_network(bus: &mut Bus, state: &Shared) {
        let state = Arc::clone(state);
        let point = |ssid, strength, rsn| Point {
            ssid,
            strength,
            rsn,
        };
        bus.service(network::SERVICE, move |builder| {
            builder
                .serve_at(network::PATH, Manager(state))
                .unwrap()
                .serve_at("/nm/Devices/1", Device(1))
                .unwrap()
                .serve_at("/nm/Devices/2", Device(2))
                .unwrap()
                .serve_at("/nm/Devices/2", Wireless)
                .unwrap()
                .serve_at("/nm/AccessPoint/1", point("Maison", 62, 0x100))
                .unwrap()
                .serve_at("/nm/AccessPoint/2", point("Café", 80, 0))
                .unwrap()
                .serve_at("/nm/AccessPoint/3", point("Maison", 90, 0x100))
                .unwrap()
                .serve_at(network::SETTINGS_PATH, Settings)
                .unwrap()
                .serve_at("/nm/Settings/1", Saved)
                .unwrap()
        });
    }

    #[test]
    fn the_wifi_tile_reads_switches_and_joins() {
        let mut bus = Bus::start("network");
        let state = Shared::default();
        lock(&state).wifi = true;
        serve_network(&mut bus, &state);
        let client = bus.connect();

        let wifi = zbus::block_on(network::read(&client)).unwrap();
        assert!(wifi.enabled && !wifi.hardware_off);
        assert_eq!(wifi.device.as_deref(), Some("/nm/Devices/2"));
        // One line per network: « Maison » on its access point in use, above the stronger
        // « Café ».
        let names: Vec<_> = wifi.networks.iter().map(|n| n.ssid.as_str()).collect();
        assert_eq!(names, ["Maison", "Café"]);
        let home = wifi.connected().unwrap().clone();
        assert_eq!(home.path, "/nm/AccessPoint/1");
        assert_eq!(home.security, Security::Psk);
        assert_eq!(wifi.networks[1].security, Security::Open);

        zbus::block_on(network::enable(&client, false)).unwrap();
        assert!(!lock(&state).wifi);
        let off = zbus::block_on(network::read(&client)).unwrap();
        assert!(!off.enabled && off.networks.is_empty());
        zbus::block_on(network::enable(&client, true)).unwrap();
        let device = wifi.device.clone().unwrap();
        zbus::block_on(network::scan(&client, &device)).unwrap();

        // A saved network is joined with its connection, without a password.
        zbus::block_on(network::connect(&client, &device, &home, None)).unwrap();
        let activated = lock(&state).activated.clone();
        let expected = (
            String::from("/nm/Settings/1"),
            String::from("/nm/AccessPoint/1"),
        );
        assert_eq!(activated, [expected]);
        // A new protected one needs the password typed; without it, nothing is sent.
        let mut other = home.clone();
        other.ssid = String::from("Voisin");
        assert!(zbus::block_on(network::connect(&client, &device, &other, None)).is_err());
        assert!(lock(&state).added.is_empty());
        let joined = network::connect(&client, &device, &other, Some("s3cret"));
        zbus::block_on(joined).unwrap();
        let added = lock(&state).added.clone();
        assert_eq!(added.len(), 1);
        let security = &added[0]["802-11-wireless-security"];
        let text = |key: &str| String::try_from(security[key].try_clone().unwrap()).unwrap();
        assert_eq!(text("psk"), "s3cret");
        assert_eq!(text("key-mgmt"), "wpa-psk");
    }

    #[test]
    fn a_refusing_networkmanager_is_reported() {
        let mut bus = Bus::start("network-refused");
        let state = Shared::default();
        lock(&state).refuse = true;
        serve_network(&mut bus, &state);
        let client = bus.connect();
        let refused = zbus::block_on(network::enable(&client, true));
        assert!(matches!(refused, Err(Problem::Refused(_))), "{refused:?}");
        let wifi = zbus::block_on(network::read(&client)).unwrap();
        assert!(!wifi.enabled && wifi.connected().is_none());
    }

    // BlueZ: one adapter, two devices, one of them connected.

    struct Adapter(Shared);

    #[zbus::interface(name = "org.bluez.Adapter1")]
    impl Adapter {
        #[zbus(property)]
        fn powered(&self) -> bool {
            lock(&self.0).powered
        }

        #[zbus(property)]
        fn set_powered(&mut self, on: bool) -> fdo::Result<()> {
            let mut state = lock(&self.0);
            if state.refuse {
                return Err(denied());
            }
            state.powered = on;
            Ok(())
        }
    }

    struct Gadget(bool);

    #[zbus::interface(name = "org.bluez.Device1")]
    impl Gadget {
        #[zbus(property)]
        fn name(&self) -> String {
            String::from("Casque")
        }

        #[zbus(property)]
        fn paired(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn connected(&self) -> bool {
            self.0
        }
    }

    #[test]
    fn the_bluetooth_tile_reads_and_switches() {
        let mut bus = Bus::start("bluez");
        let state = Shared::default();
        let shared = Arc::clone(&state);
        bus.service(bluetooth::SERVICE, move |builder| {
            builder
                .serve_at("/", zbus::fdo::ObjectManager)
                .unwrap()
                .serve_at("/org/bluez/hci0", Adapter(shared))
                .unwrap()
                .serve_at("/org/bluez/hci0/dev_01", Gadget(true))
                .unwrap()
                .serve_at("/org/bluez/hci0/dev_02", Gadget(false))
                .unwrap()
        });
        let client = bus.connect();
        let read = zbus::block_on(bluetooth::read(&client)).unwrap();
        assert_eq!(read.adapter, "/org/bluez/hci0");
        assert!(!read.powered);
        assert_eq!(read.connected, 1);
        zbus::block_on(bluetooth::power(&client, &read.adapter, true)).unwrap();
        assert!(lock(&state).powered);
        lock(&state).refuse = true;
        let refused = zbus::block_on(bluetooth::power(&client, &read.adapter, false));
        assert!(matches!(refused, Err(Problem::Refused(_))), "{refused:?}");
        assert!(lock(&state).powered);
    }

    #[test]
    fn bluez_without_an_adapter_is_absent() {
        let mut bus = Bus::start("bluez-empty");
        bus.service(bluetooth::SERVICE, |builder| {
            builder.serve_at("/", zbus::fdo::ObjectManager).unwrap()
        });
        let client = bus.connect();
        let read = zbus::block_on(bluetooth::read(&client));
        assert!(matches!(read, Err(Problem::Absent(_))), "{read:?}");
    }

    // logind: the caller's session, whose SetBrightness writes the fake sysfs file.

    struct Session {
        state: Shared,
        sysfs: PathBuf,
    }

    #[zbus::interface(name = "org.freedesktop.login1.Session")]
    impl Session {
        fn set_brightness(&self, subsystem: String, name: String, value: u32) -> fdo::Result<()> {
            let mut state = lock(&self.state);
            if state.refuse {
                return Err(denied());
            }
            let file = self.sysfs.join(&name).join("brightness");
            std::fs::write(file, value.to_string())
                .map_err(|e| fdo::Error::Failed(e.to_string()))?;
            state.brightness.push((subsystem, name, value));
            Ok(())
        }
    }

    fn backlight(root: &Path, name: &str, kind: &str, brightness: u32, max: u32) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("type"), kind).unwrap();
        std::fs::write(dir.join("brightness"), brightness.to_string()).unwrap();
        std::fs::write(dir.join("max_brightness"), max.to_string()).unwrap();
    }

    #[test]
    fn the_brightness_slider_goes_through_logind() {
        let mut bus = Bus::start("logind");
        let sysfs = bus.dir.join("backlight");
        backlight(&sysfs, "acpi_video0", "firmware", 8, 10);
        backlight(&sysfs, "amdgpu_bl1", "raw", 204, 255);
        backlight(&sysfs, "b_platform", "platform", 1, 2);
        let state = Shared::default();
        let session = Session {
            state: Arc::clone(&state),
            sysfs: sysfs.clone(),
        };
        bus.service(backlight::SERVICE, move |builder| {
            builder.serve_at(backlight::SESSION, session).unwrap()
        });
        let client = bus.connect();
        // The firmware backlight first, then the platform one, then a raw one.
        let light = backlight::find(&sysfs).unwrap();
        assert_eq!((light.name.as_str(), light.percent()), ("acpi_video0", 80));
        std::fs::remove_dir_all(sysfs.join("acpi_video0")).unwrap();
        assert_eq!(backlight::find(&sysfs).unwrap().name, "b_platform");
        std::fs::remove_dir_all(sysfs.join("b_platform")).unwrap();
        let light = backlight::find(&sysfs).unwrap();
        assert_eq!((light.name.as_str(), light.percent()), ("amdgpu_bl1", 80));

        zbus::block_on(backlight::set(&client, &light, 50)).unwrap();
        let expected = (String::from("backlight"), String::from("amdgpu_bl1"), 128);
        assert_eq!(lock(&state).brightness, [expected]);
        assert_eq!(backlight::find(&sysfs).unwrap().percent(), 50);
        // Never black: 0 % gives the step of 1 %.
        zbus::block_on(backlight::set(&client, &light, 0)).unwrap();
        assert_eq!(lock(&state).brightness[1].2, 3);

        lock(&state).refuse = true;
        let refused = zbus::block_on(backlight::set(&client, &light, 90));
        assert!(matches!(refused, Err(Problem::Refused(_))), "{refused:?}");
        // No backlight at all: a desktop screen.
        let empty = bus.dir.join("none");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(matches!(backlight::find(&empty), Err(Problem::Absent(_))));
    }

    // power-profiles-daemon, under its first name only: the bar falls back to it.

    struct Profiles(Shared);

    #[zbus::interface(name = "net.hadess.PowerProfiles")]
    impl Profiles {
        #[zbus(property)]
        fn active_profile(&self) -> String {
            lock(&self.0).profile.clone()
        }

        #[zbus(property)]
        fn set_active_profile(&mut self, profile: String) -> fdo::Result<()> {
            let mut state = lock(&self.0);
            if state.refuse {
                return Err(denied());
            }
            state.profile = profile;
            Ok(())
        }

        #[zbus(property)]
        fn profiles(&self) -> Vec<HashMap<String, OwnedValue>> {
            ["power-saver", "balanced"]
                .map(|p| {
                    let value = OwnedValue::try_from(Value::from(p)).unwrap();
                    HashMap::from([(String::from("Profile"), value)])
                })
                .to_vec()
        }
    }

    #[test]
    fn the_energy_profiles_follow_the_daemon_under_either_name() {
        let mut bus = Bus::start("profiles");
        let state = Shared::default();
        lock(&state).profile = String::from("balanced");
        let shared = Arc::clone(&state);
        bus.service("net.hadess.PowerProfiles", move |builder| {
            builder
                .serve_at("/net/hadess/PowerProfiles", Profiles(shared))
                .unwrap()
        });
        let client = bus.connect();
        let read = zbus::block_on(power::read(&client)).unwrap();
        assert_eq!(read.active, Some(Profile::Balanced));
        // This machine has no performance profile.
        assert_eq!(read.available, [Profile::Saver, Profile::Balanced]);
        zbus::block_on(power::choose(&client, &read, Profile::Saver)).unwrap();
        assert_eq!(lock(&state).profile, "power-saver");
        lock(&state).refuse = true;
        let refused = zbus::block_on(power::choose(&client, &read, Profile::Balanced));
        assert!(matches!(refused, Err(Problem::Refused(_))), "{refused:?}");
    }

    #[test]
    fn without_the_services_every_tile_says_so() {
        let bus = Bus::start("empty");
        let client = bus.connect();
        let absent = |result: Result<(), Problem>| matches!(result, Err(Problem::Absent(_)));
        assert!(absent(zbus::block_on(network::read(&client)).map(|_| ())));
        assert!(absent(zbus::block_on(network::enable(&client, true))));
        assert!(absent(zbus::block_on(bluetooth::read(&client)).map(|_| ())));
        assert!(absent(zbus::block_on(power::read(&client)).map(|_| ())));
        let light = backlight::Backlight {
            name: String::from("amdgpu_bl1"),
            brightness: 1,
            max: 255,
        };
        assert!(absent(zbus::block_on(backlight::set(&client, &light, 40))));
    }

    #[test]
    fn the_signals_of_the_services_reach_the_bar() {
        let mut bus = Bus::start("signals");
        let state = Shared::default();
        serve_network(&mut bus, &state);
        let client = bus.connect();
        let mut changes = zbus::block_on(quick::changes_on(&client)).unwrap();
        let (sender, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            zbus::block_on(async move {
                while let Some(service) = changes.next().await {
                    if sender.send(service).is_err() {
                        return;
                    }
                }
            });
        });
        // A device of NetworkManager changes its state: the Wi-Fi is read again.
        let emitter = &bus.served[0];
        let object = emitter
            .object_server()
            .interface::<_, Device>("/nm/Devices/2")
            .unwrap();
        let emitted = Device::state_changed(object.signal_emitter(), 100, 30, 0);
        zbus::block_on(emitted).unwrap();
        let got = received.recv_timeout(Duration::from_secs(5));
        assert_eq!(got.ok(), Some(Service::Wifi));
        // A scan finds an access point: the list is read again.
        let object = emitter
            .object_server()
            .interface::<_, Wireless>("/nm/Devices/2")
            .unwrap();
        let point = path("/nm/AccessPoint/4");
        let emitted = Wireless::access_point_added(object.signal_emitter(), point);
        zbus::block_on(emitted).unwrap();
        let got = received.recv_timeout(Duration::from_secs(5));
        assert_eq!(got.ok(), Some(Service::Wifi));
    }

    /// A NetworkManager that takes the name and never answers.
    struct Hung;

    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl Hung {
        #[zbus(property)]
        async fn wireless_enabled(&self) -> bool {
            std::future::pending::<()>().await;
            false
        }
    }

    #[test]
    fn a_service_that_hangs_ends_unavailable() {
        let mut bus = Bus::start("hung");
        bus.service(network::SERVICE, |builder| {
            builder.serve_at(network::PATH, Hung).unwrap()
        });
        let client = zbus::block_on(async {
            zbus::connection::Builder::address(bus.address.as_str())
                .unwrap()
                .method_timeout(Duration::from_millis(500))
                .build()
                .await
                .unwrap()
        });
        let start = std::time::Instant::now();
        let read = zbus::block_on(network::read(&client));
        assert!(matches!(read, Err(Problem::Absent(_))), "{read:?}");
        assert!(start.elapsed() < Duration::from_secs(5));
        // What the tile then shows: unavailable, not « … ».
        let load = quick::Load::from_result(read);
        assert!(matches!(load, quick::Load::Failed(Problem::Absent(_))));
    }

    /// A fake `pactl` that answers from the fixtures and writes each call to a log.
    fn fake_pactl(dir: &Path) -> (Pactl, PathBuf) {
        let log = dir.join("pactl.log");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../system-services/fixtures");
        let script = dir.join("pactl");
        let body = format!(
            "#!/bin/sh\necho \"$@\" >> '{log}'\ncase \"$1\" in\n\
             get-sink-volume) grep '^Volume' '{f}/pactl-55.txt' ;;\n\
             get-sink-mute) grep '^Mute' '{f}/pactl-55.txt' ;;\n\
             get-default-sink) echo alsa_output.pci-0000_05_00.6.analog-stereo ;;\n\
             get-default-source) echo alsa_input.pci-0000_05_00.6.analog-stereo ;;\n\
             get-source-volume) grep '^Volume' '{f}/pactl-55.txt' ;;\n\
             get-source-mute) grep '^Mute' '{f}/pactl-muted.txt' ;;\n\
             list) [ \"$2\" = sources ] && cat '{f}/pactl-sources.txt' || cat '{f}/pactl-sinks.txt' ;;\n\
             set-sink-volume) [ \"$3\" = 99% ] && exit 1 ;;\nesac\nexit 0\n",
            log = log.display(),
            f = fixtures.display(),
        );
        std::fs::write(&script, body).unwrap();
        let chmod = Command::new("chmod")
            .arg("+x")
            .arg(&script)
            .status()
            .unwrap();
        assert!(chmod.success());
        (
            Pactl {
                program: script.into_os_string(),
            },
            log,
        )
    }

    #[test]
    fn a_sound_server_that_hangs_is_given_up_after_the_time_limit() {
        let dir = std::env::temp_dir().join(format!("bar-pactl-hung-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("pactl");
        std::fs::write(&script, "#!/bin/sh\nexec sleep 30\n").unwrap();
        let chmod = Command::new("chmod")
            .arg("+x")
            .arg(&script)
            .status()
            .unwrap();
        assert!(chmod.success());
        let pactl = Pactl {
            program: script.into_os_string(),
        };
        let start = std::time::Instant::now();
        assert!(matches!(pactl.read(), Err(Problem::Absent(_))));
        let waited = start.elapsed();
        assert!(waited >= bar::system::audio::TIMEOUT, "{waited:?}");
        assert!(waited < bar::system::audio::TIMEOUT * 2, "{waited:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_volume_goes_through_pactl() {
        let dir = std::env::temp_dir().join(format!("bar-pactl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (pactl, log) = fake_pactl(&dir);
        let output = pactl.read().unwrap();
        assert_eq!((output.sound.volume, output.sound.muted), (55, false));
        assert_eq!(output.name, "Haut-parleurs intégrés");
        pactl.set_volume(62).unwrap();
        pactl.set_mute(true).unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(
            calls.contains("set-sink-volume @DEFAULT_SINK@ 62%"),
            "{calls}"
        );
        assert!(calls.contains("set-sink-mute @DEFAULT_SINK@ 1"), "{calls}");
        // The outputs and the inputs (D-20g): the monitor of the speakers is no input.
        let devices = quick::read_devices(&pactl).unwrap();
        let names: Vec<_> = devices
            .inputs
            .iter()
            .map(|e| e.description.as_str())
            .collect();
        assert_eq!(names, ["Microphone intégré", "Casque"]);
        assert_eq!(devices.outputs.len(), 2);
        assert_eq!(devices.input, "alsa_input.pci-0000_05_00.6.analog-stereo");
        let microphone = devices.microphone.unwrap();
        assert_eq!((microphone.volume, microphone.muted), (55, true));
        pactl
            .set_default(
                bar::system::audio::Device::Microphone,
                "bluez_input.headset",
            )
            .unwrap();
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(
            calls.contains("set-default-source bluez_input.headset"),
            "{calls}"
        );
        // A refusal of the sound server, and no `pactl` at all.
        assert!(matches!(pactl.set_volume(99), Err(Problem::Failed(_))));
        let missing = Pactl {
            program: dir.join("absent").into_os_string(),
        };
        assert!(matches!(missing.read(), Err(Problem::Absent(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
