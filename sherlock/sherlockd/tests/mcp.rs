//! MCP client and agent loop against the test server, started as a real process, well-behaved
//! or hostile.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig, AgentError};
use sherlockd::mcp::{Level, McpClient, McpError, McpServers, ServerConfig, ToolHost};
use sherlockd::model::{Message, ModelReply, ToolCall};
use sherlockd::scripted::{ScriptedProvider, last_tool_result};
use sherlockd::test_server::{HIDDEN_INSTRUCTION, UNLEVELED_MARKER};

const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");
const TIMEOUT: Duration = Duration::from_secs(5);

/// The test server, in the given case, marked as one of ours.
fn config(case: Option<&str>) -> ServerConfig {
    let args: Vec<&str> = ["--test-server"].into_iter().chain(case).collect();
    ServerConfig::new("test", EXE, &args)
        .timeout(TIMEOUT)
        .ours()
}

async fn servers(case: Option<&str>) -> Result<McpServers, McpError> {
    let mut servers = McpServers::new();
    servers.add(McpClient::spawn(&config(case)).await?).await?;
    Ok(servers)
}

fn call(name: &str, arguments: serde_json::Value) -> ModelReply {
    ModelReply::ToolCalls(vec![ToolCall {
        id: name.into(),
        name: name.into(),
        arguments,
    }])
}

fn echo(text: &str) -> ModelReply {
    call("echo", json!({ "texte": text }))
}

fn levels(servers: &McpServers) -> Vec<(&str, Level)> {
    servers
        .tools()
        .iter()
        .map(|t| (t.name.as_str(), t.level))
        .collect()
}

#[tokio::test]
async fn lists_tools_and_levels() {
    let servers = servers(None).await.unwrap();
    assert_eq!(
        levels(&servers),
        [
            ("echo", Level::Read),
            ("vider_corbeille", Level::Destructive)
        ]
    );
    assert_eq!(
        servers.tools()[0].input_schema["required"],
        json!(["texte"])
    );
}

#[tokio::test]
async fn third_party_server_all_destructive() {
    let mut config = config(None);
    config.trust = Default::default();
    let mut servers = McpServers::new();
    servers
        .add(McpClient::spawn(&config).await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        levels(&servers),
        [
            ("echo", Level::Destructive),
            ("vider_corbeille", Level::Destructive)
        ]
    );
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::BrokerAbsent { .. }), "{error}");
}

#[tokio::test]
async fn successful_call() {
    let mut servers = servers(None).await.unwrap();
    let output = servers
        .call("echo", json!({ "texte": "salut" }))
        .await
        .unwrap();
    assert_eq!(output.text, "salut");
    assert!(!output.is_error);
}

#[tokio::test]
async fn direct_non_read_call_refused() {
    let mut servers = servers(None).await.unwrap();
    let error = servers
        .call("vider_corbeille", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        McpError::BrokerAbsent {
            level: Level::Destructive,
            ..
        }
    ));
    assert!(!error.to_string().contains(UNLEVELED_MARKER));
    // The server stays usable: the refusal sent nothing.
    assert!(servers.call("echo", json!({ "texte": "x" })).await.is_ok());
}

#[tokio::test]
async fn agent_calls_echo_and_refuses_unleveled_tool() {
    let provider = ScriptedProvider::new()
        .reply(echo("indice"))
        .reply(call("vider_corbeille", json!({})))
        .then(|r| ModelReply::Text(last_tool_result(r).unwrap()));
    let servers = servers(None).await.unwrap();
    let mut agent = Agent::new(provider, servers, "Sherlock", AgentConfig::default());
    let answer = agent.ask("Enquête.").await.unwrap();
    assert!(answer.starts_with("courtier d'accords absent : l'outil vider_corbeille"));

    let history = agent.history();
    assert!(history.contains(&Message::ToolResult {
        call_id: "echo".into(),
        content: "indice".into(),
        is_error: false,
    }));
    assert!(!format!("{history:?}").contains(UNLEVELED_MARKER));
}

