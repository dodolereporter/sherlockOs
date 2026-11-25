//! Client MCP et boucle d'agent contre le serveur de test, lancé en vrai processus, correct ou
//! hostile.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig, AgentError};
use sherlockd::faux::{ScriptedProvider, last_tool_result};
use sherlockd::mcp::{Level, McpClient, McpError, McpServers, ServerConfig, ToolHost};
use sherlockd::modele::{Message, ModelReply, ToolCall};
use sherlockd::serveur_essai::{CONSIGNE_CACHEE, MARQUEUR_SANS_NIVEAU};

const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");
const TIMEOUT: Duration = Duration::from_secs(5);

/// Le serveur de test, dans le cas donné, marqué comme l'un des nôtres.
fn config(case: Option<&str>) -> ServerConfig {
    let args: Vec<&str> = ["--serveur-essai"].into_iter().chain(case).collect();
    ServerConfig::new("essai", EXE, &args)
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

fn echo(texte: &str) -> ModelReply {
    call("echo", json!({ "texte": texte }))
}

fn levels(servers: &McpServers) -> Vec<(&str, Level)> {
    servers
        .tools()
        .iter()
        .map(|t| (t.name.as_str(), t.level))
        .collect()
}

#[tokio::test]
async fn liste_les_outils_et_leur_niveau() {
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
async fn serveur_tiers_tout_destructif() {
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
async fn appel_reussi() {
    let mut servers = servers(None).await.unwrap();
    let output = servers
        .call("echo", json!({ "texte": "salut" }))
        .await
        .unwrap();
    assert_eq!(output.text, "salut");
    assert!(!output.is_error);
}

#[tokio::test]
async fn appel_direct_hors_lecture_refuse() {
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
    assert!(!error.to_string().contains(MARQUEUR_SANS_NIVEAU));
    // Le serveur reste utilisable : le refus n'a rien envoyé.
    assert!(servers.call("echo", json!({ "texte": "x" })).await.is_ok());
}

#[tokio::test]
async fn agent_appelle_echo_et_refuse_l_outil_sans_niveau() {
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
    assert!(!format!("{history:?}").contains(MARQUEUR_SANS_NIVEAU));
}

#[tokio::test]
async fn erreur_json_rpc_reste_une_donnee() {
    let provider = ScriptedProvider::new()
        .reply(echo("x"))
        .reply(ModelReply::Text("Fini.".into()));
    let servers = servers(Some("erreur-consigne")).await.unwrap();
    let mut agent = Agent::new(provider, servers, "Sherlock", AgentConfig::default());
    agent.ask("?").await.unwrap();
    for request in &agent.provider().requests {
        assert_eq!(request.system, "Sherlock");
    }
    let results: Vec<_> = agent
        .history()
        .iter()
        .filter(|m| format!("{m:?}").contains(CONSIGNE_CACHEE))
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
async fn limite_de_tours() {
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
async fn serveur_qui_plante() {
    let mut servers = servers(Some("plante-sur-appel")).await.unwrap();
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Closed(_)), "{error}");
    // Le serveur mort ne fige pas le client : l'appel suivant échoue aussitôt.
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Lost(_)), "{error}");
}

#[tokio::test]
async fn serveur_qui_plante_pendant_la_conversation() {
    let provider = ScriptedProvider::new()
        .reply(echo("x"))
        .then(|r| ModelReply::Text(last_tool_result(r).unwrap()));
    let servers = servers(Some("plante-sur-appel")).await.unwrap();
    let mut agent = Agent::new(provider, servers, "Sherlock", AgentConfig::default());
    assert_eq!(
        agent.ask("?").await.unwrap(),
        "le serveur essai s'est arrêté"
    );
}

#[tokio::test]
async fn delai_depasse_serveur_arrete_et_attendu() {
    let config = config(Some("muet-sur-appel")).timeout(Duration::from_millis(300));
    let client = McpClient::spawn(&config).await.unwrap();
    let pid = client.pid().unwrap();
    let mut servers = McpServers::new();
    servers.add(client).await.unwrap();
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Timeout { .. }), "{error}");
    // Ni processus ni zombie : le serveur a été arrêté puis attendu.
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    let error = servers
        .call("echo", json!({ "texte": "x" }))
        .await
        .unwrap_err();
    assert!(matches!(error, McpError::Lost(_)), "{error}");
}

#[tokio::test]
async fn liste_qui_n_est_pas_un_objet() {
    let error = servers(Some("liste-non-objet")).await.err().unwrap();
    assert!(matches!(error, McpError::Protocol { .. }), "{error}");
}

#[tokio::test]
async fn doublon_refuse() {
    for case in ["doublon-page", "doublon-pages"] {
        let error = servers(Some(case)).await.err().unwrap();
        assert!(
            matches!(&error, McpError::DuplicateTool(name) if name == "echo"),
            "{case} : {error}"
        );
    }
}

#[tokio::test]
async fn pagination_sans_fin_bornee() {
    let start = Instant::now();
    let error = servers(Some("pagination-sans-fin")).await.err().unwrap();
    assert!(matches!(&error, McpError::Protocol { detail, .. } if detail.contains("pages")));
    assert!(start.elapsed() < TIMEOUT);
}

#[tokio::test]
async fn ligne_geante_refusee() {
    let error = servers(Some("ligne-geante")).await.err().unwrap();
    assert!(
        matches!(&error, McpError::Protocol { detail, .. } if detail.contains("octets")),
        "{error}"
    );
}

#[tokio::test]
async fn serveur_introuvable() {
    let result = McpClient::spawn(&ServerConfig::new("absent", "/nulle/part", &[])).await;
    assert!(matches!(result, Err(McpError::Spawn { .. })));
}

#[tokio::test]
async fn serveur_muet() {
    let config = ServerConfig::new("muet", "sleep", &["10"]).timeout(Duration::from_millis(200));
    let result = McpClient::spawn(&config).await;
    assert!(matches!(result, Err(McpError::Timeout { .. })));
}
