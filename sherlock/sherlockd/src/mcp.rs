//! Client MCP minimal : JSON-RPC 2.0 sur l'entrée et la sortie standard d'un serveur.
//!
//! Le strict nécessaire du protocole : `initialize`, `notifications/initialized`, `tools/list`
//! et `tools/call`, un message JSON par ligne. Le serveur est un processus enfant. Tout ce qu'il
//! envoie est une donnée externe : rien ne doit faire paniquer le client ni le faire grossir sans
//! limite, et un doute sur un niveau se résout vers le plus restrictif.

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

/// Version du protocole MCP demandée au serveur.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Taille maximale d'un message lu, fin de ligne comprise : 4 Mio. Au-delà, le serveur est perdu.
/// Borne la mémoire face à un serveur hostile, en laissant de la place à un résultat d'outil
/// volumineux mais légitime.
pub const MAX_MESSAGE_BYTES: usize = 4 * 1024 * 1024;

/// Nombre maximal de pages de `tools/list`. Au-delà, la liste est refusée.
pub const MAX_TOOL_PAGES: usize = 16;

/// Nombre maximal d'outils d'un serveur. Au-delà, la liste est refusée.
pub const MAX_TOOLS_PER_SERVER: usize = 256;

/// Délai laissé au serveur pour s'arrêter de lui-même, entrée fermée, avant `SIGKILL`.
pub const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Délai par défaut d'une requête au serveur.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Longueur maximale d'un message de serveur recopié dans un journal.
const LOG_PREVIEW_BYTES: usize = 200;

/// Variables d'environnement transmises aux processus lancés par `sherlockd` : chemins, identité
/// de session, langue (`LC_*` aussi), terminal, niveau des journaux. Aucune clé, aucun jeton,
/// aucune variable qui change la destination d'une requête : tout le reste est retiré.
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

/// Garde, de `vars`, les variables de [`ENV_ALLOWED`], les `LC_*` et celles de `extra`.
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

/// Niveau d'un outil, qui décide de la règle d'accord (contexte, « Les trois niveaux »).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Lit sans rien changer. Libre, journalisé.
    Read,
    /// Change l'état du système. Accord de l'utilisateur.
    Action,
    /// Efface ou détruit. Accord renforcé. C'est aussi le niveau de tout outil douteux.
    Destructive,
}

impl Level {
    /// Lit le niveau dans les annotations standard de l'outil (ADR 0015).
    ///
    /// `readOnlyHint: true` donne la lecture, `destructiveHint: false` l'action. Tout le reste est
    /// destructif : absence d'annotations, valeur qui n'est pas un booléen, ou contradiction
    /// (`readOnlyHint` et `destructiveHint` tous deux vrais).
    pub fn from_annotations(annotations: Option<&Value>) -> Self {
        // `None` : absente ; `Some(None)` : présente mais pas un booléen.
        let hint = |name| annotations.and_then(|a| a.get(name)).map(Value::as_bool);
        match (hint("readOnlyHint"), hint("destructiveHint")) {
            (Some(Some(true)), None | Some(Some(false))) => Self::Read,
            (None | Some(Some(false)), Some(Some(false))) => Self::Action,
            _ => Self::Destructive,
        }
    }
}

impl Level {
    /// Annotations standard qui redonnent ce niveau, pour exposer l'outil à un client MCP.
    pub fn annotations(self) -> Value {
        match self {
            Self::Read => json!({ "readOnlyHint": true }),
            Self::Action => json!({ "readOnlyHint": false, "destructiveHint": false }),
            Self::Destructive => json!({ "readOnlyHint": false, "destructiveHint": true }),
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

/// Confiance accordée aux annotations d'un serveur (ADR 0015).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Trust {
    /// Serveur tiers : ses annotations sont ignorées, tous ses outils sont destructifs.
    #[default]
    ThirdParty,
    /// Serveur livré avec l'OS : ses annotations sont du code du projet, lues telles quelles.
    Ours,
}

/// Comment lancer un serveur MCP.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Nom du serveur, dans les journaux et les erreurs.
    pub name: String,
    /// Programme à lancer.
    pub program: String,
    /// Ses arguments.
    pub args: Vec<String>,
    /// Délai de chaque requête.
    pub timeout: Duration,
    /// Tiers par défaut : seule une marque explicite fait croire ses annotations.
    pub trust: Trust,
}

impl ServerConfig {
    /// Un serveur tiers, avec le délai par défaut.
    pub fn new(name: &str, program: &str, args: &[&str]) -> Self {
        Self {
            name: name.to_owned(),
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            timeout: DEFAULT_TIMEOUT,
            trust: Trust::ThirdParty,
        }
    }

    /// Marque le serveur comme l'un des nôtres.
    pub fn ours(mut self) -> Self {
        self.trust = Trust::Ours;
        self
    }

