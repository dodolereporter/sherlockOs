//! Sherlock's consent from end to end (lots A-11 to A-11c): the real `sherlockd`, as Claude
//! Code starts it (`--proxy --consent`), its broker, its journal and its test server, in front
//! of the compositor's consent window, in a process of its own named `compositor` so that each
//! side's peer check passes as in the session.
//!
//! The test plays Claude Code: it speaks MCP to the proxy and asks twice for `vider_corbeille`,
//! a destructive tool. The compositor process (this same test binary, linked as `compositor` and
//! run with `--ignored`) shows the window for each request and answers as the user would, through
//! the input entry points the session's backend calls: Escape the first time, « Autoriser » held
//! the second. The refused call runs nothing, the granted one runs, and the journal has the
//! lines.
//!
//! The test needs the `sherlockd` binary next to the compositor's: the workspace's tests build
//! both. Built alone (`cargo test -p compositor`), it says so and passes, except on the CI.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use compositor::{State, render};
    use serde_json::{Value, json};
    use smithay::backend::input::{ButtonState, KeyState};
    use smithay::input::keyboard::Keycode;
    use smithay::reexports::calloop::EventLoop;

    /// What the test server returns when `vider_corbeille` runs.
    const RAN: &str = "UNLEVELED-TOOL-EXECUTED";
    const KEY_ESCAPE: u32 = 9;
    const BTN_LEFT: u32 = 0x110;

    /// The compositor's side: started by the test below, never on its own (`E2E_SOCKET` unset).
    #[test]
    #[ignore = "run by sherlockd_asks_the_compositor_and_does_only_what_was_granted"]
    fn consent_compositor_child() {
        let Some(socket) = std::env::var_os("E2E_SOCKET").map(PathBuf::from) else {
            return;
        };
        let log = PathBuf::from(std::env::var_os("E2E_LOG").unwrap());
        let script = std::env::var("E2E_SCRIPT").unwrap();
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        let output = state.add_output((1440, 900).into(), 1.0);
        let peers = compositor::consent::Peers::Sherlockd;
        state
            .listen_consent_next_to(&socket, peers, &event_loop)
            .unwrap();
        std::fs::write(std::env::var_os("E2E_READY").unwrap(), "").unwrap();
        let mut event_loop = event_loop;
        let mut dispatch = |state: &mut State| {
            event_loop
                .dispatch(Some(Duration::from_millis(5)), state)
                .unwrap();
            state.refresh();
        };
        let deadline = Instant::now() + Duration::from_secs(90);
        for answer in script.split(',') {
            while !state.consent_shown() {
                assert!(Instant::now() < deadline, "no request came");
                dispatch(&mut state);
            }
            // What the user sees: the tool asked for, drawn in front of everything.
            let views = state.views();
            let (items, _) =
                render::items(&state, (&output, &views[0].1), None, false, state.now());
            let shown = format!("{:?}", items[0]);
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(&log)
                .unwrap();
            writeln!(file, "shown {}", shown.contains("ConsentLabel")).unwrap();
            std::thread::sleep(sherlock_ui::consent::ARMING);
            match answer {
                "deny" => {
                    state.keyboard_key(Keycode::new(KEY_ESCAPE), KeyState::Pressed, 10);
                    state.keyboard_key(Keycode::new(KEY_ESCAPE), KeyState::Released, 11);
                }
                _ => {
                    let layout = state.consent_layout(&output).unwrap();
                    let allow = layout.allow.loc + layout.allow.size.downscale(2.0).to_point();
                    state.pointer_motion(allow, 20);
                    state.pointer_button(BTN_LEFT, ButtonState::Pressed, 21);
                    std::thread::sleep(sherlock_ui::consent::HOLD);
                    state.pointer_button(BTN_LEFT, ButtonState::Released, 22);
                }
            }
            assert!(!state.consent_shown(), "answered: {answer}");
            for _ in 0..20 {
                dispatch(&mut state);
            }
        }
        // Until the asker goes: its last answer must reach it.
        while Instant::now() < deadline && !log.with_extension("done").exists() {
            dispatch(&mut state);
        }
    }

    /// The `sherlockd` binary built with this one, if it was.
    fn sherlockd() -> Option<PathBuf> {
        let path = Path::new(env!("CARGO_BIN_EXE_compositor")).with_file_name("sherlockd");
        path.exists().then_some(path)
    }

    /// Reads the proxy's answers in a thread: one JSON value per line.
    fn answers(child: &mut Child) -> mpsc::Receiver<Value> {
        let (send, receive) = mpsc::channel();
        let stdout = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if let Ok(value) = serde_json::from_str(&line) {
                    let _ = send.send(value);
                }
            }
        });
        receive
    }

    fn reply(answers: &mpsc::Receiver<Value>, id: u64) -> Value {
        loop {
            let value = answers.recv_timeout(Duration::from_secs(60)).unwrap();
            if value["id"] == id {
                return value;
            }
        }
    }

    #[test]
    fn sherlockd_asks_the_compositor_and_does_only_what_was_granted() {
        let Some(sherlockd) = sherlockd() else {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI builds sherlockd with the workspace"
            );
            println!("sherlockd not built: build the workspace to run this test");
            return;
        };
        let dir = std::env::temp_dir().join(format!("sherlock-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = dir.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let private = std::os::unix::fs::PermissionsExt::from_mode(0o700);
        std::fs::set_permissions(&run, private).unwrap();
        // This test binary, under the compositor's name: what sherlockd checks of its peer.
        let compositor = dir.join("compositor");
        let exe = std::env::current_exe().unwrap();
        if std::fs::hard_link(&exe, &compositor).is_err() {
            std::fs::copy(&exe, &compositor).unwrap();
        }
        let (socket, ready, log) = (run.join("wayland-e2e"), dir.join("ready"), dir.join("log"));
        let mut window = Command::new(&compositor)
            .args([
                "--exact",
                "tests::consent_compositor_child",
                "--ignored",
                "--test-threads=1",
            ])
            .env("E2E_SOCKET", &socket)
            .env("E2E_READY", &ready)
            .env("E2E_LOG", &log)
            .env("E2E_SCRIPT", "deny,hold")
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !ready.exists() {
            assert!(started.elapsed() < Duration::from_secs(30), "no compositor");
            std::thread::sleep(Duration::from_millis(20));
        }

        // Claude Code's side: the proxy it starts, with the consent socket in its arguments,
        // in an environment of its own.
        let consent = compositor::consent::socket_path(&socket);
        let mut proxy = Command::new(&sherlockd)
            .args(["--proxy", "--consent", consent.to_str().unwrap()])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", dir.join("home"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("XDG_STATE_HOME", dir.join("state"))
            .env("XDG_RUNTIME_DIR", &run)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let replies = answers(&mut proxy);
        let mut input = proxy.stdin.take().unwrap();
        let mut send = |value: Value| writeln!(input, "{value}").unwrap();
        send(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": { "name": "e2e", "version": "0" } } }),
        );
        reply(&replies, 1);
        send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        let call = |id: u64| {
            json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                    "params": { "name": "vider_corbeille", "arguments": {} } })
        };
        // Refused with Escape: nothing ran.
        send(call(2));
        let refused = reply(&replies, 2);
        assert!(!refused.to_string().contains(RAN), "{refused}");
        // Granted by the hold: it ran, once.
        send(call(3));
        let granted = reply(&replies, 3);
        assert_eq!(granted["result"]["content"][0]["text"], RAN, "{granted}");
        assert_ne!(granted["result"]["isError"], true);
        std::fs::write(log.with_extension("done"), "").unwrap();
        drop(send);

        // The window showed for each request, drawn first.
        let shown = std::fs::read_to_string(&log).unwrap();
        assert_eq!(shown, "shown true\nshown true\n");
        // The journal: the refusal, the consent, then the outcome of the call that ran.
        let journal = std::fs::read_to_string(dir.join("state/sherlock/journal.jsonl")).unwrap();
        let lines: Vec<Value> = journal
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let decisions: Vec<&str> = lines
            .iter()
            .filter_map(|l| l["decision"].as_str())
            .collect();
        assert_eq!(decisions, ["denied", "granted"], "{journal}");
        let outcomes: Vec<&Value> = lines
            .iter()
            .filter(|l| l.get("outcome").is_some())
            .collect();
        assert_eq!(outcomes.len(), 1, "{journal}");
        assert_eq!(outcomes[0]["outcome"], "ok");
        assert_eq!(outcomes[0]["id"], lines[1]["id"]);
        assert!(lines.iter().all(|l| l["tool"] == "vider_corbeille"));

        assert!(
            window.wait().unwrap().success(),
            "the compositor's side failed"
        );
        let _ = proxy.kill();
        let _ = proxy.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
