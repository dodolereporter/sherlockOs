//! Serveur MCP de test, lancé par `sherlockd --serveur-essai` pour `--essai` et les tests.
//!
//! Il n'agit pas sur le système. Deux outils : `echo`, déclaré en lecture, qui renvoie son texte,
//! et `vider_corbeille`, sans niveau, qui ne vide rien et renvoie [`MARQUEUR_SANS_NIVEAU`] : si ce
//! marqueur apparaît dans une conversation, l'outil sans niveau a été exécuté.

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// Texte rendu par `vider_corbeille` s'il est un jour exécuté.
pub const MARQUEUR_SANS_NIVEAU: &str = "OUTIL-SANS-NIVEAU-EXECUTE";

/// Sert les requêtes de l'entrée standard jusqu'à sa fermeture. Avec `crash_on_call`, le
/// processus s'arrête au premier `tools/call`, pour simuler un serveur qui plante.
pub fn run(crash_on_call: bool) -> io::Result<()> {
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let request: Value = match serde_json::from_str(&line?) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let (Some(id), Some(method)) = (request.get("id"), request["method"].as_str()) else {
            continue; // notification
        };
        if crash_on_call && method == "tools/call" {
            std::process::exit(3);
        }
        let result = match method {
            "initialize" => json!({
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "sherlock-essai", "version": env!("CARGO_PKG_VERSION") },
            }),
            "ping" => json!({}),
            "tools/list" => tools(),
            "tools/call" => call(&request["params"]),
            _ => {
                let error = json!({ "code": -32601, "message": "méthode inconnue" });
                writeln!(
                    out,
                    "{}",
                    json!({ "jsonrpc": "2.0", "id": id, "error": error })
                )?;
                out.flush()?;
                continue;
            }
        };
        writeln!(
            out,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "result": result })
        )?;
        out.flush()?;
    }
    Ok(())
}

fn tools() -> Value {
    json!({ "tools": [
        {
            "name": "echo",
            "description": "Renvoie le texte reçu.",
            "inputSchema": {
                "type": "object",
                "properties": { "texte": { "type": "string" } },
                "required": ["texte"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "vider_corbeille",
            "description": "Vide la corbeille. Ne déclare pas son niveau.",
            "inputSchema": { "type": "object", "properties": {} },
        },
    ]})
}

fn call(params: &Value) -> Value {
    let text = |text: &str, is_error| json!({ "content": [{ "type": "text", "text": text }], "isError": is_error });
    match params["name"].as_str() {
        Some("echo") => match params["arguments"]["texte"].as_str() {
            Some(texte) => text(texte, false),
            None => text("argument texte manquant", true),
        },
        Some("vider_corbeille") => text(MARQUEUR_SANS_NIVEAU, false),
        _ => text("outil inconnu", true),
    }
}
