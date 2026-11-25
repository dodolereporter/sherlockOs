//! Minimal MCP client: JSON-RPC 2.0 over a server's standard input and output.
//!
//! The bare minimum of the protocol: `initialize`, `notifications/initialized`, `tools/list`
//! and `tools/call`, one JSON message per line. The server is a child process. Everything it
//! sends is external data: nothing may make the client panic or grow without bound, and any
//! doubt about a level resolves to the most restrictive one.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// MCP protocol version requested from the server.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Maximum size of a message read, line ending included: 4 MiB. Beyond it, the server is lost.
/// Bounds memory against a hostile server, while leaving room for a large but legitimate tool
/// result.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Maximum number of `tools/list` pages. Beyond it, the list is refused.
pub const MAX_TOOL_PAGES: usize = 16;

/// Maximum number of tools of one server. Beyond it, the list is refused.
pub const MAX_TOOLS_PER_SERVER: usize = 256;

/// Time left to the server to stop on its own, input closed, before `SIGKILL`.
pub const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Default timeout of a request to the server.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum length of a server message copied into a log.
const LOG_PREVIEW_BYTES: usize = 200;

/// Environment variables passed to the processes started by `sherlockd`: paths, session
/// identity, language (`LC_*` too), terminal, log level. No key, no token, no variable that
/// changes where a request goes: everything else is removed.
pub const ENV_ALLOWED: [&str; 16] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LANGUAGE",
    "TZ",
    "TERM",
    "TMPDIR",
    "XDG_RUNTIME_DIR",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "RUST_LOG",
];

/// Keeps, from `vars`, the variables of [`ENV_ALLOWED`], the `LC_*` ones and those of `extra`.
pub fn clean_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    extra: &[&str],
) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(name, _)| {
            name.to_str().is_some_and(|name| {
                ENV_ALLOWED.contains(&name) || extra.contains(&name) || name.starts_with("LC_")
            })
        })
        .collect()
}

/// Level of a tool, which decides the consent rule (context, « Les trois niveaux »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Reads without changing anything. Free, logged.
    Read,
    /// Changes the system state. User consent.
    Action,
    /// Erases or destroys. Reinforced consent. Also the level of any doubtful tool.
    Destructive,
}

impl Level {
    /// Reads the level from the tool's standard annotations (ADR 0015).
    ///
    /// `readOnlyHint: true` gives read, `destructiveHint: false` action. Everything else is
    /// destructive: no annotations, a value that is not a boolean, or a contradiction
    /// (`readOnlyHint` and `destructiveHint` both true).
    pub fn from_annotations(annotations: Option<&Value>) -> Self {
        // `None`: absent; `Some(None)`: present but not a boolean.
        let hint = |name| annotations.and_then(|a| a.get(name)).map(Value::as_bool);
        match (hint("readOnlyHint"), hint("destructiveHint")) {
            (Some(Some(true)), None | Some(Some(false))) => Self::Read,
            (None | Some(Some(false)), Some(Some(false))) => Self::Action,
            _ => Self::Destructive,
        }
    }
}

impl Level {
    /// Standard annotations that give back this level, to expose the tool to an MCP client.
    pub fn annotations(self) -> Value {
        match self {
            Self::Read => json!({ "readOnlyHint": true }),
            Self::Action => json!({ "readOnlyHint": false, "destructiveHint": false }),
            Self::Destructive => json!({ "readOnlyHint": false, "destructiveHint": true }),
        }
    }
}

// In French: the model reads the level in refusals, and the demo prints it.
impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "lecture",
            Self::Action => "action",
            Self::Destructive => "destructif",
        })
    }
}

/// Trust granted to a server's annotations (ADR 0015).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Trust {
    /// Third-party server: its annotations are ignored, all its tools are destructive.
    #[default]
    ThirdParty,
    /// Server shipped with the OS: its annotations are project code, read as they are.
    Ours,
}

/// How to start an MCP server.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Server name, in logs and errors.
    pub name: String,
    /// Program to start.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Timeout of each request.
    pub timeout: Duration,
    /// Third party by default: only an explicit mark makes its annotations trusted.
    pub trust: Trust,
}

