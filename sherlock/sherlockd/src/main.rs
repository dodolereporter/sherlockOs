//! Démon de Sherlock. Il n'a pas encore de service à rendre : il sait dérouler une démonstration
//! avec le faux modèle (`--essai`) ou avec Claude Code (`--claude-code`), servir le mandataire
//! (`--mandataire`) et le serveur MCP de test (`--serveur-essai`).

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig};
use sherlockd::claude_code::{ClaudeCode, ClaudeCodeConfig};
use sherlockd::faux::ScriptedProvider;
use sherlockd::mandataire;
use sherlockd::mcp::{McpClient, McpServers, ServerConfig, ToolHost};
use sherlockd::modele::{Message, ModelReply, ModelRequest, ToolCall};
use sherlockd::serveur_essai::{self, Mode};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage : sherlockd --essai | --claude-code <demande> | --mandataire \
                     | --serveur-essai [cas]";

const SYSTEM: &str =
    "Tu es Sherlock, l'agent de NOM_OS. Réponds en français, en tutoyant, par phrases courtes.";

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
        ["--claude-code", request] => claude_code(request),
        ["--mandataire"] => serve_proxy(),
        ["--serveur-essai"] => serveur_essai::run(Mode::Normal).context("serveur d'essai"),
        ["--serveur-essai", case] => {
            let mode = Mode::from_arg(case).with_context(|| format!("cas inconnu : {case}"))?;
            serveur_essai::run(mode).context("serveur d'essai")
        }
        _ => bail!("{USAGE}"),
    }
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("création du runtime")
}

fn exe() -> anyhow::Result<String> {
    let exe = std::env::current_exe().context("chemin de sherlockd")?;
    let exe = exe.to_str().context("chemin de sherlockd illisible")?;
    Ok(exe.to_owned())
}

/// Les serveurs MCP de Sherlock. Pour l'instant, le seul serveur de test, l'un des nôtres : ses
/// annotations sont crues (ADR 0015).
async fn servers() -> anyhow::Result<McpServers> {
    let exe = exe()?;
    let config = ServerConfig::new("essai", &exe, &["--serveur-essai"])
        .timeout(Duration::from_secs(5))
        .ours();
    let client = McpClient::spawn(&config).await?;
    let mut servers = McpServers::new();
    servers.add(client).await.context("découverte des outils")?;
    Ok(servers)
}

/// Sert le mandataire sur l'entrée et la sortie standard, pour Claude Code.
fn serve_proxy() -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let mut servers = servers().await?;
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        let served = mandataire::serve(input, tokio::io::stdout(), &mut servers).await;
        servers.shutdown().await;
        served.context("service du mandataire")
    })
}

/// Confie la demande à Claude Code, branché sur le mandataire, et affiche la conversation.
fn claude_code(request: &str) -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let mut claude = ClaudeCode::new(ClaudeCodeConfig::new(&exe()?, SYSTEM));
        let answer = claude.ask(request).await;
        print_history(claude.history())?;
        answer?;
        Ok(())
    })
}

/// Déroule un scénario du faux modèle contre le serveur de test et affiche la conversation.
fn essai() -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let servers = servers().await?;
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
        writeln!(out)?;
        drop(out);

        let mut agent = Agent::new(scenario(), servers, SYSTEM, AgentConfig::default());
        agent
            .ask("Répète « Bonjour » avec l'outil echo, puis vide la corbeille.")
            .await
            .context("conversation")?;
        print_history(agent.history())
    })
}

/// Affiche la conversation sur la sortie standard : c'est la sortie de la commande.
fn print_history(history: &[Message]) -> anyhow::Result<()> {
    let mut out = std::io::stdout().lock();
    for message in history {
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
