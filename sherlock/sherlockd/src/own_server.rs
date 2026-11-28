//! What Sherlock's own MCP servers share (the windows server, A-12, the files server, A-13): the
//! loop that serves standard input, the shape of a result, the annotations of a level, and the
//! cleaning of a text that comes from outside (a title, a file name, a file's content).

use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The levels a tool declares, as its annotations (ADR 0015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Declared {
    Read,
    Action,
    Destructive,
}

impl Declared {
    pub fn annotations(self) -> Value {
        match self {
            Self::Read => json!({ "readOnlyHint": true, "openWorldHint": false }),
            Self::Action => {
                json!({ "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false })
            }
            Self::Destructive => {
                json!({ "readOnlyHint": false, "destructiveHint": true, "openWorldHint": false })
            }
        }
    }
}

/// Serves the requests of standard input until it closes: `tools` for `tools/list`, `call` for
/// each `tools/call`, with the call's name and arguments.
pub fn serve(
    name: &str,
    tools: &[Value],
    mut call: impl FnMut(&str, &Value) -> Value,
) -> io::Result<()> {
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let Ok(request) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let (Some(id), Some(method)) = (request.get("id"), request["method"].as_str()) else {
            continue; // notification
        };
        let params = &request["params"];
        let reply = match method {
            "initialize" => Ok(json!({
                "protocolVersion": params["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": name, "version": env!("CARGO_PKG_VERSION") },
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools })),
            "tools/call" => Ok(call(
                params["name"].as_str().unwrap_or(""),
                &params["arguments"],
            )),
            _ => Err(json!({ "code": -32601, "message": "unknown method" })),
        };
        let reply = match reply {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

/// A tool's result: one text block.
pub fn result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// Whether `c` breaks a line or reorders the text around it without being seen: the control
/// characters, the line and paragraph separators, the zero-width and bidirectional marks.
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{200b}'..='\u{200f}' | '\u{feff}'
        )
        || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// `text` without hidden characters, cut to `max` characters, « … » after.
pub fn plain(text: &str, max: usize) -> String {
    cut(
        &text.chars().filter(|c| !hidden(*c)).collect::<String>(),
        max,
    )
}

/// `text` cut to `max` characters, « … » after.
pub fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// `value` as one line of JSON, every hidden character escaped (`\u202e`), even those JSON
/// leaves as they are: a value that must stay exact (a file name) cannot break a line nor
/// reorder what follows it.
pub fn json_line(value: &Value) -> String {
    let mut text = String::new();
    for c in value.to_string().chars() {
        if hidden(c) {
            // Outside strings, JSON has no hidden character: these are all inside one.
            text.push_str(&format!("\\u{:04x}", u32::from(c)));
        } else {
            text.push(c);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_json_line_escapes_what_json_leaves_and_stays_json() {
        let name = "facture\u{202e}fdp.exe\u{2028}x";
        let line = json_line(&json!({ "nom": name }));
        assert!(!line.chars().any(hidden), "{line}");
        let back: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(back["nom"], name);
        assert_eq!(plain("a\n\u{202e}b", 10), "ab");
        assert_eq!(cut("abcdef", 3), "abc…");
    }
}