impl ServerConfig {
    /// A third-party server, with the default timeout.
    pub fn new(name: &str, program: &str, args: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            timeout: DEFAULT_TIMEOUT,
            trust: Trust::ThirdParty,
        }
    }

    /// Marks the server as one of ours.
    pub fn ours(mut self) -> Self {
        self.trust = Trust::Ours;
        self
    }

    /// Changes the request timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

/// A tool exposed by an MCP server.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Unique tool name.
    pub name: String,
    /// Description meant for the model.
    pub description: String,
    /// JSON schema of the arguments.
    pub input_schema: Value,
    /// Level read from the annotations, or destructive for a third-party server.
    pub level: Level,
}

/// Result of a tool call.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// The text blocks of the result, joined together.
    pub text: String,
    /// True if the server reports that the tool failed.
    pub is_error: bool,
}

/// MCP client error. The messages are in French: an error of a tool call is returned to the
/// model as its result.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// The server program could not be started.
    #[error("le serveur {server} ne se lance pas : {source}")]
    Spawn {
        server: String,
        source: std::io::Error,
    },
    /// The server closed its output: it stopped or crashed.
    #[error("le serveur {0} s'est arrêté")]
    Closed(String),
    /// The server was stopped after an error, it receives nothing anymore.
    #[error("le serveur {0} est perdu")]
    Lost(String),
    /// The server did not answer in time.
    #[error("le serveur {server} n'a pas répondu à {method} en {delay:?}")]
    Timeout {
        server: String,
        method: String,
        delay: Duration,
    },
    /// The server sent a message the client refuses.
    #[error("réponse invalide du serveur {server} : {detail}")]
    Protocol { server: String, detail: String },
    /// The server answered with a JSON-RPC error.
    #[error("le serveur {server} refuse {method} : {message} (code {code})")]
    Rpc {
        server: String,
        method: String,
        code: i64,
        message: String,
    },
    /// A tool name appears twice, within one server or across two servers.
    #[error("l'outil {0} est exposé deux fois")]
    DuplicateTool(String),
    /// No server exposes this tool.
    #[error("outil inconnu : {0}")]
    UnknownTool(String),
    /// The tool's level requires consent, and the broker (lot A-11) does not exist yet.
    #[error(
        "courtier d'accords absent : l'outil {tool} est de niveau {level}, \
         seuls les outils de lecture s'exécutent"
    )]
    BrokerAbsent { tool: String, level: Level },
}

