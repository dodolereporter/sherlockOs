//! `sherlockd` asking the compositor's consent window (A-11c), against a fake compositor that
//! speaks its protocol on a socket of the test's own: the request it is sent, the answer read,
//! and every way it can go wrong refusing the call. The calls go to the real test server, which
//! writes down what it receives.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::{Value, json};
    use sherlockd::broker::{Broker, Journal};
    use sherlockd::mcp::{McpClient, McpError, McpServers, ServerConfig, ToolHost};
    use sherlockd::window::CompositorWindow;

    const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sherlockd-window-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// What the fake compositor does with a request.
    #[derive(Clone, Copy)]
    enum Answer {
        Granted,
        Denied,
        /// The connection closed without an answer.
        Hangs,
        /// No answer at all: the broker gives up.
        Never,
        /// An answer to another request.
        OtherId,
    }

    /// A fake compositor at `dir/consent`: each request it reads goes on `seen`, with whether
    /// the asker closed the connection while it waited.
    fn compositor(dir: &Path, answer: Answer) -> mpsc::Receiver<(Option<Value>, bool)> {
        let listener = UnixListener::bind(dir.join("consent")).unwrap();
        let (seen, received) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                let request: Option<Value> = match reader.read_line(&mut line) {
                    Ok(n) if n > 0 => serde_json::from_str(&line).ok(),
                    _ => None,
                };
                let id = request.as_ref().and_then(|r| r["id"].as_u64()).unwrap_or(0);
                let reply = match answer {
                    Answer::Granted => Some(json!({ "id": id, "decision": "granted" })),
                    Answer::Denied => Some(json!({ "id": id, "decision": "denied" })),
                    Answer::OtherId => Some(json!({ "id": id + 1, "decision": "granted" })),
                    Answer::Hangs | Answer::Never => None,
                };
                if let Some(reply) = reply {
                    let _ = writeln!(stream, "{reply}");
                }
                // Waits for the asker to close: withdrawn.
                let closed = match answer {
                    Answer::Never => {
                        let mut rest = String::new();
                        matches!(reader.read_line(&mut rest), Ok(0))
                    }
                    _ => false,
                };
                if seen.send((request, closed)).is_err() {
                    return;
                }
            }
        });
        received
    }

    async fn start(dir: &Path, window: CompositorWindow) -> McpServers {
        let trace = dir.join("trace");
        let trace = trace.to_str().unwrap();
        let config = ServerConfig::new("test", EXE, &["--test-server", "--trace", trace])
            .timeout(Duration::from_secs(5))
            .ours();
        let journal = Journal::open(&dir.join("journal.jsonl")).unwrap();
        let broker =
            Broker::new(Box::new(window), journal).with_timeout(Duration::from_millis(400));
        let mut servers = McpServers::with_broker(broker);
        servers
            .add(McpClient::spawn(&config).await.unwrap())
            .await
            .unwrap();
        servers
    }

    fn received(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("trace")).unwrap_or_default()
    }

    #[tokio::test]
    async fn the_window_asked_grants_or_refuses_and_any_failure_refuses() {
        for (name, answer, runs) in [
            ("granted", Answer::Granted, true),
            ("denied", Answer::Denied, false),
            ("hangs", Answer::Hangs, false),
            ("never", Answer::Never, false),
            ("other-id", Answer::OtherId, false),
        ] {
            let dir = folder(name);
            let seen = compositor(&dir, answer);
            let window = CompositorWindow::at(&dir.join("consent")).trusting_any_program();
            let mut servers = start(&dir, window).await;
            let result = servers
                .call("vider_corbeille", json!({ "dossier": "Trash" }))
                .await;
            assert_eq!(result.is_ok(), runs, "{name}: {result:?}");
            assert_eq!(received(&dir).contains("vider_corbeille"), runs, "{name}");
            // The compositor was asked about that call, with its level and arguments.
            let (request, closed) = seen.recv_timeout(Duration::from_secs(5)).unwrap();
            let request = request.unwrap();
            assert_eq!(request["tool"], "vider_corbeille");
            assert_eq!(request["level"], "destructive");
            assert!(request["arguments"].as_str().unwrap().contains("Trash"));
            // Given up after its delay, the asker closed the connection: the window goes.
            if matches!(answer, Answer::Never) {
                assert!(closed, "{name}: withdrawn");
                assert!(matches!(result, Err(McpError::Refused { .. })));
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[tokio::test]
    async fn no_compositor_or_another_program_on_the_socket_refuses_without_asking() {
        // No socket at all.
        let dir = folder("absent");
        let window = CompositorWindow::at(&dir.join("consent"));
        let mut servers = start(&dir, window).await;
        let refused = servers.call("vider_corbeille", json!({})).await;
        assert!(
            matches!(refused, Err(McpError::BrokerAbsent { .. })),
            "{refused:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
        // A process that is not the compositor holds the path: it is told nothing.
        let dir = folder("squatter");
        let seen = compositor(&dir, Answer::Granted);
        let window = CompositorWindow::at(&dir.join("consent"));
        let mut servers = start(&dir, window).await;
        let refused = servers.call("vider_corbeille", json!({})).await;
        assert!(
            matches!(refused, Err(McpError::BrokerAbsent { .. })),
            "{refused:?}"
        );
        assert!(!received(&dir).contains("vider_corbeille"));
        let (request, _) = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(request.is_none(), "the squatter read no request");
        // The read beside it still runs.
        servers
            .call("echo", json!({ "texte": "lu" }))
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
