//! Sherlock daemon. It has no service to provide yet: it can run a demonstration with the fake
//! model (`--demo`) or with Claude Code (`--claude-code`), serve the proxy (`--proxy`) and the
//! test MCP server (`--test-server`).

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig};
use sherlockd::claude_code::{ClaudeCode, ClaudeCodeConfig};
use sherlockd::mcp::{McpClient, McpServers, ServerConfig, ToolHost};
use sherlockd::model::{Message, ModelReply, ModelRequest, ToolCall};
use sherlockd::proxy;
use sherlockd::scripted::ScriptedProvider;
use sherlockd::test_server::{self, Mode};
use tracing_subscriber::EnvFilter;

const USAGE: &str =
    "usage: sherlockd --demo | --claude-code < request | --proxy | --test-server [case]";

/// The longest request read on standard input.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

const SYSTEM: &str =
    "Tu es Sherlock, l'agent de SherlockOS. Réponds en français, en tutoyant, par phrases courtes.";

fn main() -> anyhow::Result<()> {
    // Logs on standard error: standard output carries the MCP protocol or the conversation.
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
        ["--demo"] => demo(),
        // Never in the arguments: every local account reads them (`ps`, `/proc/*/cmdline`).
        ["--claude-code"] => claude_code(&read_request(std::io::stdin().lock())?),
        ["--claude-code", _] => {
            bail!("the request is read on standard input, never in the arguments")
        }
        ["--proxy"] => serve_proxy(),
        ["--test-server"] => test_server::run(Mode::Normal).context("test server"),
        ["--test-server", case] => {
            let mode = Mode::from_arg(case).with_context(|| format!("unknown case: {case}"))?;
            test_server::run(mode).context("test server")
        }
        _ => bail!("{USAGE}"),
    }
}

fn runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("creating the runtime")
}

fn exe() -> anyhow::Result<String> {
    let exe = std::env::current_exe().context("path of sherlockd")?;
    let exe = exe
        .to_str()
        .context("path of sherlockd is not valid UTF-8")?;
    Ok(exe.to_owned())
}

/// Sherlock's MCP servers. For now, only the test server, one of ours: its annotations are
/// trusted (ADR 0015).
async fn servers() -> anyhow::Result<McpServers> {
    let exe = exe()?;
    let config = ServerConfig::new("test", &exe, &["--test-server"])
        .timeout(Duration::from_secs(5))
        .ours();
    let client = McpClient::spawn(&config).await?;
    let mut servers = McpServers::new();
    servers.add(client).await.context("tool discovery")?;
    Ok(servers)
}

/// The request on `input`, to its end: `MAX_REQUEST_BYTES` at most, in UTF-8, not empty.
fn read_request(input: impl std::io::Read) -> anyhow::Result<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    input
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("reading the request")?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        bail!("request longer than {MAX_REQUEST_BYTES} bytes");
    }
    let request = String::from_utf8(bytes).context("request not in UTF-8")?;
    let request = request.trim();
    if request.is_empty() {
        bail!("empty request on standard input");
    }
    Ok(request.to_owned())
}

/// Serves the proxy on standard input and output, for Claude Code.
fn serve_proxy() -> anyhow::Result<()> {
    // Started by Claude Code: never outlives it, even killed.
    sherlockd::process::die_with_own_parent().context("tying the proxy to Claude Code")?;
    runtime()?.block_on(async {
        let mut servers = servers().await?;
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        let served = proxy::serve(input, tokio::io::stdout(), &mut servers).await;
        servers.shutdown().await;
        served.context("serving the proxy")
    })
}

/// Hands the request to Claude Code, plugged into the proxy, and prints the conversation.
fn claude_code(request: &str) -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let mut claude = ClaudeCode::new(ClaudeCodeConfig::new(&exe()?, SYSTEM));
        let answer = claude.ask(request).await;
        print_history(claude.history())?;
        answer?;
        Ok(())
    })
}

/// Runs a fake model script against the test server and prints the conversation.
fn demo() -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let servers = servers().await?;
        let mut out = std::io::stdout().lock();
        // The conversation is the command's output, not a log: it goes to stdout.
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

        let mut agent = Agent::new(script(), servers, SYSTEM, AgentConfig::default());
        agent
            .ask("Répète « Bonjour » avec l'outil echo, puis vide la corbeille.")
            .await
            .context("conversation")?;
        print_history(agent.history())
    })
}

/// Prints the conversation on standard output: it is the command's output.
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

/// The fake model calls `echo`, then the unleveled tool, then answers with `echo`'s result.
fn script() -> ScriptedProvider {
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
            let refusal = result("2").unwrap_or_default();
            ModelReply::Text(format!(
                "Le serveur de test répond « {echo} ». Je n'ai pas vidé la corbeille ({refusal})."
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_comes_whole_and_bounded() {
        assert_eq!(
            read_request("  Range mes photos\n".as_bytes()).unwrap(),
            "Range mes photos"
        );
        assert!(read_request("\n".as_bytes()).is_err());
        assert!(read_request(&b"caf\xe9"[..]).is_err());
        let long = "a".repeat(MAX_REQUEST_BYTES as usize + 1);
        assert!(read_request(long.as_bytes()).is_err());
    }
}