impl McpError {
    /// True if, after this error, the exchange with the server is no longer reliable.
    fn loses_server(&self) -> bool {
        matches!(
            self,
            Self::Closed(_) | Self::Timeout { .. } | Self::Protocol { .. }
        )
    }
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct Incoming {
    id: Option<Value>,
    method: Option<String>,
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTool {
    name: String,
    #[serde(default)]
    description: String,
    input_schema: Value,
    annotations: Option<Value>,
}

struct Connection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

/// Connection to an MCP server started as a child process.
pub struct McpClient {
    name: String,
    // `None` once the server is lost and stopped.
    connection: Option<Connection>,
    next_id: u64,
    timeout: Duration,
    trust: Trust,
}

impl McpClient {
    /// Starts the server and performs the initialization.
    pub async fn spawn(config: &ServerConfig) -> Result<Self, McpError> {
        let spawn_err = |source| McpError::Spawn {
            server: config.name.clone(),
            source,
        };
        let mut child = Command::new(&config.program)
            .args(&config.args)
            // A server receives no key and no token from the session.
            .env_clear()
            .envs(clean_env(std::env::vars_os(), &[]))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Safety net if the client goes away without `shutdown`.
            .kill_on_drop(true)
            .spawn()
            .map_err(spawn_err)?;
        let missing = || std::io::Error::other("tube standard absent");
        let stdin = child.stdin.take().ok_or_else(missing).map_err(spawn_err)?;
        let stdout = child.stdout.take().ok_or_else(missing).map_err(spawn_err)?;
        let mut client = Self {
            name: config.name.clone(),
            connection: Some(Connection {
                child,
                stdin,
                stdout: BufReader::new(stdout),
            }),
            next_id: 1,
            timeout: config.timeout,
            trust: config.trust,
        };
        if let Err(error) = client.initialize().await {
            client.shutdown().await;
            return Err(error);
        }
        Ok(client)
    }

    /// Server name, as given at startup.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Process identifier of the server, `None` once the server is stopped and reaped.
    pub fn pid(&self) -> Option<u32> {
        self.connection.as_ref().and_then(|c| c.child.id())
    }

    async fn initialize(&mut self) -> Result<(), McpError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": { "name": "sherlockd", "version": env!("CARGO_PKG_VERSION") },
        });
        let result = self.request("initialize", params).await?;
        let version = result.get("protocolVersion").and_then(Value::as_str);
        if version != Some(PROTOCOL_VERSION) {
            return Err(self.protocol(format!("version du protocole {version:?}")));
        }
        self.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await
    }

    /// Lists the server's tools, page after page, within the caps.
    pub async fn list_tools(&mut self) -> Result<Vec<ToolSpec>, McpError> {
        let mut tools = Vec::new();
        let mut names = HashSet::new();
        let mut cursor: Option<Value> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor.map_or_else(|| json!({}), |c| json!({ "cursor": c }));
            let mut result = self.request("tools/list", params).await?;
            let raw = result.get_mut("tools").map(Value::take);
            let raw = raw.ok_or_else(|| self.protocol("tools/list sans liste d'outils".into()))?;
            let raw: Vec<RawTool> = serde_json::from_value(raw)
                .map_err(|e| self.protocol(format!("tools/list : {e}")))?;
            for tool in raw {
                if !names.insert(tool.name.clone()) {
                    return Err(McpError::DuplicateTool(tool.name));
                }
                if tools.len() == MAX_TOOLS_PER_SERVER {
                    let detail = format!("plus de {MAX_TOOLS_PER_SERVER} outils");
                    return Err(self.protocol(detail));
                }
                let level = match self.trust {
                    Trust::Ours => Level::from_annotations(tool.annotations.as_ref()),
                    Trust::ThirdParty => Level::Destructive,
                };
                tools.push(ToolSpec {
                    name: tool.name,
                    description: tool.description,
                    input_schema: tool.input_schema,
                    level,
                });
            }
            match result.get("nextCursor") {
                Some(next) if !next.is_null() => cursor = Some(next.clone()),
                _ => return Ok(tools),
            }
        }
        Err(self.protocol(format!("plus de {MAX_TOOL_PAGES} pages d'outils")))
    }

    /// Calls a tool and returns its text blocks. Without a level check: only
    /// [`McpServers::call`], which checks it, uses it.
    pub(crate) async fn call_tool(
        &mut self,
        tool: &str,
        arguments: Value,
    ) -> Result<ToolOutput, McpError> {
        let params = json!({ "name": tool, "arguments": arguments });
        let result = self.request("tools/call", params).await?;
        let content = result.get("content").and_then(Value::as_array);
        let content = content.ok_or_else(|| self.protocol("tools/call sans contenu".into()))?;
        let text = content
            .iter()
            .map(|block| match block.get("text").and_then(Value::as_str) {
                Some(text) => text.to_owned(),
                None => "[bloc non textuel ignoré]".to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(ToolOutput { text, is_error })
    }

    /// Sends a request and waits for its response. After a stop, a timeout or a refused message,
    /// the exchange is no longer reliable: the server is stopped.
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        let delay = self.timeout;
        let exchange = async {
            self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
                .await?;
            self.wait_response(id, method).await
        };
        let result = tokio::time::timeout(delay, exchange)
            .await
            .unwrap_or_else(|_| {
                Err(McpError::Timeout {
                    server: self.name.clone(),
                    method: method.to_owned(),
                    delay,
                })
            });
        if let Err(error) = &result
            && error.loses_server()
        {
            tracing::warn!(server = %self.name, %error, "server lost, stopping");
            self.shutdown().await;
        }
        result
    }

    async fn wait_response(&mut self, id: u64, method: &str) -> Result<Value, McpError> {
        loop {
            let line = self.read_message().await?;
            let message: Incoming = serde_json::from_slice(&line)
                .map_err(|e| self.protocol(format!("JSON illisible : {e}")))?;
            match (message.id, message.method) {
                // Request from the server: only `ping` is known, the rest is refused.
                (Some(their_id), Some(their_method)) => {
                    let reply = if their_method == "ping" {
                        json!({ "jsonrpc": "2.0", "id": their_id, "result": {} })
                    } else {
                        let error = json!({ "code": -32601, "message": "unknown method" });
                        json!({ "jsonrpc": "2.0", "id": their_id, "error": error })
                    };
                    self.send(&reply).await?;
                }
                (Some(their_id), None) if their_id == json!(id) => {
                    return match (message.result, message.error) {
                        (_, Some(e)) => Err(McpError::Rpc {
                            server: self.name.clone(),
                            method: method.to_owned(),
                            code: e.code,
                            message: e.message,
                        }),
                        (Some(result), None) => Ok(result),
                        (None, None) => Err(self.protocol("réponse sans résultat".into())),
                    };
                }
                // Notification, or late response to an abandoned request: ignored.
                _ => tracing::debug!(server = %self.name, "message ignored: {}", preview(&line)),
            }
        }
    }

    /// Reads a line of at most [`MAX_MESSAGE_BYTES`] bytes.
    async fn read_message(&mut self) -> Result<Vec<u8>, McpError> {
        let connection = self.connection.as_mut();
        let connection = connection.ok_or_else(|| McpError::Lost(self.name.clone()))?;
        let mut line = Vec::new();
        let limit = MAX_MESSAGE_BYTES as u64;
        let mut reader = (&mut connection.stdout).take(limit);
        let read = reader.read_until(b'\n', &mut line).await;
        match read {
            Ok(_) if line.ends_with(b"\n") => Ok(line),
            Ok(_) if line.len() as u64 >= limit => {
                Err(self.protocol(format!("message de plus de {MAX_MESSAGE_BYTES} octets")))
            }
            // End of stream, possibly in the middle of a line.
            Ok(_) | Err(_) => Err(McpError::Closed(self.name.clone())),
        }
    }

    async fn send(&mut self, message: &Value) -> Result<(), McpError> {
        let connection = self.connection.as_mut();
        let connection = connection.ok_or_else(|| McpError::Lost(self.name.clone()))?;
        let mut line = message.to_string();
        line.push('\n');
        let written = async {
            connection.stdin.write_all(line.as_bytes()).await?;
            connection.stdin.flush().await
        };
        written
            .await
            .map_err(|_| McpError::Closed(self.name.clone()))
    }

    /// Stops the server: input closed for a graceful stop, `SIGKILL` after [`SHUTDOWN_GRACE`],
    /// then waits for the process so as not to leave a zombie.
    pub async fn shutdown(&mut self) {
        let Some(Connection {
            mut child,
            stdin,
            stdout,
        }) = self.connection.take()
        else {
            return;
        };
        drop(stdin);
        drop(stdout);
        if let Ok(Ok(_)) = tokio::time::timeout(SHUTDOWN_GRACE, child.wait()).await {
            return;
        }
        if let Err(error) = child.start_kill() {
            tracing::warn!(server = %self.name, %error, "SIGKILL failed");
        }
        if let Err(error) = child.wait().await {
            tracing::warn!(server = %self.name, %error, "waiting for the server failed");
        }
    }

    fn protocol(&self, detail: String) -> McpError {
        McpError::Protocol {
            server: self.name.clone(),
            detail,
        }
    }
}

