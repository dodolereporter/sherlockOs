//! The conversation for the shell (lot A-19): `sherlockd --ask` reads a request on standard
//! input, never in its arguments, and writes what happens as JSON lines on standard output, one
//! event per line, as soon as it happens. The launcher's panel « Demande à Sherlock » reads them:
//! Sherlock's texts, the steps of his plan (each call of a tool) and their state, the final
//! answer, or why there is none.
//!
//! The state of a step follows the consent broker (A-11): the proxy, which Claude Code starts,
//! decides and writes each decision to the journal; `--ask` follows the journal from where it
//! was when the request started and gives each decision to the first step of that tool still
//! waiting for one. The panel only shows: the consent window stays the compositor's.
//!
//! Input: `{"request": "...", "history": [["request", "answer"], ...]}`, or plain text. The
//! history is the conversation the panel keeps: a new sherlockd per request, the recap made
//! from it (`ClaudeCode::remember`).

use std::collections::VecDeque;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::claude_code::ClaudeCodeError;
use crate::model::{Message, ModelReply};

/// Longest input read on standard input.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
/// Exchanges of the history taken, at most.
pub const MAX_HISTORY: usize = 20;
/// Bytes of the arguments of a step shown.
pub const MAX_SUMMARY: usize = 160;
/// Longest journal line read; a longer one is skipped.
const MAX_JOURNAL_LINE: usize = 8 * 1024;

/// What the panel is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub request: String,
    /// Earlier exchanges, oldest first.
    pub history: Vec<(String, String)>,
}

impl Input {
    /// Reads the input: the JSON object, or the whole text as the request. `None` when it is
    /// empty, too long, or not UTF-8.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_INPUT_BYTES {
            return None;
        }
        let text = std::str::from_utf8(bytes).ok()?.trim();
        let input = match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(object)) => {
                let request = object.get("request")?.as_str()?.trim().to_owned();
                let history = object
                    .get("history")
                    .and_then(Value::as_array)
                    .map(|pairs| {
                        pairs
                            .iter()
                            .filter_map(|pair| {
                                let pair = pair.as_array()?;
                                let (q, a) = (pair.first()?.as_str()?, pair.get(1)?.as_str()?);
                                Some((q.to_owned(), a.to_owned()))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let skip = history.len().saturating_sub(MAX_HISTORY);
                Self {
                    request,
                    history: history.into_iter().skip(skip).collect(),
                }
            }
            _ => Self {
                request: text.to_owned(),
                history: Vec::new(),
            },
        };
        (!input.request.is_empty()).then_some(input)
    }
}

/// The state of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Called: waiting for the broker (the user's consent, for an action).
    Waiting,
    Granted,
    Refused,
    Done,
    Failed,
}

impl State {
    pub fn key(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Granted => "granted",
            Self::Refused => "refused",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// Why there is no answer, for a clear message in the panel.
pub fn reason(error: &ClaudeCodeError) -> &'static str {
    match error {
        ClaudeCodeError::NotInstalled => "not-installed",
        ClaudeCodeError::NotLoggedIn => "not-logged-in",
        ClaudeCodeError::Timeout(_) => "offline",
        ClaudeCodeError::Failed(text) if offline(text) => "offline",
        _ => "failed",
    }
}

/// Whether Claude Code's failure says the network is missing.
fn offline(text: &str) -> bool {
    let text = text.to_lowercase();
    [
        "enotfound",
        "econnrefused",
        "eai_again",
        "network",
        "connection error",
        "unable to connect",
        "offline",
    ]
    .iter()
    .any(|word| text.contains(word))
}

/// A step of the plan, while it runs.
#[derive(Debug, Clone)]
struct Step {
    id: String,
    tool: String,
    state: State,
}

/// The events of a request: made from the conversation's messages and the journal's lines.
#[derive(Debug, Default)]
pub struct Events {
    steps: VecDeque<Step>,
}

impl Events {
    /// The events a message of the conversation gives.
    pub fn message(&mut self, message: &Message) -> Vec<Value> {
        match message {
            Message::User(_) => Vec::new(),
            Message::Assistant(ModelReply::Text(text)) => {
                vec![json!({ "event": "text", "text": text })]
            }
            Message::Assistant(ModelReply::ToolCalls(calls)) => calls
                .iter()
                .map(|call| {
                    self.steps.push_back(Step {
                        id: call.id.clone(),
                        tool: call.name.clone(),
                        state: State::Waiting,
                    });
                    json!({
                        "event": "step",
                        "id": call.id,
                        "tool": call.name,
                        "summary": cut(&call.arguments.to_string(), MAX_SUMMARY),
                        "state": State::Waiting.key(),
                    })
                })
                .collect(),
            Message::ToolResult {
                call_id, is_error, ..
            } => {
                let Some(step) = self.steps.iter_mut().find(|s| s.id == *call_id) else {
                    return Vec::new();
                };
                // A refusal stays a refusal; the error result that follows says nothing more.
                if step.state == State::Refused {
                    return Vec::new();
                }
                step.state = if *is_error {
                    State::Failed
                } else {
                    State::Done
                };
                vec![state(step, None)]
            }
        }
    }