#[tokio::test]
async fn json_rpc_error_stays_data() {
    let provider = ScriptedProvider::new()
        .reply(echo("x"))
        .reply(ModelReply::Text("Fini.".into()));
    let servers = servers(Some("error-with-instruction")).await.unwrap();
    let mut agent = Agent::new(provider, servers, "Sherlock", AgentConfig::default());
    agent.ask("?").await.unwrap();
    for request in &agent.provider().requests {
        assert_eq!(request.system, "Sherlock");
    }
    let results: Vec<_> = agent
        .history()
        .iter()
        .filter(|m| format!("{m:?}").contains(HIDDEN_INSTRUCTION))
        .collect();
    assert!(
        matches!(
            results.as_slice(),
            [Message::ToolResult { is_error: true, .. }]
        ),
        "{results:?}"
    );
}

#[tokio::test]
async fn turn_limit() {
    let mut provider = ScriptedProvider::new();
    for _ in 0..5 {
        provider = provider.reply(echo("encore"));
    }
    let config = AgentConfig {
        max_turns: 2,
        ..AgentConfig::default()
    };
    let mut agent = Agent::new(provider, servers(None).await.unwrap(), "Sherlock", config);
    assert!(matches!(
        agent.ask("Boucle.").await,
        Err(AgentError::TurnLimit(2))
    ));
}

#[tokio::test]
async fn crashing_server() {
    let mut servers = servers(Some("crash-on-call")).await.unwrap();
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Closed(_)), "{error}");
    // The dead server does not freeze the client: the next call fails at once.
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Lost(_)), "{error}");
}

#[tokio::test]
async fn server_crashing_during_conversation() {
    let provider = ScriptedProvider::new()
        .reply(echo("x"))
        .then(|r| ModelReply::Text(last_tool_result(r).unwrap()));
    let servers = servers(Some("crash-on-call")).await.unwrap();
    let mut agent = Agent::new(provider, servers, "Sherlock", AgentConfig::default());
    assert_eq!(
        agent.ask("?").await.unwrap(),
        "le serveur test s'est arrêté"
    );
}

#[tokio::test]
async fn timeout_server_stopped_and_reaped() {
    let config = config(Some("silent-on-call")).timeout(Duration::from_millis(300));
    let client = McpClient::spawn(&config).await.unwrap();
    let pid = client.pid().unwrap();
    let mut servers = McpServers::new();
    servers.add(client).await.unwrap();
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Timeout { .. }), "{error}");
    // Neither process nor zombie: the server was stopped then reaped.
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Lost(_)), "{error}");
}

#[tokio::test]
async fn list_not_an_object() {
    let error = servers(Some("list-not-object")).await.err().unwrap();
    assert!(matches!(error, McpError::Protocol { .. }), "{error}");
}

#[tokio::test]
async fn duplicate_refused() {
    for case in ["duplicate-in-page", "duplicate-across-pages"] {
        let error = servers(Some(case)).await.err().unwrap();
        assert!(
            matches!(&error, McpError::DuplicateTool(name) if name == "echo"),
            "{case}: {error}"
        );
    }
}

#[tokio::test]
async fn endless_pagination_bounded() {
    let start = Instant::now();
    let error = servers(Some("endless-pages")).await.err().unwrap();
    assert!(matches!(&error, McpError::Protocol { detail, .. } if detail.contains("pages")));
    assert!(start.elapsed() < TIMEOUT);
}

#[tokio::test]
async fn giant_line_refused() {
    let error = servers(Some("giant-line")).await.err().unwrap();
    assert!(
        matches!(&error, McpError::Protocol { detail, .. } if detail.contains("octets")),
        "{error}"
    );
}

#[tokio::test]
async fn server_not_found() {
    let result = McpClient::spawn(&ServerConfig::new("absent", "/nowhere", &[])).await;
    assert!(matches!(result, Err(McpError::Spawn { .. })));
}

#[tokio::test]
async fn silent_server() {
    let config = ServerConfig::new("silent", "sleep", &["10"]).timeout(Duration::from_millis(200));
    let result = McpClient::spawn(&config).await;
    assert!(matches!(result, Err(McpError::Timeout { .. })));
}
