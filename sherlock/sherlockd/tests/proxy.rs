//! The proxy as a real process, in front of the test server: what Claude Code would see.

use std::process::Stdio;

use serde_json::{Value, json};
use sherlockd::test_server::UNLEVELED_MARKER;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_sherlockd");

fn call(id: u64, name: &str, arguments: Value) -> Value {
    let params = json!({ "name": name, "arguments": arguments });
    json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params })
}

#[tokio::test]
async fn relays_read_and_refuses_the_rest() {
    let requests = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {} } }),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        call(3, "vider_corbeille", json!({})),
        call(4, "echo", json!({ "texte": "Bonjour" })),
    ];
    let mut child = Command::new(EXE)
        .arg("--proxy")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in &requests {
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().await.unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(UNLEVELED_MARKER));
    let replies: Vec<Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 4);

    let tools = &replies[1]["result"]["tools"];
    assert_eq!(tools[0]["name"], "echo");
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools[1]["name"], "vider_corbeille");
    assert_eq!(tools[1]["annotations"]["destructiveHint"], true);

    let refusal = &replies[2]["result"];
    assert_eq!(refusal["isError"], true);
    let text = refusal["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("courtier d'accords absent"), "{text}");

    assert_eq!(replies[3]["result"]["content"][0]["text"], "Bonjour");
    assert_eq!(replies[3]["result"]["isError"], false);
}
