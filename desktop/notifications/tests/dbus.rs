//! The server on a private session bus, started for each test with its own configuration (no
//! service activation, nothing of the user's session): `Notify`, replacement, closing,
//! expiry, actions, Do not disturb, urgency, and the name already taken.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader};
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc::{self, Receiver};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use notifications::core::{Core, Event, Sink};
    use notifications::history;
    use notifications::server::{self, INTERFACE, NAME, PATH};
    use notifications_config::{Follower, NotificationsConfig};
    use sherlock_ui::notifications as ui;
    use zbus::blocking::{Connection, MessageIterator, connection};
    use zbus::zvariant::Value;

    /// A private bus: its own `dbus-daemon`, socket in a folder of its own, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        dir: PathBuf,
    }

    impl Bus {
        fn start(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("notifications-{name}-{}", std::process::id()));
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

        fn connect(&self) -> Connection {
            connection::Builder::address(self.address.as_str())
                .unwrap()
                .build()
                .unwrap()
        }

        /// The daemon on this bus, its events, and the path of its settings file.
        fn serve(&self) -> (Arc<Core>, Receiver<Event>, PathBuf) {
            let (sender, events) = mpsc::channel();
            let sender = Mutex::new(sender);
            let sink: Sink = Box::new(move |event| {
                let _ = sender.lock().unwrap().send(event);
            });
            let settings = self.dir.join("notifications.conf");
            let core = Core::new(sink, Some(Follower::new(settings.clone())));
            let builder = connection::Builder::address(self.address.as_str()).unwrap();
            let served = server::start(builder, &core).unwrap();
            // The connection lives as long as the core: leaked for the test.
            std::mem::forget(served.expect("the name is free on a private bus"));
            (core, events, settings)
        }
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// `Notify` from a client.
    fn notify(
        client: &Connection,
        replaces: u32,
        summary: &str,
        actions: &[&str],
        hints: HashMap<&str, Value<'_>>,
        timeout: i32,
    ) -> u32 {
        let body = &(
            "test",
            replaces,
            "",
            summary,
            "<b>Corps</b>",
            actions,
            hints,
            timeout,
        );
        let reply = client
            .call_method(Some(NAME), PATH, Some(INTERFACE), "Notify", body)
            .unwrap();
        reply.body().deserialize().unwrap()
    }

    fn urgency(level: u8) -> HashMap<&'static str, Value<'static>> {
        HashMap::from([("urgency", Value::U8(level))])
    }

    /// The signals of the server, as `(member, id, argument)`.
    fn signals(client: &Connection) -> Receiver<(String, u32, String)> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(INTERFACE)
            .unwrap()
            .build();
        let iterator = MessageIterator::for_match_rule(rule, client, None).unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for message in iterator.flatten() {
                let header = message.header();
                let member = header.member().map(|m| m.to_string()).unwrap_or_default();
                let body = message.body();
                let parsed = match member.as_str() {
                    "NotificationClosed" => body
                        .deserialize::<(u32, u32)>()
                        .map(|(i, r)| (i, r.to_string())),
                    _ => body.deserialize::<(u32, String)>(),
                };
                if let Ok((id, argument)) = parsed
                    && sender.send((member, id, argument)).is_err()
                {
                    return;
                }
            }
        });
        receiver
    }

    fn next(signals: &Receiver<(String, u32, String)>) -> (String, u32, String) {
        signals
            .recv_timeout(Duration::from_secs(5))
            .expect("a signal")
    }

    fn shown(events: &Receiver<Event>) -> Vec<(u32, String)> {
        events
            .try_iter()
            .filter_map(|event| match event {
                Event::Show(note) => Some((note.id, note.summary.clone())),
                Event::Close(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_notification_shows_then_closes() {
        let bus = Bus::start("close");
        let (core, events, _) = bus.serve();
        let client = bus.connect();
        let signals = signals(&client);
        let id = notify(&client, 0, "Build terminé", &[], HashMap::new(), 0);
        assert_eq!(id, 1);
        assert_eq!(shown(&events), [(1, String::from("Build terminé"))]);
        client
            .call_method(
                Some(NAME),
                PATH,
                Some(INTERFACE),
                "CloseNotification",
                &(id,),
            )
            .unwrap();
        assert_eq!(
            next(&signals),
            (String::from("NotificationClosed"), id, String::from("3"))
        );
        assert!(matches!(events.try_recv(), Ok(Event::Close(1))));
        assert!(core.open(id).is_none());
        // Closing it again, or an id never given: the empty error of the specification.
        for id in [id, 99] {
            let closed = client.call_method(
                Some(NAME),
                PATH,
                Some(INTERFACE),
                "CloseNotification",
                &(id,),
            );
            assert!(closed.is_err());
        }

        let reply = client
            .call_method(Some(NAME), PATH, Some(INTERFACE), "GetCapabilities", &())
            .unwrap();
        let capabilities: Vec<String> = reply.body().deserialize().unwrap();
        assert!(
            ["actions", "body", "body-markup"]
                .iter()
                .all(|c| capabilities.iter().any(|x| x == c))
        );
        let reply = client
            .call_method(
                Some(NAME),
                PATH,
                Some(INTERFACE),
                "GetServerInformation",
                &(),
            )
            .unwrap();
        let info: (String, String, String, String) = reply.body().deserialize().unwrap();
        assert_eq!(
            (info.0.as_str(), info.1.as_str(), info.3.as_str()),
            ("notifications", "SherlockOS", "1.2")
        );
    }

    #[test]
    fn replaces_id_updates_the_same_bubble() {
        let bus = Bus::start("replace");
        let (core, events, _) = bus.serve();
        let client = bus.connect();
        let id = notify(&client, 0, "Copie : 10 %", &[], HashMap::new(), 0);
        let again = notify(&client, id, "Copie : 90 %", &[], HashMap::new(), 0);
        assert_eq!(again, id);
        assert_eq!(core.shown(), [id]);
        assert_eq!(
            shown(&events),
            [
                (id, String::from("Copie : 10 %")),
                (id, String::from("Copie : 90 %"))
            ]
        );
        // An id this daemon never gave is not taken over.
        assert_eq!(notify(&client, 77, "Autre", &[], HashMap::new(), 0), id + 1);
    }

    #[test]
    fn notifications_expire_except_critical_ones() {
        let bus = Bus::start("expire");
        let (core, events, _) = bus.serve();
        let client = bus.connect();
        let signals = signals(&client);
        let critical = notify(&client, 0, "Batterie faible", &[], urgency(2), 100);
        let id = notify(&client, 0, "Court", &[], HashMap::new(), 100);
        assert_eq!(
            next(&signals),
            (String::from("NotificationClosed"), id, String::from("1"))
        );
        assert!(
            events
                .try_iter()
                .any(|e| matches!(e, Event::Close(i) if i == id))
        );
        assert!(core.open(critical).is_some_and(|n| n.timeout.is_none()));
        // Without a timeout from the application: the default one.
        let default = notify(&client, 0, "Défaut", &[], urgency(1), -1);
        assert_eq!(
            core.open(default).and_then(|n| n.timeout),
            Some(ui::DEFAULT_TIMEOUT)
        );
        std::thread::sleep(Duration::from_millis(300));
        assert!(core.open(critical).is_some());
    }

    #[test]
    fn actions_are_invoked_then_close() {
        let bus = Bus::start("actions");
        let (core, _events, _) = bus.serve();
        let client = bus.connect();
        let signals = signals(&client);
        let actions = ["default", "Ouvrir", "restart", "Relancer"];
        let id = notify(
            &client,
            0,
            "front-next s'est arrêté",
            &actions,
            HashMap::new(),
            0,
        );
        core.invoke(id, "nothing");
        core.invoke(id, "restart");
        assert_eq!(
            next(&signals),
            (String::from("ActionInvoked"), id, String::from("restart"))
        );
        assert_eq!(
            next(&signals),
            (String::from("NotificationClosed"), id, String::from("2"))
        );
        // A click on the bubble runs its default action.
        let id = notify(&client, 0, "Clic", &actions, HashMap::new(), 0);
        core.click(id);
        assert_eq!(
            next(&signals),
            (String::from("ActionInvoked"), id, String::from("default"))
        );
        assert_eq!(
            next(&signals),
            (String::from("NotificationClosed"), id, String::from("2"))
        );
        // Without one, a click only closes it.
        let id = notify(&client, 0, "Sans action", &[], HashMap::new(), 0);
        core.click(id);
        assert_eq!(
            next(&signals),
            (String::from("NotificationClosed"), id, String::from("2"))
        );
        // A resident notification stays after its action.
        let hints = HashMap::from([("resident", Value::Bool(true))]);
        let id = notify(&client, 0, "Résidente", &actions, hints, 0);
        core.invoke(id, "restart");
        assert_eq!(
            next(&signals),
            (String::from("ActionInvoked"), id, String::from("restart"))
        );
        assert!(core.open(id).is_some());
    }

    #[test]
    fn do_not_disturb_hides_all_but_critical_and_keeps_the_history() {
        let bus = Bus::start("dnd");
        let (core, events, settings) = bus.serve();
        let client = bus.connect();
        let signals = signals(&client);
        NotificationsConfig {
            do_not_disturb: true,
            until: None,
        }
        .save(&settings)
        .unwrap();
        let quiet = notify(&client, 0, "Silencieuse", &[], urgency(1), 0);
        let low = notify(&client, 0, "Basse", &[], urgency(0), 0);
        let critical = notify(&client, 0, "Critique", &[], urgency(2), 0);
        assert_eq!(shown(&events), [(critical, String::from("Critique"))]);
        assert_eq!(core.shown(), [critical]);
        let history: Vec<u32> = core.history().iter().map(|n| n.id).collect();
        assert_eq!(history, [critical, low, quiet]);
        // Hidden ones are not kept open: closed by the daemon (reason 4), even without a timeout.
        let mut closed = vec![next(&signals), next(&signals)];
        closed.sort();
        let reason = |id| (String::from("NotificationClosed"), id, String::from("4"));
        assert_eq!(closed, [reason(quiet), reason(low)]);
        assert!(core.open(quiet).is_none() && core.open(low).is_none());
        // Read again at once: the next notification shows.
        NotificationsConfig {
            do_not_disturb: false,
            until: None,
        }
        .save(&settings)
        .unwrap();
        // An end already past (D-23e): Do not disturb holds no more.
        NotificationsConfig {
            do_not_disturb: true,
            until: Some(1),
        }
        .save(&settings)
        .unwrap();
        let after = notify(&client, 0, "Après", &[], urgency(1), 0);
        assert_eq!(shown(&events), [(after, String::from("Après"))]);
    }

    #[test]
    fn an_application_cannot_take_over_the_notification_of_another() {
        let bus = Bus::start("owner");
        let (core, _events, _) = bus.serve();
        let first = bus.connect();
        let second = bus.connect();
        let id = notify(
            &first,
            0,
            "À supprimer ?",
            &["delete", "Supprimer"],
            HashMap::new(),
            0,
        );
        // The second one names it: a new notification, the first one untouched.
        let other = notify(
            &second,
            id,
            "Leurre",
            &["delete", "Ouvrir"],
            HashMap::new(),
            0,
        );
        assert_ne!(other, id);
        let kept = core.open(id).unwrap();
        assert_eq!(kept.summary, "À supprimer ?");
        assert_eq!(kept.actions[0].1, "Supprimer");
        // Nor can it close it.
        let closed = second.call_method(
            Some(NAME),
            PATH,
            Some(INTERFACE),
            "CloseNotification",
            &(id,),
        );
        assert!(closed.is_err());
        assert!(core.open(id).is_some());
        // Its owner still can replace it.
        assert_eq!(
            notify(&first, id, "Mise à jour", &[], HashMap::new(), 0),
            id
        );
    }

    #[test]
    fn a_second_daemon_does_not_take_the_name() {
        let bus = Bus::start("taken");
        let other = bus.connect();
        other.request_name(NAME).unwrap();
        let sink: Sink = Box::new(|_| {});
        let core = Core::new(sink, None);
        let builder = connection::Builder::address(bus.address.as_str()).unwrap();
        assert!(server::start(builder, &core).unwrap().is_none());
    }

    /// `List` of the history interface.
    fn history(client: &Connection) -> Vec<history::Entry> {
        let reply = client
            .call_method(
                Some(NAME),
                history::PATH,
                Some(history::INTERFACE),
                "List",
                &(),
            )
            .unwrap();
        reply.body().deserialize().unwrap()
    }

    /// `Changed` of the history interface, one `()` per signal.
    fn changes(client: &Connection) -> Receiver<()> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(history::INTERFACE)
            .unwrap()
            .member("Changed")
            .unwrap()
            .build();
        let iterator = MessageIterator::for_match_rule(rule, client, None).unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for _ in iterator.flatten() {
                if sender.send(()).is_err() {
                    return;
                }
            }
        });
        receiver
    }

    #[test]
    fn the_history_interface_lists_clears_and_runs_actions() {
        let bus = Bus::start("history");
        let (core, _events, _) = bus.serve();
        let client = bus.connect();
        let signals = signals(&client);
        let changed = changes(&client);
        let first = notify(&client, 0, "Build terminé", &[], HashMap::new(), 0);
        changed.recv_timeout(Duration::from_secs(5)).unwrap();
        let actions = ["restart", "Relancer", "logs", "Voir les logs"];
        let second = notify(&client, 0, "front-next", &actions, urgency(2), 0);
        changed.recv_timeout(Duration::from_secs(5)).unwrap();

        // Newest first, the body as plain text, the actions of the open one.
        let list = history(&client);
        assert_eq!(list.len(), 2);
        let (id, app, summary, body, actions, open, received, critical) = &list[0];
        assert_eq!(
            (*id, app.as_str(), summary.as_str()),
            (second, "test", "front-next")
        );
        assert_eq!(body, "Corps");
        assert_eq!(
            actions[0],
            (String::from("restart"), String::from("Relancer"))
        );
        assert!(*open && *critical && *received > 1_700_000_000);
        assert_eq!(list[1].0, first);
        assert!(!list[1].7);

        // An action from the centre: `ActionInvoked`, then it closes and stays in the history.
        let invoke = |id: u32, key: &str| {
            client.call_method(
                Some(NAME),
                history::PATH,
                Some(history::INTERFACE),
                "InvokeAction",
                &(id, key),
            )
        };
        invoke(second, "logs").unwrap();
        assert_eq!(
            next(&signals),
            (String::from("ActionInvoked"), second, String::from("logs"))
        );
        assert_eq!(
            next(&signals),
            (
                String::from("NotificationClosed"),
                second,
                String::from("2")
            )
        );
        changed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(!history(&client)[0].5, "closed: no more actions");
        // Closed, or a key it does not have: refused.
        assert!(invoke(second, "logs").is_err());
        assert!(invoke(first, "nothing").is_err());

        // « Tout effacer »: the history goes, the open notification stays on screen.
        client
            .call_method(
                Some(NAME),
                history::PATH,
                Some(history::INTERFACE),
                "Clear",
                &(),
            )
            .unwrap();
        changed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(history(&client).is_empty());
        assert!(core.open(first).is_some());
    }
}