    /// The events a line of the journal gives: a decision for the first step of its tool still
    /// waiting for one. Outcomes and lines of other sessions give none.
    pub fn journal(&mut self, line: &str) -> Vec<Value> {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let (Some(tool), Some(decision)) = (
            entry.get("tool").and_then(Value::as_str),
            entry.get("decision").and_then(Value::as_str),
        ) else {
            return Vec::new();
        };
        let Some(step) = self
            .steps
            .iter_mut()
            .find(|s| s.tool == tool && s.state == State::Waiting)
        else {
            return Vec::new();
        };
        step.state = match decision {
            "granted" => State::Granted,
            _ => State::Refused,
        };
        let level = entry.get("level").and_then(Value::as_str);
        vec![state(step, level)]
    }
}

fn state(step: &Step, level: Option<&str>) -> Value {
    let mut event = json!({ "event": "state", "id": step.id, "state": step.state.key() });
    if let Some(level) = level {
        event["level"] = json!(level);
    }
    event
}

/// `text` cut to `max` bytes on a character boundary, « … » after.
fn cut(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Follows the journal from its length when the request starts: each new complete line, once.
pub struct Follower {
    path: Option<PathBuf>,
    offset: u64,
    partial: Vec<u8>,
}

impl Follower {
    pub fn new(path: Option<PathBuf>) -> Self {
        let offset = path
            .as_deref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map_or(0, |m| m.len());
        Self {
            path,
            offset,
            partial: Vec::new(),
        }
    }

    /// The lines written since the last call.
    pub fn lines(&mut self) -> Vec<String> {
        use std::io::{Read, Seek, SeekFrom};
        let Some(path) = &self.path else {
            return Vec::new();
        };
        let Ok(mut file) = std::fs::File::open(path) else {
            return Vec::new();
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        let read = (&mut file)
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .unwrap_or(0);
        self.offset += read as u64;
        self.partial.extend_from_slice(&bytes);
        let mut lines = Vec::new();
        while let Some(end) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            if line.len() <= MAX_JOURNAL_LINE
                && let Ok(text) = String::from_utf8(line)
            {
                lines.push(text.trim_end().to_owned());
            }
        }
        if self.partial.len() > MAX_JOURNAL_LINE {
            self.partial.clear();
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ToolCall;

    #[test]
    fn the_input_is_json_or_plain_text_and_bounded() {
        let input = Input::parse(br#"{"request":" Hello ","history":[["a","b"],["c"]]}"#);
        assert_eq!(
            input,
            Some(Input {
                request: "Hello".into(),
                history: vec![("a".into(), "b".into())],
            })
        );
        assert_eq!(
            Input::parse(b"Range mes fichiers").unwrap().request,
            "Range mes fichiers"
        );
        assert_eq!(Input::parse(b"  "), None);
        assert_eq!(Input::parse(br#"{"request":""}"#), None);
        assert_eq!(Input::parse(&vec![b'a'; MAX_INPUT_BYTES + 1]), None);
    }

    #[test]
    fn a_step_follows_the_broker_then_its_result() {
        let mut events = Events::default();
        let call = |id: &str, name: &str| {
            Message::Assistant(ModelReply::ToolCalls(vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: json!({"name": "front-next"}),
            }]))
        };
        let shown = events.message(&call("1", "restart"));
        assert_eq!(shown[0]["event"], "step");
        assert_eq!(shown[0]["state"], "waiting");
        let _ = events.message(&call("2", "restart"));
        // The first decision for that tool goes to the first step waiting.
        let line = r#"{"id":7,"tool":"restart","level":"action","decision":"granted"}"#;
        let granted = events.journal(line);
        assert_eq!(granted[0]["id"], "1");
        assert_eq!(granted[0]["state"], "granted");
        assert_eq!(granted[0]["level"], "action");
        let denied = events.journal(r#"{"tool":"restart","decision":"denied"}"#);
        assert_eq!(denied[0]["id"], "2");
        assert_eq!(denied[0]["state"], "refused");
        // An outcome, another tool, garbage: nothing.
        assert!(
            events
                .journal(r#"{"tool":"restart","outcome":"ok"}"#)
                .is_empty()
        );
        assert!(events.journal("not json").is_empty());
        let result = |id: &str, is_error| Message::ToolResult {
            call_id: id.into(),
            content: String::new(),
            is_error,
        };
        assert_eq!(events.message(&result("1", false))[0]["state"], "done");
        assert!(
            events.message(&result("2", true)).is_empty(),
            "refused stays"
        );
    }

    #[test]
    fn the_reasons_are_told_apart() {
        assert_eq!(reason(&ClaudeCodeError::NotInstalled), "not-installed");
        assert_eq!(reason(&ClaudeCodeError::NotLoggedIn), "not-logged-in");
        let failed = ClaudeCodeError::Failed("API Error: Connection error. ENOTFOUND".into());
        assert_eq!(reason(&failed), "offline");
        assert_eq!(reason(&ClaudeCodeError::Failed("other".into())), "failed");
    }

    #[test]
    fn the_journal_is_followed_from_where_it_was() {
        let dir = std::env::temp_dir().join(format!("sherlockd-follow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        std::fs::write(&path, "{\"old\":1}\n").unwrap();
        let mut follower = Follower::new(Some(path.clone()));
        assert!(
            follower.lines().is_empty(),
            "what was there before is skipped"
        );
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"a\":1}\n{\"b\"").unwrap();
        assert_eq!(follower.lines(), ["{\"a\":1}"]);
        file.write_all(b":2}\n").unwrap();
        assert_eq!(follower.lines(), ["{\"b\":2}"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
