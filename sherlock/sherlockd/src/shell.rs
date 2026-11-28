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
/// Steps of an announced plan taken, at most, and the length of their title and description.
pub const MAX_PLAN_STEPS: usize = 12;
const MAX_TITLE: usize = 120;
const MAX_DESCRIPTION: usize = 300;

/// What the model is asked, in the system instructions of `--ask` (lot A-19e): announce its
/// plan first, in a block the shell reads. In French, since the model reads it.
pub const PLAN_INSTRUCTIONS: &str = "Avant d'appeler le moindre outil, annonce ton plan une seule fois, dans un \
     bloc <plan>…</plan> qui contient seulement une liste JSON d'étapes, chacune \
     {\"title\": …, \"description\": …, \"tool\": …} (l'outil que l'étape appellera, sans \
     préfixe). Écris le bloc avant tout autre texte, puis agis. Sans outil à appeler, pas de \
     bloc.";

/// Longest journal line read; a longer one is skipped.
const MAX_JOURNAL_LINE: usize = 8 * 1024;

/// What the panel is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub request: String,
    /// Earlier exchanges, oldest first.
    pub history: Vec<(String, String)>,
    /// The conversation's session in the journal (A-20e), when the panel gives a valid one.
    pub session: Option<String>,
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
                let session = object.get("session").and_then(Value::as_str);
                Self {
                    request,
                    history: history.into_iter().skip(skip).collect(),
                    session: session
                        .filter(|s| crate::broker::valid_session(s))
                        .map(str::to_owned),
                }
            }
            _ => Self {
                request: text.to_owned(),
                history: Vec::new(),
                session: None,
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
    /// The tools of the plan announced, by step; `true` once a call took that step.
    plan: Vec<(String, bool)>,
    /// The session of this request's journal lines (A-20e): those of another are left out.
    session: Option<String>,
}

/// The plan in `text`, and the text without it: `None` without a readable `<plan>` block. Each
/// step is `{"title", "description", "tool"}`, bounded.
pub fn plan(text: &str) -> Option<(Vec<Value>, String)> {
    let start = text.find("<plan>")?;
    let end = start + text[start..].find("</plan>")?;
    let body = &text[start + "<plan>".len()..end];
    let steps: Vec<Value> = serde_json::from_str(body.trim()).ok()?;
    let steps: Vec<Value> = steps
        .iter()
        .take(MAX_PLAN_STEPS)
        .map(|step| {
            let field = |name: &str, max| {
                cut(
                    step.get(name).and_then(Value::as_str).unwrap_or_default(),
                    max,
                )
            };
            json!({
                "title": field("title", MAX_TITLE),
                "description": field("description", MAX_DESCRIPTION),
                "tool": field("tool", MAX_TITLE),
            })
        })
        .collect();
    let rest = format!("{}{}", &text[..start], &text[end + "</plan>".len()..]);
    Some((steps, rest.trim().to_owned()))
}

impl Events {
    /// The events a message of the conversation gives.
    pub fn message(&mut self, message: &Message) -> Vec<Value> {
        match message {
            Message::User(_) => Vec::new(),
            Message::Assistant(ModelReply::Text(text)) => {
                let mut events = Vec::new();
                let mut said = text.as_str();
                let rest;
                // The plan, announced first (lot A-19e): once per request.
                if self.plan.is_empty()
                    && let Some((steps, left)) = plan(text)
                {
                    self.plan = steps
                        .iter()
                        .map(|s| (s["tool"].as_str().unwrap_or_default().to_owned(), false))
                        .collect();
                    events.push(json!({ "event": "plan", "steps": steps }));
                    rest = left;
                    said = &rest;
                }
                if !said.is_empty() {
                    events.push(json!({ "event": "text", "text": said }));
                }
                events
            }
            Message::Assistant(ModelReply::ToolCalls(calls)) => calls
                .iter()
                .map(|call| {
                    self.steps.push_back(Step {
                        id: call.id.clone(),
                        tool: call.name.clone(),
                        state: State::Waiting,
                    });
                    // The step of the plan it takes: the first one of that tool not taken.
                    let planned = self
                        .plan
                        .iter_mut()
                        .position(|(tool, taken)| *tool == call.name && !*taken);
                    if let Some(index) = planned {
                        self.plan[index].1 = true;
                    }
                    json!({
                        "event": "step",
                        "id": call.id,
                        "tool": call.name,
                        "summary": cut(&call.arguments.to_string(), MAX_SUMMARY),
                        "state": State::Waiting.key(),
                        "plan": planned,
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

    /// Only the journal lines of `session` count, once it is known (A-20e).
    pub fn for_session(mut self, session: &str) -> Self {
        self.session = Some(session.to_owned());
        self
    }

    /// The events a line of the journal gives: a decision for the first step of its tool still
    /// waiting for one. Outcomes and lines of other sessions give none.
    pub fn journal(&mut self, line: &str) -> Vec<Value> {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        let session = entry.get("session").and_then(Value::as_str);
        if self.session.is_some() && session != self.session.as_deref() {
            return Vec::new();
        }
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
        let input = Input::parse(
            br#"{"request":" Hello ","history":[["a","b"],["c"]],"session":"c0ffee-1"}"#,
        );
        assert_eq!(
            input,
            Some(Input {
                request: "Hello".into(),
                history: vec![("a".into(), "b".into())],
                session: Some("c0ffee-1".into()),
            })
        );
        // A session that is not a plain name is left aside: the request keeps one of its own.
        let trapped = Input::parse(
            br#"{"request":"x","session":"a
b"}"#,
        )
        .unwrap();
        assert_eq!(trapped.session, None);
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
        // With its session known, a line of another conversation gives nothing (A-20e).
        let mut mine = Events::default().for_session("s-1");
        let _ = mine.message(&call("1", "restart"));
        let other = r#"{"id":1,"tool":"restart","decision":"granted","session":"s-2"}"#;
        assert!(mine.journal(other).is_empty());
        let ours = r#"{"id":1,"tool":"restart","decision":"granted","session":"s-1"}"#;
        assert_eq!(mine.journal(ours)[0]["state"], "granted");
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
    fn the_plan_announced_first_is_read_and_its_steps_taken_in_order() {
        let text = "Voici.\n<plan>[{\"title\":\"Lire\",\"description\":\"La liste.\",\"tool\":\"list\"},\
                    {\"title\":\"Relancer\",\"description\":\"\",\"tool\":\"restart\"}]</plan>";
        let mut events = Events::default();
        let shown = events.message(&Message::Assistant(ModelReply::Text(text.into())));
        assert_eq!(shown[0]["event"], "plan");
        assert_eq!(shown[0]["steps"][1]["title"], "Relancer");
        assert_eq!(shown[0]["steps"][0]["tool"], "list");
        assert_eq!(shown[1]["text"], "Voici.");
        let call = |id: &str, name: &str| {
            Message::Assistant(ModelReply::ToolCalls(vec![ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: json!({}),
            }]))
        };
        assert_eq!(events.message(&call("b", "restart"))[0]["plan"], 1);
        assert_eq!(events.message(&call("a", "list"))[0]["plan"], 0);
        assert!(
            events.message(&call("c", "list"))[0]["plan"].is_null(),
            "not planned"
        );
        // A block that is not a list of steps stays text.
        assert!(plan("<plan>pas du JSON</plan>").is_none());
        assert!(plan("sans bloc").is_none());
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