/// Start of a server message, control characters escaped, for a log.
fn preview(line: &[u8]) -> String {
    let end = line.len().min(LOG_PREVIEW_BYTES);
    let text = String::from_utf8_lossy(&line[..end]);
    let mut preview: String = text.escape_debug().collect();
    if end < line.len() {
        preview.push('…');
    }
    preview
}

/// What executes tools for the agent: the MCP servers, or a fake in the tests.
///
/// An implementation itself refuses any tool that is not read-level: the agent checks it too,
/// but must not be the only safeguard.
pub trait ToolHost {
    /// Available tools, with their level.
    fn tools(&self) -> &[ToolSpec];

    /// Executes a read-level tool, refuses the others.
    fn call(
        &mut self,
        tool: &str,
        arguments: Value,
    ) -> impl Future<Output = Result<ToolOutput, McpError>> + Send;
}

/// The session's MCP servers, and the tools each one exposes.
#[derive(Default)]
pub struct McpServers {
    servers: Vec<McpClient>,
    tools: Vec<ToolSpec>,
    owners: Vec<usize>,
}

impl McpServers {
    /// No server.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an initialized server and discovers its tools. On error, the server is stopped.
    pub async fn add(&mut self, mut client: McpClient) -> Result<(), McpError> {
        let tools = match client.list_tools().await {
            Ok(tools) => tools,
            Err(error) => {
                client.shutdown().await;
                return Err(error);
            }
        };
        if let Some(tool) = tools
            .iter()
            .find(|t| self.tools.iter().any(|o| o.name == t.name))
        {
            client.shutdown().await;
            return Err(McpError::DuplicateTool(tool.name.clone()));
        }
        self.owners
            .extend(std::iter::repeat_n(self.servers.len(), tools.len()));
        self.tools.extend(tools);
        self.servers.push(client);
        Ok(())
    }

