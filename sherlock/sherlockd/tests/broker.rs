//! The consent broker (A-11) in front of the test server, a real process that writes down each
//! call it receives: no call beyond read ever reaches it without the user's consent, whatever
//! goes wrong with the consent window (refused, closed, never answered, its compositor gone).

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::time::Duration;

    use serde_json::{Value, json};
    use sherlockd::agent::{Agent, AgentConfig};
    use sherlockd::broker::{Broker, Consent, Decision, Journal, Nobody, Request};
    use sherlockd::mcp::{McpClient, McpError, McpServers, ServerConfig, ToolHost};
    use sherlockd::model::{ModelReply, ToolCall};
    use sherlockd::scripted::ScriptedProvider;
    use sherlockd::test_server::UNLEVELED_MARKER;

    const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// A folder of the test's own, for the server's trace and the journal.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sherlockd-broker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The test server, one of ours, writing its calls to `dir/trace`, behind a broker that asks
    /// `consent` and journals to `dir/journal.jsonl`, with an answer delay of 300 ms.
    async fn servers(dir: &Path, consent: Box<dyn Consent>) -> McpServers {
        let trace = dir.join("trace");
        let trace = trace.to_str().unwrap();
        let config = ServerConfig::new("test", EXE, &["--test-server", "--trace", trace])
            .timeout(TIMEOUT)
            .ours();
        let journal = Journal::open(&dir.join("journal.jsonl")).unwrap();
        let broker = Broker::new(consent, journal).with_timeout(Duration::from_millis(300));
        let mut servers = McpServers::with_broker(broker);
        servers
            .add(McpClient::spawn(&config).await.unwrap())
            .await
            .unwrap();
        servers
    }

    /// The tools the server received, in order.
    fn received(dir: &Path) -> Vec<String> {
        let text = std::fs::read_to_string(dir.join("trace")).unwrap_or_default();
        text.lines().map(str::to_owned).collect()
    }

    fn journal(dir: &Path) -> Vec<Value> {
        let text = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Answers with `decision`, and remembers what it was asked.
    struct Answers(Decision, std::sync::Arc<std::sync::Mutex<Vec<Request>>>);

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

    /// The window stays open, nobody answers.
    struct Silent;

    impl Consent for Silent {
        fn ask<'a>(
            &'a mut self,
            _: &'a Request,
        ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            Box::pin(std::future::pending())
        }
    }

    /// The compositor goes away during the request: its channel closes before any answer.
    struct Gone;

    impl Consent for Gone {
        fn ask<'a>(
            &'a mut self,
            _: &'a Request,
        ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            let (sender, answer) = std::sync::mpsc::channel::<Decision>();
            drop(sender);
            Box::pin(async move { answer.recv().unwrap_or(Decision::Unavailable) })
        }
    }

    #[tokio::test]
    async fn no_action_reaches_the_server_without_the_users_consent() {
        let cases: Vec<(&str, Box<dyn Consent>)> = vec![
            (
                "denied",
                Box::new(Answers(Decision::Denied, Default::default())),
            ),
            (
                "closed",
                Box::new(Answers(Decision::Denied, Default::default())),
            ),
            ("silent", Box::new(Silent)),
            ("gone", Box::new(Gone)),
            ("nobody", Box::new(Nobody)),
        ];
        for (name, consent) in cases {
            let dir = folder(name);
            let mut servers = servers(&dir, consent).await;
            // Directly, the only public path to a server's call.
            let refused = servers.call("vider_corbeille", json!({})).await;
            assert!(
                matches!(
                    refused,
                    Err(McpError::Refused { .. } | McpError::BrokerAbsent { .. })
                ),
                "{name}: {refused:?}"
            );
            // Through the agent loop, asked by an injection the model obeyed.
            let calls = vec![
                ToolCall {
                    id: "1".into(),
                    name: "vider_corbeille".into(),
                    arguments: json!({}),
                },
                ToolCall {
                    id: "2".into(),
                    name: "echo".into(),
                    arguments: json!({ "texte": "lu" }),
                },
            ];
            let script = ScriptedProvider::new()
                .reply(ModelReply::ToolCalls(calls))
                .reply(ModelReply::Text("fini".into()));
            let mut agent = Agent::new(script, servers, "Tu es Sherlock.", AgentConfig::default());
            assert_eq!(agent.ask("vide la corbeille").await.unwrap(), "fini");
            // The read ran, the action never reached the server.
            assert_eq!(received(&dir), ["echo"], "{name}");
            let text = format!("{:?}", agent.history());
            assert!(!text.contains(UNLEVELED_MARKER), "{name}");
            // Each refusal is journaled, as is the read.
            let decisions: Vec<String> = journal(&dir)
                .iter()
                .filter_map(|l| l["decision"].as_str().map(str::to_owned))
                .collect();
            assert_eq!(decisions.len(), 3, "{name}: {decisions:?}");
            assert!(
                decisions[0] != "granted" && decisions[1] != "granted",
                "{name}"
            );
            assert_eq!(decisions[2], "granted", "{name}: the read");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[tokio::test]
    async fn a_consent_runs_that_call_once_and_asks_reinforced_for_the_destructive_level() {
        let dir = folder("granted");
        let asked = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let consent = Answers(Decision::Granted, asked.clone());
        let mut servers = servers(&dir, Box::new(consent)).await;
        let output = servers
            .call(
                "vider_corbeille",
                json!({ "dossier": "/home/camille/.local/share/Trash" }),
            )
            .await
            .unwrap();
        assert_eq!(output.text, UNLEVELED_MARKER);
        assert_eq!(received(&dir), ["vider_corbeille"]);
        // The user saw the tool, its level and its arguments, and a reinforced consent was asked.
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 1);
        assert!(asked[0].reinforced() && asked[0].summary.contains("Trash"));
        // Journaled: the consent, then the outcome, under the same id.
        let lines = journal(&dir);
        assert_eq!(lines[0]["decision"], "granted");
        assert!(lines[0]["level"].is_string());
        assert_eq!(
            (lines[1]["outcome"].as_str(), &lines[1]["id"]),
            (Some("ok"), &lines[0]["id"])
        );
        // The read asks nobody.
        servers.call("echo", json!({ "texte": "x" })).await.unwrap();
        assert_eq!(servers.tools().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
