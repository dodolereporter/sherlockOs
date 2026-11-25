//! Client MCP et boucle d'agent contre le serveur de test, lancé en vrai processus.

use std::time::Duration;

use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig, AgentError};
use sherlockd::faux::{ScriptedProvider, last_tool_result};
use sherlockd::mcp::{Level, McpClient, McpError, McpServers, ToolHost};
use sherlockd::modele::{Message, ModelReply, ToolCall};
use sherlockd::serveur_essai::MARQUEUR_SANS_NIVEAU;

const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");
const TIMEOUT: Duration = Duration::from_secs(5);

async fn server(args: &[&str]) -> Result<McpClient, McpError> {
    McpClient::spawn("essai", EXE, args, TIMEOUT).await
}

async fn servers() -> Result<McpServers, McpError> {
    let mut servers = McpServers::new();
    servers.add(server(&["--serveur-essai"]).await?).await?;
    Ok(servers)
}

fn call(name: &str, arguments: serde_json::Value) -> ModelReply {
    ModelReply::ToolCalls(vec![ToolCall {
        id: name.into(),
        name: name.into(),
        arguments,
    }])
}

#[tokio::test]
async fn liste_les_outils_et_leur_niveau() {
    let tools = server(&["--serveur-essai"])
        .await
        .unwrap()
        .list_tools()
        .await
        .unwrap();
    let names: Vec<_> = tools.iter().map(|t| (t.name.as_str(), t.level)).collect();
    assert_eq!(
        names,
        [
            ("echo", Level::Read),
            ("vider_corbeille", Level::Destructive)
        ]
    );
    assert_eq!(tools[0].input_schema["required"], json!(["texte"]));
}

#[tokio::test]
async fn appel_reussi() {
    let mut client = server(&["--serveur-essai"]).await.unwrap();
    let output = client
        .call_tool("echo", json!({ "texte": "salut" }))
        .await
        .unwrap();
    assert_eq!(output.text, "salut");
    assert!(!output.is_error);
}

#[tokio::test]
async fn agent_appelle_echo_et_refuse_l_outil_sans_niveau() {
    let provider = ScriptedProvider::new()
        .reply(call("echo", json!({ "texte": "indice" })))
        .reply(call("vider_corbeille", json!({})))
        .then(|r| ModelReply::Text(last_tool_result(r).unwrap()));
    let mut agent = Agent::new(
        provider,
        servers().await.unwrap(),
        "Sherlock",
        AgentConfig::default(),
    );
    let answer = agent.ask("Enquête.").await.unwrap();
    assert!(answer.starts_with("courtier d'accords absent : l'outil vider_corbeille"));

    let history = agent.history();
    assert!(history.contains(&Message::ToolResult {
        call_id: "echo".into(),
        content: "indice".into(),
        is_error: false,
    }));
    assert!(!format!("{history:?}").contains(MARQUEUR_SANS_NIVEAU));
}

#[tokio::test]
async fn limite_de_tours() {
    let mut provider = ScriptedProvider::new();
    for _ in 0..5 {
        provider = provider.reply(call("echo", json!({ "texte": "encore" })));
    }
    let config = AgentConfig {
        max_turns: 2,
        ..AgentConfig::default()
    };
    let mut agent = Agent::new(provider, servers().await.unwrap(), "Sherlock", config);
    assert!(matches!(
        agent.ask("Boucle.").await,
        Err(AgentError::TurnLimit(2))
    ));
}

#[tokio::test]
async fn serveur_qui_plante() {
    let mut client = server(&["--serveur-essai", "--plante-sur-appel"])
        .await
        .unwrap();
    let error = client
        .call_tool("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Closed(_)), "{error}");
    // Le serveur mort ne fige pas le client : l'appel suivant échoue aussitôt.
    let error = client
        .call_tool("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Closed(_)), "{error}");
}

#[tokio::test]
async fn serveur_qui_plante_pendant_la_conversation() {
    let mut servers = McpServers::new();
    let client = server(&["--serveur-essai", "--plante-sur-appel"])
        .await
        .unwrap();
    servers.add(client).await.unwrap();
    assert_eq!(servers.tools().len(), 2);
    let provider = ScriptedProvider::new()
        .reply(call("echo", json!({ "texte": "x" })))
        .then(|r| ModelReply::Text(last_tool_result(r).unwrap()));
    let mut agent = Agent::new(provider, servers, "Sherlock", AgentConfig::default());
    assert_eq!(
        agent.ask("?").await.unwrap(),
        "le serveur essai s'est arrêté"
    );
}

#[tokio::test]
async fn serveur_introuvable() {
    let result = McpClient::spawn("absent", "/nulle/part", &[], TIMEOUT).await;
    assert!(matches!(result, Err(McpError::Spawn { .. })));
}

#[tokio::test]
async fn serveur_muet() {
    let delay = Duration::from_millis(200);
    let result = McpClient::spawn("muet", "sleep", &["10"], delay).await;
    assert!(matches!(result, Err(McpError::Timeout { .. })));
}
