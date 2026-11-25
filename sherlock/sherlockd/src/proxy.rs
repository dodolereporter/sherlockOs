//! The proxy: `sherlockd` itself serves an MCP server, placed in front of the real ones
//! (ADR 0016).
//!
//! Claude Code sees only this server. It gets from it the tool list of the configured servers,
//! and each of its calls goes through [`agent::execute`], then [`McpServers::call`]: same levels,
//! same refusals, same traces as the agent loop. Everything the client sends is external data,
//! read within the same bounds as a server's responses.

use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::agent::{self, CallError};
use crate::mcp::{MAX_MESSAGE_BYTES, PROTOCOL_VERSION, ToolHost};

/// Protocol versions accepted from the client. The proxy only uses `tools/list` and
/// `tools/call`, identical in these versions. Any other request gets [`PROTOCOL_VERSION`].
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-11-25", PROTOCOL_VERSION, "2025-03-26"];

/// Maximum number of tool calls relayed for one conversation: the cap of the agent loop, 8 turns
/// of 8 calls. Beyond it, each call gets an error result.
pub const MAX_CALLS: usize = 64;

/// Serves the client until the end of its input, or until a message is too long.
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
                tracing::warn!("client message over {MAX_MESSAGE_BYTES} bytes, stopping");
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

/// Reply to a client message, `None` for a notification or a response.
async fn answer<H: ToolHost>(line: &[u8], host: &mut H, calls: &mut usize) -> Option<Value> {
    let Ok(message) = serde_json::from_slice::<Value>(line) else {
        let error = json!({ "code": -32700, "message": "unreadable JSON" });
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
            let error = json!({ "code": -32601, "message": "unknown method" });
            return Some(json!({ "jsonrpc": "2.0", "id": id, "error": error }));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// Relays a call through [`agent::execute`]. A refusal or a failure is returned as an error
/// result: the model can explain it, and nothing goes to the server.
async fn call<H: ToolHost>(host: &mut H, params: &Value, calls: &mut usize) -> Value {
    let tool = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    *calls += 1;
    let result = if *calls > MAX_CALLS {
        tracing::warn!(tool, "call refused: call cap reached");
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

    /// Host that counts the calls it executes.
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
    async fn refuses_non_read_without_executing() {
        let mut host = host();
        let replies = exchange(&mut host, &[call(1, "effacer"), call(2, "inconnu")]).await;
        for reply in &replies {
            assert_eq!(reply["result"]["isError"], true);
        }
        let refusal = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(refusal.contains("courtier d'accords absent"), "{refusal}");
        assert_eq!(host.1, 0);
    }

    #[tokio::test]
    async fn call_cap_per_conversation() {
        let mut host = host();
        let requests: Vec<Value> = (0..MAX_CALLS as u64 + 2).map(|i| call(i, "lire")).collect();
        let replies = exchange(&mut host, &requests).await;
        assert_eq!(host.1, MAX_CALLS);
        assert_eq!(replies[MAX_CALLS]["result"]["isError"], true);
    }

    #[tokio::test]
    async fn lists_with_level_and_ignores_notifications() {
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
    async fn giant_message_stops_service() {
        let mut host = host();
        let input = vec![b'a'; MAX_MESSAGE_BYTES + 10];
        let mut output = Vec::new();
        serve(input.as_slice(), &mut output, &mut host)
            .await
            .unwrap();
        assert!(output.is_empty());
    }
}