    /// Stops every server, each like [`McpClient::shutdown`].
    pub async fn shutdown(&mut self) {
        for server in &mut self.servers {
            server.shutdown().await;
        }
    }
}

impl ToolHost for McpServers {
    fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Result<ToolOutput, McpError> {
        let index = self.tools.iter().position(|t| t.name == tool);
        let index = index.ok_or_else(|| McpError::UnknownTool(tool.to_owned()))?;
        let level = self.tools[index].level;
        if level != Level::Read {
            // Until the broker exists (A-11), nothing but read calls goes to the server.
            return Err(McpError::BrokerAbsent {
                tool: tool.to_owned(),
                level,
            });
        }
        let server = self.servers.get_mut(self.owners[index]);
        let server = server.ok_or_else(|| McpError::UnknownTool(tool.to_owned()))?;
        tracing::debug!(server = server.name(), tool, "tools/call");
        server.call_tool(tool, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_read_from_annotations() {
        let level = |v: Value| Level::from_annotations(Some(&v));
        assert_eq!(level(json!({ "readOnlyHint": true })), Level::Read);
        assert_eq!(
            level(json!({ "readOnlyHint": true, "destructiveHint": false })),
            Level::Read
        );
        assert_eq!(
            level(json!({ "readOnlyHint": false, "destructiveHint": false })),
            Level::Action
        );
        assert_eq!(level(json!({ "destructiveHint": false })), Level::Action);
        assert_eq!(level(json!({ "readOnlyHint": false })), Level::Destructive);
        assert_eq!(
            level(json!({ "destructiveHint": true })),
            Level::Destructive
        );
    }

    #[test]
    fn destructive_when_in_doubt() {
        let level = |v: Value| Level::from_annotations(Some(&v));
        assert_eq!(Level::from_annotations(None), Level::Destructive);
        assert_eq!(level(json!(5)), Level::Destructive);
        // Contradiction.
        assert_eq!(
            level(json!({ "readOnlyHint": true, "destructiveHint": true })),
            Level::Destructive
        );
        // Values that are not booleans.
        assert_eq!(level(json!({ "readOnlyHint": "oui" })), Level::Destructive);
        assert_eq!(level(json!({ "readOnlyHint": null })), Level::Destructive);
        assert_eq!(
            level(json!({ "readOnlyHint": 1, "destructiveHint": false })),
            Level::Destructive
        );
        assert_eq!(
            level(json!({ "readOnlyHint": true, "destructiveHint": "non" })),
            Level::Destructive
        );
    }

    #[test]
    fn environment_without_keys() {
        let vars = [
            "PATH",
            "HOME",
            "LC_ALL",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_BASE_URL",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "GITHUB_TOKEN",
            "HTTPS_PROXY",
        ]
        .map(|n| (OsString::from(n), OsString::from("x")));
        let kept = |extra| -> Vec<OsString> {
            clean_env(vars.clone(), extra)
                .into_iter()
                .map(|(n, _)| n)
                .collect()
        };
        assert_eq!(kept(&[]), ["PATH", "HOME", "LC_ALL"]);
        assert_eq!(
            kept(&["HTTPS_PROXY"]),
            ["PATH", "HOME", "LC_ALL", "HTTPS_PROXY"]
        );
    }

    #[test]
    fn preview_truncated_and_escaped() {
        assert_eq!(preview(b"a\x1b\n"), "a\\u{1b}\\n");
        let long = vec![b'x'; 1000];
        assert_eq!(preview(&long).chars().count(), LOG_PREVIEW_BYTES + 1);
    }
}
