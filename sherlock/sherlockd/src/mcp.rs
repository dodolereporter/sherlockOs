//! Client MCP minimal : JSON-RPC 2.0 sur l'entrée et la sortie standard d'un serveur.
//!
//! Le strict nécessaire du protocole : `initialize`, `notifications/initialized`, `tools/list`
//! et `tools/call`, un message JSON par ligne. Le serveur est un processus enfant, tué quand le
//! client disparaît.

use std::fmt;
use std::future::Future;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Version du protocole MCP demandée au serveur.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Niveau d'un outil, qui décide de la règle d'accord (contexte, « Les trois niveaux »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Lit sans rien changer. Libre, journalisé.
    Read,
    /// Change l'état du système. Accord de l'utilisateur.
    Action,
    /// Efface ou détruit. Accord renforcé. C'est aussi le niveau d'un outil qui n'en déclare pas.
    Destructive,
}

impl Level {
    /// Lit le niveau dans les annotations standard de l'outil (proposition de l'ADR 0015).
    ///
    /// `readOnlyHint: true` donne la lecture ; sinon `destructiveHint: false` donne l'action ; tout
    /// le reste, absence d'annotations comprise, est destructif, comme le veulent les valeurs par
    /// défaut du protocole.
    pub fn from_annotations(annotations: Option<&Value>) -> Self {
        let hint = |name: &str| {
            annotations
                .and_then(|a| a.get(name))
                .and_then(Value::as_bool)
        };
        match (hint("readOnlyHint"), hint("destructiveHint")) {
            (Some(true), _) => Self::Read,
            (_, Some(false)) => Self::Action,
            _ => Self::Destructive,
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "lecture",
            Self::Action => "action",
            Self::Destructive => "destructif",
        })
    }
}

/// Un outil exposé par un serveur MCP.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    /// Nom unique de l'outil.
    pub name: String,
    /// Description destinée au modèle.
    pub description: String,
    /// Schéma JSON des arguments.
    pub input_schema: Value,
    /// Niveau lu dans les annotations.
    pub level: Level,
}

/// Résultat d'un appel d'outil.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    /// Les blocs de texte du résultat, mis bout à bout.
    pub text: String,
    /// Vrai si le serveur signale que l'outil a échoué.
    pub is_error: bool,
}

/// Erreur du client MCP.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// Le programme du serveur n'a pas pu être lancé.
    #[error("le serveur {server} ne se lance pas : {source}")]
    Spawn {
        server: String,
        source: std::io::Error,
    },
    /// Le serveur a fermé sa sortie : il s'est arrêté ou a planté.
    #[error("le serveur {0} s'est arrêté")]
    Closed(String),
    /// Le serveur n'a pas répondu à temps.
    #[error("le serveur {server} n'a pas répondu à {method} en {delay:?}")]
    Timeout {
        server: String,
        method: String,
        delay: Duration,
    },
    /// Le serveur a envoyé un message que le client ne comprend pas.
    #[error("réponse invalide du serveur {server} : {detail}")]
    Protocol {
        server: String,
        detail: String,
    },
    /// Le serveur a répondu par une erreur JSON-RPC.
    #[error("le serveur {server} refuse {method} : {message} (code {code})")]
    Rpc {
        server: String,
        method: String,
        code: i64,
        message: String,
    },
    /// Deux serveurs exposent un outil du même nom.
    #[error("l'outil {0} est exposé par deux serveurs")]
    DuplicateTool(String),
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

/// Connexion à un serveur MCP lancé en processus enfant.
pub struct McpClient {
    name: String,
    // Gardé pour que `kill_on_drop` tue le serveur avec le client.
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    timeout: Duration,
}

