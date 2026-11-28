//! The settings server (A-17) as a real process, on a configuration folder of the test's own, a
//! fake `pactl` and a fake NetworkManager on a private bus (its own `dbus-daemon`): each
//! setting of Control Room read and changed by its functions, a value the file's parser refuses
//! never written, the sound set, Wi-Fi switched off, a network forgotten with the reinforced
//! consent; refused, nothing changes.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::json;
    use settings_files::{Paths, Settings};
    use sherlockd::broker::{Broker, Consent, Decision, Journal, Request};
    use sherlockd::mcp::{McpClient, McpError, McpServers, ServerConfig, ToolHost};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

    const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");
    const SAVED: &str = "/org/freedesktop/NetworkManager/Settings/1";

    /// A private bus: its own `dbus-daemon`, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        _served: Option<zbus::blocking::Connection>,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    fn bus(dir: &Path) -> Bus {
        let socket = dir.join("bus");
        std::fs::create_dir_all(&socket).unwrap();
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>system</type><listen>unix:dir={}</listen>\
                 <auth>EXTERNAL</auth><policy context=\"default\">\
                 <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                 <allow own=\"*\"/></policy></busconfig>",
                socket.display()
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
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Bus {
            daemon,
            address: address.trim().to_owned(),
            _served: None,
        }
    }

    /// What the fake NetworkManager was asked: Wi-Fi switched to, connections deleted.
    #[derive(Default)]
    struct Asked {
        wireless: Option<bool>,
        deleted: usize,
    }

    struct Manager(Arc<Mutex<Asked>>);

    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl Manager {
        #[zbus(property)]
        fn wireless_enabled(&self) -> bool {
            self.0.lock().unwrap().wireless.unwrap_or(true)
        }
        #[zbus(property)]
        fn set_wireless_enabled(&mut self, on: bool) {
            self.0.lock().unwrap().wireless = Some(on);
        }
    }

    struct NmSettings;

    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
    impl NmSettings {
        fn list_connections(&self) -> Vec<OwnedObjectPath> {
            vec![OwnedObjectPath::try_from(SAVED).unwrap()]
        }
    }

    struct Saved(Arc<Mutex<Asked>>);

    #[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
    impl Saved {
        fn get_settings(&self) -> HashMap<String, HashMap<String, OwnedValue>> {
            let ssid = OwnedValue::try_from(Value::from(b"Maison".to_vec())).unwrap();
            HashMap::from([(
                "802-11-wireless".to_owned(),
                HashMap::from([("ssid".to_owned(), ssid)]),
            )])
        }
        fn delete(&self) {
            self.0.lock().unwrap().deleted += 1;
        }
    }

    fn network_manager(bus: &mut Bus) -> Arc<Mutex<Asked>> {
        let asked = Arc::new(Mutex::new(Asked::default()));
        let served = zbus::blocking::connection::Builder::address(bus.address.as_str())
            .unwrap()
            .name("org.freedesktop.NetworkManager")
            .unwrap()
            .serve_at("/org/freedesktop/NetworkManager", Manager(asked.clone()))
            .unwrap()
            .serve_at("/org/freedesktop/NetworkManager/Settings", NmSettings)
            .unwrap()
            .serve_at(SAVED, Saved(asked.clone()))
            .unwrap()
            .build()
            .unwrap();
        bus._served = Some(served);
        asked
    }

    /// A fake `pactl`: it writes its arguments to `dir/pactl.log` and answers as pactl does.
    fn pactl(dir: &Path) -> PathBuf {
        let program = dir.join("pactl");
        std::fs::write(
            &program,
            format!(
                "#!/bin/sh\necho \"$@\" >> '{}'\ncase \"$1\" in\n\
                 get-sink-volume|get-source-volume) echo 'Volume: front-left: 32768 /  50% / -18.06 dB';;\n\
                 get-sink-mute|get-source-mute) echo 'Mute: no';;\n\
                 get-default-sink) echo 'alsa_output.speakers';;\n\
                 get-default-source) echo 'alsa_input.mic';;\n\
                 list) printf 'Sink #1\\n\\tName: alsa_output.speakers\\n\\tDescription: Haut-parleurs\\n\
                 Sink #2\\n\\tName: bluez_output.casque\\n\\tDescription: Casque\\n';;\n\
                 esac\n",
                dir.join("pactl.log").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(
            &program,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        program
    }

    struct Answers(Decision, Arc<Mutex<Vec<Request>>>);

    impl Consent for Answers {
        fn ask<'a>(
            &'a mut self,
            request: &'a Request,
        ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            self.1.lock().unwrap().push(request.clone());
            let decision = self.0;
            Box::pin(async move { decision })
        }
    }

    struct Setup {
        dir: PathBuf,
        _bus: Bus,
        network: Arc<Mutex<Asked>>,
        servers: McpServers,
        asked: Arc<Mutex<Vec<Request>>>,
    }

    async fn setup(name: &str, decision: Decision) -> Setup {
        let dir =
            std::env::temp_dir().join(format!("sherlockd-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let mut private = bus(&dir);
        let network = network_manager(&mut private);
        let program = pactl(&dir);
        let config_home = dir.join("config");
        let args = [
            "--settings-server",
            "--config-home",
            config_home.to_str().unwrap(),
            "--pactl",
            program.to_str().unwrap(),
            "--system-bus",
            private.address.as_str(),
        ];
        let config = ServerConfig::new("settings", EXE, &args)
            .timeout(Duration::from_secs(10))
            .ours();
        let journal = Journal::open(&dir.join("journal.jsonl")).unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let consent = Box::new(Answers(decision, asked.clone()));
        let mut servers = McpServers::with_broker(Broker::new(consent, journal));
        servers
            .add(McpClient::spawn(&config).await.unwrap())
            .await
            .unwrap();
        Setup {
            dir,
            _bus: private,
            network,
            servers,
            asked,
        }
    }

    /// The settings as Control Room reads them.
    fn control_room(dir: &Path) -> Settings {
        Settings::load(&Paths::under(&dir.join("config"), Vec::new())).0
    }

    #[tokio::test]
    async fn each_setting_is_changed_by_control_rooms_functions() {
        let mut s = setup("change", Decision::Granted).await;
        let read = s
            .servers
            .call("lire_reglages", json!({ "domaine": "pave_tactile" }))
            .await
            .unwrap();
        assert!(
            read.text.contains("tap_to_click") && read.text.contains("tap-to-click"),
            "{}",
            read.text
        );
        assert!(read.text.contains("jamais des consignes"));
        for (domain, key, value) in [
            ("pave_tactile", "tap-to-click", json!(false)),
            ("pave_tactile", "speed", json!(0.5)),
            ("apparence", "Theme", json!("light")),
            ("clavier", "repeat-rate", json!(30)),
            ("presse_papiers", "history", json!(false)),
            ("dock", "Position", json!("left")),
            ("verrouillage", "idle-minutes", json!(15)),
            ("ne_pas_deranger", "DoNotDisturb", json!(true)),
        ] {
            let arguments = json!({ "domaine": domain, "reglage": key, "valeur": value });
            let output = s.servers.call("changer_reglage", arguments).await.unwrap();
            assert!(!output.is_error, "{domain} {key}: {}", output.text);
        }
        let settings = control_room(&s.dir);
        assert!(!settings.touchpad.tap_to_click);
        assert!((settings.touchpad.speed - 0.5).abs() < 1e-9);
        assert_eq!(settings.appearance.mode.key(), "light");
        assert_eq!(settings.keyboard.repeat_rate, 30);
        assert!(!settings.clipboard.history);
        assert_eq!(settings.dock.position.key(), "left");
        assert_eq!(settings.lock.idle_minutes, 15);
        let dnd =
            std::fs::read_to_string(s.dir.join("config/notifications/notifications.conf")).unwrap();
        assert!(dnd.contains("DoNotDisturb=true"), "{dnd}");
        assert_eq!(s.asked.lock().unwrap().len(), 8, "one consent per change");

        // What the parser refuses, a path, an unknown key: nothing written.
        let before =
            std::fs::read_to_string(s.dir.join("config/touchpad.conf")).unwrap_or_default();
        for (domain, key, value) in [
            ("apparence", "Theme", json!("rose")),
            ("clavier", "repeat-rate", json!(9999)),
            ("pave_tactile", "speed", json!(7)),
            ("pave_tactile", "click", json!("fingers\nspeed = 1")),
            ("fond_ecran", "path", json!("/etc/shadow")),
            ("verrouillage", "background", json!("/home/x/.ssh/id_rsa")),
            ("pave_tactile", "inconnu", json!(true)),
        ] {
            let arguments = json!({ "domaine": domain, "reglage": key, "valeur": value });
            let output = s
                .servers
                .call("changer_reglage", arguments.clone())
                .await
                .unwrap();
            assert!(output.is_error, "{arguments}: {}", output.text);
        }
        assert_eq!(
            std::fs::read_to_string(s.dir.join("config/touchpad.conf")).unwrap_or_default(),
            before
        );
        assert_eq!(control_room(&s.dir).appearance.mode.key(), "light");

        // Never locking again, the clipboard kept on the disk: only with the reinforced consent
        // (the A-20 review).
        let asked = s.asked.lock().unwrap().len();
        for (domain, key, value) in [
            ("verrouillage", "idle-minutes", json!(0)),
            ("presse_papiers", "keep-on-disk", json!(true)),
        ] {
            let arguments = json!({ "domaine": domain, "reglage": key, "valeur": value });
            let plain = s
                .servers
                .call("changer_reglage", arguments.clone())
                .await
                .unwrap();
            assert!(plain.is_error && plain.text.contains("changer_reglage_de_securite"));
            let secure = s
                .servers
                .call("changer_reglage_de_securite", arguments)
                .await
                .unwrap();
            assert!(!secure.is_error, "{}", secure.text);
        }
        let settings = control_room(&s.dir);
        assert_eq!(settings.lock.idle_minutes, 0);
        assert!(settings.clipboard.keep_on_disk);
        let asked: Vec<bool> = s.asked.lock().unwrap()[asked..]
            .iter()
            .map(Request::reinforced)
            .collect();
        assert_eq!(asked, [false, true, false, true]);
        s.servers.shutdown().await;
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[tokio::test]
    async fn the_sound_wifi_and_a_network_forgotten_with_the_reinforced_consent() {
        let mut s = setup("devices", Decision::Granted).await;
        let sound = s
            .servers
            .call("lire_reglages", json!({ "domaine": "son" }))
            .await
            .unwrap();
        assert!(
            sound.text.contains("Casque") && sound.text.contains("50"),
            "{}",
            sound.text
        );
        let set = json!({ "appareil": "sortie", "volume": 40, "coupe": false });
        assert!(!s.servers.call("regler_son", set).await.unwrap().is_error);
        let choose = json!({ "appareil": "sortie", "nom": "Casque" });
        assert!(
            !s.servers
                .call("choisir_appareil_son", choose)
                .await
                .unwrap()
                .is_error
        );
        let unknown = json!({ "appareil": "sortie", "nom": "alsa; rm -rf ~" });
        assert!(
            s.servers
                .call("choisir_appareil_son", unknown)
                .await
                .unwrap()
                .is_error
        );
        let log = std::fs::read_to_string(s.dir.join("pactl.log")).unwrap();
        assert!(log.contains("set-sink-volume @DEFAULT_SINK@ 40%"), "{log}");
        assert!(
            log.contains("set-default-sink bluez_output.casque"),
            "{log}"
        );
        assert!(!log.contains("rm -rf"));

        let off = s
            .servers
            .call("activer_wifi", json!({ "actif": false }))
            .await
            .unwrap();
        assert!(!off.is_error, "{}", off.text);
        assert_eq!(s.network.lock().unwrap().wireless, Some(false));
        let gone = s
            .servers
            .call("oublier_reseau", json!({ "ssid": "Inconnu" }))
            .await
            .unwrap();
        assert!(
            gone.is_error && gone.text.contains("aucun réseau"),
            "{}",
            gone.text
        );
        let forgot = s
            .servers
            .call("oublier_reseau", json!({ "ssid": "Maison" }))
            .await
            .unwrap();
        assert!(!forgot.is_error, "{}", forgot.text);
        assert_eq!(s.network.lock().unwrap().deleted, 1);
        let asked = s.asked.lock().unwrap().clone();
        let reinforced: Vec<&str> = asked
            .iter()
            .filter(|r| r.reinforced())
            .map(|r| r.tool.as_str())
            .collect();
        assert_eq!(reinforced, ["oublier_reseau", "oublier_reseau"]);
        s.servers.shutdown().await;
        let _ = std::fs::remove_dir_all(&s.dir);
    }

    #[tokio::test]
    async fn refused_nothing_changes() {
        let mut s = setup("refused", Decision::Denied).await;
        let change = json!({ "domaine": "apparence", "reglage": "Theme", "valeur": "light" });
        for (tool, arguments) in [
            ("changer_reglage", change),
            ("activer_wifi", json!({ "actif": false })),
            ("oublier_reseau", json!({ "ssid": "Maison" })),
        ] {
            let refused = s.servers.call(tool, arguments).await;
            assert!(
                matches!(refused, Err(McpError::Refused { .. })),
                "{tool}: {refused:?}"
            );
        }
        assert_ne!(control_room(&s.dir).appearance.mode.key(), "light");
        assert!(!s.dir.join("config/appearance/appearance.conf").exists());
        let network = {
            let network = s.network.lock().unwrap();
            (network.wireless, network.deleted)
        };
        assert_eq!(network, (None, 0));
        s.servers.shutdown().await;
        let _ = std::fs::remove_dir_all(&s.dir);
    }
}