    /// Change le délai des requêtes.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
    /// Niveau lu dans les annotations, ou destructif pour un serveur tiers.
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
    /// Le serveur a été arrêté après une erreur, il ne reçoit plus rien.
    #[error("le serveur {0} est perdu")]
    Lost(String),
    /// Le serveur n'a pas répondu à temps.
    #[error("le serveur {server} n'a pas répondu à {method} en {delay:?}")]
    Timeout {
        server: String,
        method: String,
        delay: Duration,
    },
    /// Le serveur a envoyé un message que le client refuse.
    #[error("réponse invalide du serveur {server} : {detail}")]
    Protocol { server: String, detail: String },
    /// Le serveur a répondu par une erreur JSON-RPC.
    #[error("le serveur {server} refuse {method} : {message} (code {code})")]
    Rpc {
        server: String,
        method: String,
        code: i64,
        message: String,
    },
    /// Un nom d'outil apparaît deux fois, dans un serveur ou entre deux serveurs.
    #[error("l'outil {0} est exposé deux fois")]
    DuplicateTool(String),
    /// Aucun serveur n'expose cet outil.
    #[error("outil inconnu : {0}")]
    UnknownTool(String),
    /// Le niveau de l'outil exige un accord, et le courtier (lot A-11) n'existe pas encore.
    #[error(
        "courtier d'accords absent : l'outil {tool} est de niveau {level}, \
         seuls les outils de lecture s'exécutent"
    )]
    BrokerAbsent { tool: String, level: Level },
}

impl McpError {
    /// Vrai si, après cette erreur, l'échange avec le serveur n'est plus fiable.
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

/// Connexion à un serveur MCP lancé en processus enfant.
pub struct McpClient {
    name: String,
    // `None` une fois le serveur perdu et arrêté.
    connection: Option<Connection>,
    next_id: u64,
    timeout: Duration,
    trust: Trust,
}

impl McpClient {
    /// Lance le serveur et mène l'initialisation.
    pub async fn spawn(config: &ServerConfig) -> Result<Self, McpError> {
        let spawn_err = |source| McpError::Spawn {
            server: config.name.clone(),
            source,
        };
        let mut child = Command::new(&config.program)
            .args(&config.args)
            // Un serveur ne reçoit ni clé ni jeton de la session.
            .env_clear()
            .envs(clean_env(std::env::vars_os(), &[]))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // Filet de sécurité si le client disparaît sans `shutdown`.
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

    /// Nom du serveur, tel que donné au lancement.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Identifiant du processus du serveur, `None` une fois le serveur arrêté et attendu.
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

    /// Liste les outils du serveur, page après page, dans la limite des plafonds.
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

    /// Appelle un outil et rend ses blocs de texte. Sans contrôle du niveau : seul
    /// [`McpServers::call`], qui le vérifie, s'en sert.
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

    /// Envoie une requête et attend sa réponse. Après un arrêt, un délai dépassé ou un message
    /// refusé, l'échange n'est plus fiable : le serveur est arrêté.
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
            tracing::warn!(server = %self.name, %error, "serveur perdu, arrêt");
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
                _ => tracing::debug!(server = %self.name, "message ignoré : {}", preview(&line)),
            }
        }
    }

    /// Lit une ligne d'au plus [`MAX_MESSAGE_BYTES`] octets.
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
            // Fin de flux, éventuellement au milieu d'une ligne.
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

    /// Arrête le serveur : entrée fermée pour un arrêt doux, `SIGKILL` après
    /// [`SHUTDOWN_GRACE`], puis attente du processus pour ne pas laisser de zombie.
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
            tracing::warn!(server = %self.name, %error, "SIGKILL impossible");
        }
        if let Err(error) = child.wait().await {
            tracing::warn!(server = %self.name, %error, "attente du serveur impossible");
        }
    }

    fn protocol(&self, detail: String) -> McpError {
        McpError::Protocol {
            server: self.name.clone(),
            detail,
        }
    }
}

/// Début d'un message de serveur, caractères de contrôle échappés, pour un journal.
fn preview(line: &[u8]) -> String {
    let end = line.len().min(LOG_PREVIEW_BYTES);
    let text = String::from_utf8_lossy(&line[..end]);
    let mut preview: String = text.escape_debug().collect();
    if end < line.len() {
        preview.push('…');
    }
    preview
}

/// Ce qui exécute les outils pour l'agent : les serveurs MCP, ou un faux dans les tests.
///
/// Une implémentation refuse elle-même tout outil qui n'est pas en lecture : l'agent le vérifie
/// aussi, mais ne doit pas être le seul rempart.
pub trait ToolHost {
    /// Outils disponibles, avec leur niveau.
    fn tools(&self) -> &[ToolSpec];

    /// Exécute un outil en lecture, refuse les autres.
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

    /// Ajoute un serveur initialisé et découvre ses outils. En cas d'erreur, le serveur est arrêté.
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

    /// Arrête tous les serveurs, chacun comme [`McpClient::shutdown`].
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
            // En attendant le courtier (A-11), rien d'autre que la lecture ne part au serveur.
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
    fn niveau_lu_dans_les_annotations() {
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
    fn dans_le_doute_destructif() {
        let level = |v: Value| Level::from_annotations(Some(&v));
        assert_eq!(Level::from_annotations(None), Level::Destructive);
        assert_eq!(level(json!(5)), Level::Destructive);
        // Contradiction.
        assert_eq!(
            level(json!({ "readOnlyHint": true, "destructiveHint": true })),
            Level::Destructive
        );
        // Valeurs qui ne sont pas des booléens.
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
    fn environnement_sans_cle() {
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
    fn apercu_tronque_et_echappe() {
        assert_eq!(preview(b"a\x1b\n"), "a\\u{1b}\\n");
        let long = vec![b'x'; 1000];
        assert_eq!(preview(&long).chars().count(), LOG_PREVIEW_BYTES + 1);
    }
}
