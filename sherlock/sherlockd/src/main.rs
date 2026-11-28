//! Sherlock daemon. It has no service to provide yet: it can run a demonstration with the fake
//! model (`--demo`) or with Claude Code (`--claude-code`), serve the proxy (`--proxy`) and the
//! test MCP server (`--test-server`).

use std::io::Write;
use std::time::Duration;

use anyhow::{Context, bail};
use serde_json::json;
use sherlockd::agent::{Agent, AgentConfig};
use sherlockd::broker::{Broker, Consent, Journal, Nobody};
use sherlockd::claude_code::{ClaudeCode, ClaudeCodeConfig};
use sherlockd::mcp::{McpClient, McpServers, ServerConfig, ToolHost};
use sherlockd::model::{Message, ModelReply, ModelRequest, ToolCall};
use sherlockd::proxy;
use sherlockd::scripted::ScriptedProvider;
use sherlockd::test_server::{self, Mode};
use sherlockd::window::CompositorWindow;
use sherlockd::windows_server::{self, Compositor};
use sherlockd::{containers_server, files_server};
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: sherlockd --demo | --claude-code < request | --ask < request | --proxy \
     | --test-server [case] | --windows-server <socket> | --files-server [--settings <file>] \
     | --containers-server [--socket <path>]";

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
        // The shell's panel (A-19): the request on standard input, the events as JSON lines.
        ["--ask"] => ask(&read_input(std::io::stdin().lock())?),
        ["--claude-code", _] | ["--ask", _] => {
            bail!("the request is read on standard input, never in the arguments")
        }
        ["--proxy"] => serve_proxy(CompositorWindow::of_session(), None),
        // Claude Code starts the proxy without the Wayland variables: the socket of the
        // consent window is given to it (A-11c), and the session of the panel's conversation
        // its journal lines carry (A-20e).
        ["--proxy", "--consent", path] => {
            serve_proxy(Some(CompositorWindow::at(std::path::Path::new(path))), None)
        }
        ["--proxy", "--session", session] => {
            serve_proxy(CompositorWindow::of_session(), Some(session))
        }
        ["--proxy", "--consent", path, "--session", session] => serve_proxy(
            Some(CompositorWindow::at(std::path::Path::new(path))),
            Some(session),
        ),
        ["--test-server"] => test_server::run(Mode::Normal).context("test server"),
        ["--test-server", "--trace", trace] => {
            test_server::run_traced(Mode::Normal, Some(std::path::Path::new(trace)))
                .context("test server")
        }
        ["--test-server", case] => {
            let mode = Mode::from_arg(case).with_context(|| format!("unknown case: {case}"))?;
            test_server::run(mode).context("test server")
        }
        // Started by sherlockd itself, with the compositor's windows socket (A-12).
        ["--windows-server", socket] => {
            windows_server::run(&Compositor::at(std::path::Path::new(socket)))
                .context("windows server")
        }
        // For the tests, whose fake compositor is the test itself.
        ["--windows-server", "--any-peer", socket] => {
            let compositor = Compositor::at(std::path::Path::new(socket)).trusting_any_program();
            windows_server::run(&compositor).context("windows server")
        }
        // Started by sherlockd itself, in the folders of the session's settings (A-13).
        ["--files-server"] => files_server::run(None).context("files server"),
        ["--files-server", "--settings", settings] => {
            files_server::run(Some(std::path::Path::new(settings))).context("files server")
        }
        // Started by sherlockd itself, on the session's Docker or Podman daemon (A-14).
        ["--containers-server"] => containers_server::run(None).context("containers server"),
        ["--containers-server", "--socket", socket] => {
            containers_server::run(Some(std::path::Path::new(socket))).context("containers server")
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

/// Sherlock's MCP servers: the test server, the files (A-13) and containers (A-14) servers and,
/// in a Wayland session, the windows server (A-12), all ours: their annotations are trusted (ADR 0015).
async fn servers(
    window: Option<CompositorWindow>,
    session: Option<&str>,
) -> anyhow::Result<McpServers> {
    let exe = exe()?;
    // The windows of the session (A-12), through the compositor's windows socket.
    let windows = window.as_ref().and_then(CompositorWindow::windows_socket);
    let config = ServerConfig::new("test", &exe, &["--test-server"])
        .timeout(Duration::from_secs(5))
        .ours();
    let client = McpClient::spawn(&config).await?;
    // The compositor's consent window asks the user (A-11c); outside a Wayland session, nobody
    // is asked and only reads run. Every decision is journaled; without a journal, reads still
    // run, actions never.
    let journal = match Journal::default_path().map(|path| Journal::open(&path)) {
        Some(Ok(journal)) => journal,
        Some(Err(error)) => {
            tracing::error!(%error, "consent journal unavailable: actions refused");
            Journal::none()
        }
        None => Journal::none(),
    };
    let journal = match session {
        Some(session) => journal.for_session(session),
        None => journal,
    };
    let consent: Box<dyn Consent> = match window {
        Some(window) => Box::new(window),
        None => Box::new(Nobody),
    };
    let mut servers = McpServers::with_broker(Broker::new(consent, journal));
    servers.add(client).await.context("tool discovery")?;
    // The files of the allowed folders (A-13).
    let config = ServerConfig::new("files", &exe, &["--files-server"])
        .timeout(Duration::from_secs(30))
        .ours();
    match McpClient::spawn(&config).await {
        Ok(client) => servers.add(client).await.context("files tools")?,
        Err(error) => tracing::warn!(%error, "no files server"),
    }
    // The containers of the Docker or Podman daemon (A-14).
    let config = ServerConfig::new("containers", &exe, &["--containers-server"])
        .timeout(Duration::from_secs(60))
        .ours();
    match McpClient::spawn(&config).await {
        Ok(client) => servers.add(client).await.context("containers tools")?,
        Err(error) => tracing::warn!(%error, "no containers server"),
    }
    if let Some(socket) = windows.as_deref().and_then(std::path::Path::to_str) {
        let config = ServerConfig::new("windows", &exe, &["--windows-server", socket])
            .timeout(Duration::from_secs(10))
            .ours();
        match McpClient::spawn(&config).await {
            Ok(client) => servers.add(client).await.context("windows tools")?,
            Err(error) => tracing::warn!(%error, "no windows server"),
        }
    }
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

/// The shell's input on `input`: `shell::MAX_INPUT_BYTES` at most.
fn read_input(input: impl std::io::Read) -> anyhow::Result<sherlockd::shell::Input> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    let max = sherlockd::shell::MAX_INPUT_BYTES as u64;
    input
        .take(max + 1)
        .read_to_end(&mut bytes)
        .context("reading the request")?;
    sherlockd::shell::Input::parse(&bytes).context("empty, too long or not UTF-8 request")
}

/// Writes `events` as JSON lines, at once: the lock of standard output keeps lines whole.
fn emit(events: Vec<serde_json::Value>) {
    let mut stdout = std::io::stdout().lock();
    for event in events {
        let _ = writeln!(stdout, "{event}");
    }
    let _ = stdout.flush();
}

/// The shell's request (A-19): the conversation as JSON lines, as it happens, the steps
/// following the consent journal, then the answer or why there is none.
fn ask(input: &sherlockd::shell::Input) -> anyhow::Result<()> {
    use sherlockd::shell::{Events, Follower, reason};
    use std::sync::{Arc, Mutex, PoisonError};
    runtime()?.block_on(async {
        // The plan announced first, for the shell's panel (lot A-19e).
        let system = format!("{SYSTEM} {}", sherlockd::shell::PLAN_INSTRUCTIONS);
        let mut config = ClaudeCodeConfig::new(&exe()?, &system);
        if let Some(path) =
            CompositorWindow::of_session().and_then(|w| w.path().to_str().map(String::from))
        {
            config.proxy_args = vec!["--proxy".into(), "--consent".into(), path];
        }
        // The conversation's session, the panel's, or one of this request's own: the proxy
        // writes it on each journal line, and only those lines count here (A-20e).
        let session = input
            .session
            .clone()
            .unwrap_or_else(sherlockd::broker::new_session);
        config
            .proxy_args
            .extend(["--session".into(), session.clone()]);
        let mut claude = ClaudeCode::new(config);
        for (request, answer) in &input.history {
            claude.remember(request, answer);
        }
        let events = Arc::new(Mutex::new(Events::default().for_session(&session)));
        let mut follower = Follower::new(Journal::default_path());
        let journal = {
            let events = events.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    for line in follower.lines() {
                        let shown = events
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .journal(&line);
                        emit(shown);
                    }
                }
            })
        };
        emit(vec![json!({ "event": "started" })]);
        let shown = events.clone();
        let answer = claude
            .ask_with(&input.request, &mut |message| {
                let lines = shown
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .message(message);
                emit(lines);
            })
            .await;
        journal.abort();
        let last = match answer {
            Ok(text) => json!({ "event": "answer", "text": text }),
            Err(error) => {
                tracing::warn!(%error, "no answer");
                json!({ "event": "failed", "reason": reason(&error), "message": error.to_string() })
            }
        };
        emit(vec![last]);
        Ok(())
    })
}

