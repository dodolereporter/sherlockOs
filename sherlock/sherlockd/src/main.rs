//! Démon de Sherlock. Le lot A-10 ne pose que la boucle d'agent : le démon n'a pas encore de
//! service à rendre, il sait dérouler une démonstration (`--essai`) et servir le serveur MCP de
//! test (`--serveur-essai`).

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig};
use sherlockd::faux::ScriptedProvider;
use sherlockd::mcp::{McpClient, McpServers, ToolHost};
use sherlockd::modele::{Message, ModelReply, ModelRequest, ToolCall};
use sherlockd::serveur_essai;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage : sherlockd --essai | --serveur-essai [--plante-sur-appel]";

fn main() -> anyhow::Result<()> {
    // Journaux sur la sortie d'erreur : la sortie standard porte le protocole MCP ou la conversation.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--essai"] => essai(),
        ["--serveur-essai"] => serveur_essai::run(false).context("serveur d'essai"),
        ["--serveur-essai", "--plante-sur-appel"] => {
            serveur_essai::run(true).context("serveur d'essai")
        }
        _ => bail!("{USAGE}"),
    }
}

/// Déroule un scénario du faux modèle contre le serveur de test et affiche la conversation.
fn essai() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("création du runtime")?;
    runtime.block_on(async {
        let exe = std::env::current_exe().context("chemin de sherlockd")?;
        let exe = exe.to_str().context("chemin de sherlockd illisible")?;
        let timeout = Duration::from_secs(5);
        let client = McpClient::spawn("essai", exe, &["--serveur-essai"], timeout).await?;
        let mut servers = McpServers::new();
        servers.add(client).await.context("découverte des outils")?;

        let mut out = std::io::stdout().lock();
        // La conversation est la sortie de la commande, pas un journal : elle va sur stdout.
        writeln!(out, "Outils du serveur de test :")?;
        for tool in servers.tools() {
            writeln!(
                out,
                "  {} ({}) : {}",
                tool.name, tool.level, tool.description
            )?;
        }

        let system = "Tu es Sherlock, l'agent de NOM_OS. Réponds en français.";
        let mut agent = Agent::new(scenario(), servers, system, AgentConfig::default());
        agent
            .ask("Répète « Bonjour » avec l'outil echo, puis vide la corbeille.")
            .await
            .context("conversation")?;

        writeln!(out)?;
        for message in agent.history() {
            match message {
                Message::User(text) => writeln!(out, "Toi : {text}")?,
                Message::Assistant(ModelReply::Text(text)) => writeln!(out, "Sherlock : {text}")?,
                Message::Assistant(ModelReply::ToolCalls(calls)) => {
                    for call in calls {
                        writeln!(out, "Sherlock appelle {} {}", call.name, call.arguments)?;
                    }
                }
                Message::ToolResult {
                    content,
                    is_error: false,
                    ..
                } => writeln!(out, "  résultat : {content}")?,
                Message::ToolResult {
                    content,
                    is_error: true,
                    ..
                } => writeln!(out, "  refus : {content}")?,
            }
        }
        Ok(())
    })
}

/// Le faux modèle appelle `echo`, puis l'outil sans niveau, puis répond avec le résultat d'`echo`.
fn scenario() -> ScriptedProvider {
    let call = |id: &str, name: &str, arguments| {
        ModelReply::ToolCalls(vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
        }])
    };
    ScriptedProvider::new()
        .reply(call("1", "echo", json!({ "texte": "Bonjour" })))
        .reply(call("2", "vider_corbeille", json!({})))
        .then(|request: &ModelRequest<'_>| {
            let result = |id: &str| {
                request.messages.iter().find_map(|m| match m {
                    Message::ToolResult {
                        call_id, content, ..
                    } if call_id == id => Some(content.clone()),
                    _ => None,
                })
            };
            let echo = result("1").unwrap_or_default();
            let refus = result("2").unwrap_or_default();
            ModelReply::Text(format!(
                "Le serveur de test répond « {echo} ». Je n'ai pas vidé la corbeille ({refus})."
            ))
        })
}
