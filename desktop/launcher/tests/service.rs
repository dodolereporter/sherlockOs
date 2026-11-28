//! The resident launcher (lot P-02): shown and hidden by its service, its search empty at each
//! opening, the second press handed to the service, a click beside the panel closing it, and
//! the session starting the service again when it stops. No screen: the panel is drawn by the
//! headless simulator, the session's programs are fakes.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command as Process;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use iced::Point;
    use iced::futures::StreamExt;
    use iced::futures::channel::{mpsc, oneshot};
    use iced::futures::executor::block_on;
    use iced_test::simulator::{self, Simulator};
    use launcher::app::{self, Launcher, Message};
    use launcher::apps::Apps;
    use launcher::service::{self, Inbox, Request};
    use launcher::source::{Command, Source};
    use mime_apps::Dirs;
    use sherlock_ui::launcher as look;

    /// A temporary folder under `CARGO_TARGET_TMPDIR`, removed at the end of the test.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("service-{name}-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temporary folder");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn apps() -> Apps {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data");
        let dirs = Dirs {
            config_home: data.join("config"),
            config_dirs: Vec::new(),
            data_home: data,
            data_dirs: Vec::new(),
        };
        Apps::load(&dirs, &["fr".into()], None)
    }

    /// A resident launcher on the fixtures, nothing started, and how many times it reloaded.
    fn resident() -> (Launcher, Arc<AtomicUsize>) {
        let spawn: app::Spawn = Arc::new(|_: &Command, _: Option<&str>| Ok(1));
        let reloads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reloads);
        let reload: app::Reload = Arc::new(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            let (_, walked) = oneshot::channel();
            (vec![Box::new(apps()) as Box<dyn Source>], walked)
        });
        let (_, receiver) = mpsc::unbounded();
        let launcher = Launcher::new(vec![Box::new(apps())], spawn)
            .resident(Inbox::from_receiver(receiver), reload);
        (launcher, reloads)
    }

    #[test]
    fn a_toggle_shows_then_hides_and_each_opening_starts_afresh() {
        let (mut launcher, _) = resident();
        assert!(!launcher.shown(), "hidden at the start");
        let _ = launcher.update(Message::Service(Request::Toggle));
        assert!(launcher.shown());
        let _ = launcher.update(Message::Query("ki".into()));
        let _ = launcher.update(Message::Key(app::Key::Down));
        assert!(launcher.hits().count() > 0);
        // The second press hides it.
        let _ = launcher.update(Message::Service(Request::Toggle));
        assert!(!launcher.shown() && launcher.closing);
        // Shown again: the search is empty, nothing selected, no result.
        let _ = launcher.update(Message::Service(Request::Toggle));
        assert!(launcher.shown());
        assert_eq!(launcher.query, "");
        assert_eq!((launcher.selected, launcher.hits().count()), (0, 0));
        // Another overlay opening hides it; a hidden launcher ignores that.
        let _ = launcher.update(Message::Service(Request::Hide));
        assert!(!launcher.shown());
        let _ = launcher.update(Message::Service(Request::Hide));
        assert!(!launcher.shown());
    }

    #[test]
    fn escape_and_a_start_hide_a_resident_launcher_without_ending_it() {
        let (mut launcher, _) = resident();
        let _ = launcher.update(Message::Service(Request::Toggle));
        let _ = launcher.update(Message::Key(app::Key::Escape));
        assert!(launcher.closing);
        let _ = launcher.update(Message::Service(Request::Toggle));
        let _ = launcher.update(Message::Query("ki".into()));
        let _ = launcher.update(Message::Key(app::Key::Enter));
        assert!(launcher.closing, "a start hides it");
        assert!(launcher.is_resident());
    }

    /// The messages a left click at `point` gives on a screen of `size`.
    fn click(launcher: &Launcher, size: (f32, f32), point: Point) -> Vec<Message> {
        let mut ui = Simulator::with_size(launcher::settings(), size, launcher.view_screen());
        ui.point_at(point);
        let _ = ui.simulate(simulator::click());
        ui.into_messages().collect()
    }

    #[test]
    fn a_click_beside_the_panel_closes_it_and_a_click_on_it_does_not() {
        for size in [(1920.0, 1080.0), (1366.0, 768.0), (800.0, 600.0)] {
            let (mut launcher, _) = resident();
            let _ = launcher.update(Message::Service(Request::Toggle));
            launcher.full = true;
            let middle = size.0 / 2.0;
            let dismissed =
                |messages: &[Message]| messages.iter().any(|m| matches!(m, Message::Dismiss));
            // On the panel, in its padding beside the search row: nothing closes.
            let inside = click(
                &launcher,
                size,
                Point::new(middle - look::WIDTH / 2.0 + 3.0, look::TOP + 3.0),
            );
            assert!(!dismissed(&inside), "{size:?}: {inside:?}");
            // On the search field: no close either.
            let field = Point::new(
                middle,
                look::TOP + look::PADDING + look::SEARCH_HEIGHT / 2.0,
            );
            assert!(!dismissed(&click(&launcher, size, field)), "{size:?}");
            // Above, beside and below the panel, as far as the corners: it closes.
            let below = look::TOP + launcher.height() + 10.0;
            for point in [
                Point::new(10.0, 10.0),
                Point::new(middle, look::TOP - 10.0),
                Point::new(middle + look::WIDTH / 2.0 + 10.0, look::TOP + 20.0),
                Point::new(middle, below),
                Point::new(size.0 - 1.0, size.1 - 1.0),
            ] {
                let messages = click(&launcher, size, point);
                assert!(dismissed(&messages), "{size:?} {point:?}: {messages:?}");
            }
            for message in click(&launcher, size, Point::new(10.0, 10.0)) {
                let _ = launcher.update(message);
            }
            assert!(launcher.closing, "{size:?}");
        }
    }

    /// Spawns the launcher binary as a second press would, with `dir` as its runtime folder only.
    fn press(dir: &Path) -> std::process::Child {
        Process::new(env!("CARGO_BIN_EXE_launcher"))
            .env_clear()
            .env("XDG_RUNTIME_DIR", dir)
            .env("PATH", "/usr/bin:/bin")
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("launcher started")
    }

    #[test]
    fn a_press_while_the_service_runs_toggles_it_in_less_than_100_ms() {
        let dir = TempDir::new("press");
        let service = service::listen(&dir.0, service::LAUNCHER)
            .expect("listen")
            .expect("first");
        let mut requests = service.spawn().expect("thread").stream();
        let (mut launcher, _) = resident();
        launcher.full = true;
        // The program started by the shortcut hands its press over and ends at once.
        let mut child = press(&dir.0);
        assert_eq!(block_on(requests.next()), Some(Request::Toggle));
        assert!(child.wait().expect("ended").success());
        let path = service::socket(&dir.0, service::LAUNCHER);
        // The best of 5: a loaded machine can pause the test during one run, which says nothing of
        // the cost of an opening. From a press sent to the service to the panel's view built.
        // Laying it out and drawing it is the GPU's work, measured in a session with the start of
        // the program of the shortcut (docs/launcher.md, Performance): the simulator's own set-up
        // costs more than that here.
        let mut best = Duration::MAX;
        for round in 0..6 {
            let started = Instant::now();
            let sent = path.clone();
            let client = std::thread::spawn(move || service::send(&sent, Request::Toggle));
            let request = block_on(requests.next());
            assert_eq!(request, Some(Request::Toggle));
            let _ = launcher.update(Message::Service(Request::Toggle));
            assert_eq!(launcher.shown(), round % 2 == 0, "every press toggles");
            let view = launcher.view_screen();
            best = best.min(started.elapsed());
            drop(view);
            client.join().expect("joined").expect("acknowledged");
        }
        assert!(best < Duration::from_millis(100), "{best:?}");
    }

    #[test]
    fn the_session_starts_the_service_again_when_it_stops() {
        let dir = TempDir::new("session");
        let d = &dir.0;
        for sub in ["bin", "path", "home", "run", "out"] {
            std::fs::create_dir_all(d.join(sub)).expect("folder");
        }
        let write = |path: &Path, body: &str| {
            std::fs::write(path, body).expect("written");
            let status = Process::new("chmod")
                .arg("+x")
                .arg(path)
                .status()
                .expect("chmod");
            assert!(status.success());
        };
        let out = d.join("out");
        let o = out.display();
        // The compositor runs the shell part with a display, long enough for three starts.
        write(
            &d.join("bin/compositor"),
            "#!/bin/sh\n[ \"$1\" = --xwayland ] && shift\nWAYLAND_DISPLAY=wayland-test \"$@\" &\nsleep 4\n",
        );
        for program in ["bar", "dock", "notifications", "sherlockos-apps"] {
            write(&d.join("bin").join(program), "#!/bin/sh\nsleep 5\n");
        }
        // The service dies at once, every time.
        write(
            &d.join("bin/launcher"),
            &format!("#!/bin/sh\necho \"$@\" >> '{o}/launcher'\nexit 1\n"),
        );
        let tools = [
            "sh", "dirname", "readlink", "mkdir", "mv", "awk", "date", "wc", "cat", "sleep", "env",
            "kill", "rm",
        ];
        for tool in tools {
            let found = Process::new("sh")
                .args(["-c", &format!("command -v {tool}")])
                .output()
                .expect("found");
            let found = String::from_utf8(found.stdout).expect("text");
            if found.trim().starts_with('/') {
                std::os::unix::fs::symlink(found.trim(), d.join("path").join(tool)).expect("link");
            }
        }
        write(
            &d.join("path/dbus-update-activation-environment"),
            "#!/bin/sh\n",
        );
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../compositor/session/sherlockos-session");
        let status = Process::new("env")
            .arg("-i")
            .arg(format!("PATH={}", d.join("path").display()))
            .arg(format!("HOME={}", d.join("home").display()))
            .arg(format!("XDG_RUNTIME_DIR={}", d.join("run").display()))
            .arg(format!("SHERLOCKOS_BIN={}", d.join("bin").display()))
            .arg(format!("SHERLOCKOS_DATA={}", d.join("data").display()))
            .arg("SHERLOCKOS_SESSION_BUS=shared")
            .arg("SHERLOCKOS_XWAYLAND=0")
            .arg("SHERLOCKOS_POLKIT_AGENTS=/nowhere/agent")
            .arg("sh")
            .arg(&script)
            .status()
            .expect("session");
        assert!(status.success(), "{status}");
        let starts = std::fs::read_to_string(out.join("launcher")).unwrap_or_default();
        let starts: Vec<&str> = starts.lines().collect();
        assert!(starts.len() >= 2, "started again: {starts:?}");
        assert!(
            starts.iter().all(|line| *line == "--resident"),
            "{starts:?}"
        );
        let log = std::fs::read_to_string(d.join("home/.local/state/sherlockos/session.log"))
            .unwrap_or_default();
        assert!(log.contains("launcher stopped (status 1)"), "{log}");
    }

    #[test]
    fn the_requests_of_a_service_reach_a_resident_launcher_in_order() {
        let (sender, receiver) = mpsc::unbounded();
        let inbox = Inbox::from_receiver(receiver);
        sender.unbounded_send(Request::Toggle).expect("sent");
        sender.unbounded_send(Request::Hide).expect("sent");
        drop(sender);
        let got: Vec<Request> = block_on(inbox.stream().collect());
        assert_eq!(got, [Request::Toggle, Request::Hide]);
    }
}