/// Serves the proxy on standard input and output, for Claude Code.
fn serve_proxy(window: Option<CompositorWindow>, session: Option<&str>) -> anyhow::Result<()> {
    // Started by Claude Code: never outlives it, even killed.
    sherlockd::process::die_with_own_parent().context("tying the proxy to Claude Code")?;
    runtime()?.block_on(async {
        let mut servers = servers(window, session).await?;
        let input = tokio::io::BufReader::new(tokio::io::stdin());
        let served = proxy::serve(input, tokio::io::stdout(), &mut servers).await;
        servers.shutdown().await;
        served.context("serving the proxy")
    })
}

/// Hands the request to Claude Code, plugged into the proxy, and prints the conversation.
fn claude_code(request: &str) -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let mut config = ClaudeCodeConfig::new(&exe()?, SYSTEM);
        // The proxy asks the session's consent window: Claude Code does not pass it the
        // Wayland variables, so its socket goes in the proxy's arguments (A-11c).
        if let Some(path) =
            CompositorWindow::of_session().and_then(|w| w.path().to_str().map(String::from))
        {
            config.proxy_args = vec!["--proxy".into(), "--consent".into(), path];
        }
        let mut claude = ClaudeCode::new(config);
        let answer = claude.ask(request).await;
        print_history(claude.history())?;
        answer?;
        Ok(())
    })
}

/// Runs a fake model script against the test server and prints the conversation.
fn demo() -> anyhow::Result<()> {
    runtime()?.block_on(async {
        let servers = servers(CompositorWindow::of_session(), None).await?;
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
