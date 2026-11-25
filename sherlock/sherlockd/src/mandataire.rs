//! Le mandataire : `sherlockd` sert lui-même un serveur MCP, placé devant les vrais (ADR 0016).
//!
//! Claude Code ne voit que ce serveur. Il en reçoit la liste des outils des serveurs configurés,
//! et chacun de ses appels passe par [`agent::execute`], puis par [`McpServers::call`] : mêmes
//! niveaux, mêmes refus, mêmes traces que la boucle d'agent. Tout ce que le client envoie est une
//! donnée externe, lue dans les mêmes bornes que les réponses d'un serveur.

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::agent::{self, CallError};
use crate::mcp::{MAX_MESSAGE_BYTES, PROTOCOL_VERSION, ToolHost};

/// Versions du protocole acceptées du client. Le mandataire n'utilise que `tools/list` et
/// `tools/call`, identiques dans ces versions. Une autre demande reçoit [`PROTOCOL_VERSION`].
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-11-25", PROTOCOL_VERSION, "2025-03-26"];

/// Nombre maximal d'appels d'outils relayés pour une conversation : le plafond de la boucle
/// d'agent, 8 tours de 8 appels. Au-delà, chaque appel reçoit un résultat en erreur.
pub const MAX_CALLS: usize = 64;

/// Sert le client jusqu'à la fin de son entrée, ou jusqu'à un message trop long.
pub async fn serve<R, W, H>(mut input: R, mut output: W, host: &mut H) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
    H: ToolHost,
{
    let mut calls = 0;
    loop {
        let mut line = Vec::new();
        let limit = MAX_MESSAGE_BYTES as u64;
        (&mut input)
            .take(limit)
            .read_until(b'\n', &mut line)
            .await?;
        if !line.ends_with(b"\n") {
            if line.len() as u64 >= limit {
                tracing::warn!("message du client de plus de {MAX_MESSAGE_BYTES} octets, arrêt");
            }
            return Ok(());
        }
        let Some(reply) = answer(&line, host, &mut calls).await else {
            continue;
        };
        let mut reply = reply.to_string();
        reply.push('\n');
        output.write_all(reply.as_bytes()).await?;
        output.flush().await?;
    }
}

/// Réponse à un message du client, `None` pour une notification ou une réponse.
async fn answer<H: ToolHost>(line: &[u8], host: &mut H, calls: &mut usize) -> Option<Value> {
    let Ok(message) = serde_json::from_slice::<Value>(line) else {
        let error = json!({ "code": -32700, "message": "JSON illisible" });
        return Some(json!({ "jsonrpc": "2.0", "id": null, "error": error }));
    };
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str)?;
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str);
            let version = asked.filter(|v| SUPPORTED_VERSIONS.contains(v));
            json!({
                "protocolVersion": version.unwrap_or(PROTOCOL_VERSION),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "sherlockd", "version": env!("CARGO_PKG_VERSION") },
            })
        }
        "ping" => json!({}),
        "tools/list" => {
            let tools = host.tools().iter().map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.input_schema,
                    "annotations": t.level.annotations(),
                })
            });
            json!({ "tools": tools.collect::<Vec<_>>() })
        }
        "tools/call" => call(host, &params, calls).await,
        _ => {
            let error = json!({ "code": -32601, "message": "méthode inconnue" });
            return Some(json!({ "jsonrpc": "2.0", "id": id, "error": error }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Relaie un appel par [`agent::execute`]. Un refus ou un échec est rendu comme résultat en
/// erreur : le modèle peut l'expliquer, et rien ne part au serveur.
async fn call<H: ToolHost>(host: &mut H, params: &Value, calls: &mut usize) -> Value {
    let tool = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    *calls += 1;
    let result = if *calls > MAX_CALLS {
        tracing::warn!(tool, "appel refusé : plafond d'appels atteint");
        Err(CallError::SessionCallLimit(MAX_CALLS))
    } else {
        agent::execute(host, tool, arguments).await
    };
    let (text, is_error) = match result {
        Ok(output) => (output.text, output.is_error),
        Err(error) => (error.to_string(), true),
    };
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{Level, McpError, ToolOutput, ToolSpec};

    /// Hôte qui compte les appels qu'il exécute.
    struct CountingHost(Vec<ToolSpec>, usize);

    impl ToolHost for CountingHost {
        fn tools(&self) -> &[ToolSpec] {
            &self.0
        }

        async fn call(&mut self, _: &str, _: Value) -> Result<ToolOutput, McpError> {
            self.1 += 1;
            Ok(ToolOutput {
                text: "lu".into(),
                is_error: false,
            })
        }
    }

    fn host() -> CountingHost {
        let spec = |name: &str, level| ToolSpec {
            name: name.into(),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
            level,
        };
        CountingHost(
            vec![
                spec("lire", Level::Read),
                spec("effacer", Level::Destructive),
            ],
            0,
        )
    }

    async fn exchange(host: &mut CountingHost, requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let mut output = Vec::new();
        serve(input.as_bytes(), &mut output, host).await.unwrap();
        let output = String::from_utf8(output).unwrap();
        output
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn call(id: u64, name: &str) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name } })
    }

    #[tokio::test]
    async fn refuse_hors_lecture_sans_rien_executer() {
        let mut host = host();
        let replies = exchange(&mut host, &[call(1, "effacer"), call(2, "inconnu")]).await;
        for reply in &replies {
            assert_eq!(reply["result"]["isError"], true);
        }
        let refus = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(refus.contains("courtier d'accords absent"), "{refus}");
        assert_eq!(host.1, 0);
    }

    #[tokio::test]
    async fn plafond_d_appels_par_conversation() {
        let mut host = host();
        let requests: Vec<Value> = (0..MAX_CALLS as u64 + 2).map(|i| call(i, "lire")).collect();
        let replies = exchange(&mut host, &requests).await;
        assert_eq!(host.1, MAX_CALLS);
        assert_eq!(replies[MAX_CALLS]["result"]["isError"], true);
    }

    #[tokio::test]
    async fn liste_avec_le_niveau_et_ignore_les_notifications() {
        let mut host = host();
        let requests = [
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": { "protocolVersion": "1999-01-01" } }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        ];
        let replies = exchange(&mut host, &requests).await;
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
        let tools = &replies[1]["result"]["tools"];
        assert_eq!(tools[0]["annotations"], json!({ "readOnlyHint": true }));
        assert_eq!(tools[1]["annotations"]["destructiveHint"], true);
    }

    #[tokio::test]
    async fn message_geant_arrete_le_service() {
        let mut host = host();
        let input = vec![b'a'; MAX_MESSAGE_BYTES + 10];
        let mut output = Vec::new();
        serve(input.as_slice(), &mut output, &mut host)
            .await
            .unwrap();
        assert!(output.is_empty());
    }
}