impl McpClient {
    /// Lance le serveur et mène l'initialisation. `timeout` borne chaque requête.
    pub async fn spawn(
        name: &str,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<Self, McpError> {
        let spawn_err = |source| McpError::Spawn {
            server: name.to_owned(),
            source,
        };
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(spawn_err)?;
        let missing = || std::io::Error::other("tube standard absent");
        let stdin = child.stdin.take().ok_or_else(missing).map_err(spawn_err)?;
        let stdout = child.stdout.take().ok_or_else(missing).map_err(spawn_err)?;
        let mut client = Self {
            name: name.to_owned(),
            _child: child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
            timeout,
        };
        client.initialize().await?;
        Ok(client)
    }

    /// Nom du serveur, tel que donné au lancement.
    pub fn name(&self) -> &str {
        &self.name
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

    /// Liste les outils du serveur, page après page.
    pub async fn list_tools(&mut self) -> Result<Vec<ToolSpec>, McpError> {
        let mut tools = Vec::new();
        let mut cursor: Option<Value> = None;
        loop {
            let params = cursor.map_or_else(|| json!({}), |c| json!({ "cursor": c }));
            let mut result = self.request("tools/list", params).await?;
            let raw: Vec<RawTool> = serde_json::from_value(result["tools"].take())
                .map_err(|e| self.protocol(format!("tools/list : {e}")))?;
            tools.extend(raw.into_iter().map(|t| ToolSpec {
                level: Level::from_annotations(t.annotations.as_ref()),
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            }));
            match result.get("nextCursor") {
                Some(next) if !next.is_null() => cursor = Some(next.clone()),
                _ => return Ok(tools),
            }
        }
    }

    /// Appelle un outil et rend ses blocs de texte.
    pub async fn call_tool(
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
                None => format!("[bloc {} ignoré]", block["type"]),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Ok(ToolOutput { text, is_error })
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id;
        self.next_id += 1;
        let delay = self.timeout;
        let exchange = async {
            self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
                .await?;
            self.wait_response(id, method).await
        };
        tokio::time::timeout(delay, exchange)
            .await
            .unwrap_or_else(|_| {
                Err(McpError::Timeout {
                    server: self.name.clone(),
                    method: method.to_owned(),
                    delay,
                })
            })
    }

    async fn wait_response(&mut self, id: u64, method: &str) -> Result<Value, McpError> {
        loop {
            let line = match self.stdout.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) | Err(_) => return Err(McpError::Closed(self.name.clone())),
            };
            let message: Incoming = serde_json::from_str(&line)
                .map_err(|e| self.protocol(format!("JSON illisible : {e}")))?;
            match (message.id, message.method) {
                // Requête du serveur : seul `ping` est connu, le reste est refusé.
                (Some(their_id), Some(their_method)) => {
                    let reply = if their_method == "ping" {
                        json!({ "jsonrpc": "2.0", "id": their_id, "result": {} })
                    } else {
                        let error = json!({ "code": -32601, "message": "méthode inconnue" });
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
                // Notification, ou réponse tardive à une requête abandonnée : ignorée.
                _ => tracing::debug!(server = %self.name, "message ignoré : {line}"),
            }
        }
    }

    async fn send(&mut self, message: &Value) -> Result<(), McpError> {
        let mut line = message.to_string();
        line.push('\n');
        let written = async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await
        };
        written
            .await
            .map_err(|_| McpError::Closed(self.name.clone()))
    }

    fn protocol(&self, detail: String) -> McpError {
        McpError::Protocol {
            server: self.name.clone(),
            detail,
        }
    }
}

/// Ce qui exécute les outils pour l'agent : les serveurs MCP, ou un faux dans les tests.
pub trait ToolHost {
    /// Outils disponibles, avec leur niveau.
    fn tools(&self) -> &[ToolSpec];

    /// Exécute un outil. L'agent a déjà vérifié son niveau.
    fn call(
        &mut self,
        tool: &str,
        arguments: Value,
    ) -> impl Future<Output = Result<ToolOutput, McpError>> + Send;
}

/// Les serveurs MCP de la session, et l'outil que chacun expose.
#[derive(Default)]
pub struct McpServers {
    servers: Vec<McpClient>,
    tools: Vec<ToolSpec>,
    owners: Vec<usize>,
}

impl McpServers {
    /// Aucun serveur.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute un serveur initialisé et découvre ses outils.
    pub async fn add(&mut self, mut client: McpClient) -> Result<(), McpError> {
        let tools = client.list_tools().await?;
        for tool in &tools {
            if self.tools.iter().any(|t| t.name == tool.name) {
                return Err(McpError::DuplicateTool(tool.name.clone()));
            }
        }
        self.owners
            .extend(std::iter::repeat_n(self.servers.len(), tools.len()));
        self.tools.extend(tools);
        self.servers.push(client);
        Ok(())
    }
}

impl ToolHost for McpServers {
    fn tools(&self) -> &[ToolSpec] {
        &self.tools
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Result<ToolOutput, McpError> {
        let index = self.tools.iter().position(|t| t.name == tool);
        let server = index.and_then(|i| self.servers.get_mut(self.owners[i]));
        let server = server.ok_or_else(|| McpError::Protocol {
            server: "?".into(),
            detail: format!("aucun serveur n'expose {tool}"),
        })?;
        tracing::debug!(server = server.name(), tool, "tools/call");
        server.call_tool(tool, arguments).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn niveau_lu_dans_les_annotations() {
        let level = |v: Value| Level::from_annotations(Some(&v));
        assert_eq!(level(json!({ "readOnlyHint": true })), Level::Read);
        assert_eq!(
            level(json!({ "readOnlyHint": false, "destructiveHint": false })),
            Level::Action
        );
        assert_eq!(level(json!({ "readOnlyHint": false })), Level::Destructive);
        assert_eq!(level(json!({ "destructiveHint": false })), Level::Action);
        assert_eq!(
            level(json!({ "destructiveHint": true })),
            Level::Destructive
        );
        assert_eq!(level(json!({ "readOnlyHint": "oui" })), Level::Destructive);
    }

    #[test]
    fn outil_sans_niveau_destructif() {
        assert_eq!(Level::from_annotations(None), Level::Destructive);
    }
}
