//! The consent broker (lot A-11, ADR 0011 and ADR 0031): before a tool that is not a read runs,
//! the user is asked, in a window drawn by the compositor; the answer and the call are written
//! to an append-only journal.
//!
//! - **Read** runs freely, and is journaled.
//! - **Action** waits for the user's consent; **destructive** for a reinforced one, which the
//!   window asks for (ADR 0031). A read that reaches outside the machine is an action (rule S2,
//!   `Level::from_annotations`).
//! - **Refused by default**: anything but a consent refuses the call. The user says no, the
//!   window is closed, nobody answers within `CONSENT_TIMEOUT`, the compositor is unreachable or
//!   goes away mid-request, the journal cannot be written: refused.
//! - A consent is for one call: the tool and the exact arguments the user saw. It is a
//!   [`Permit`], which only this module makes and which [`crate::mcp::McpServers`] checks before
//!   anything reaches a server.

use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::mcp::Level;

/// How long the user has to answer. Past it, the call is refused.
pub const CONSENT_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes of the arguments shown in the window and written to the journal.
pub const MAX_SUMMARY: usize = 2048;

/// What the user is asked about: one call.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Unique within the session: the answer and the journal lines name it.
    pub id: u64,
    pub tool: String,
    pub level: Level,
    /// The arguments as JSON, cut to `MAX_SUMMARY`.
    pub summary: String,
}

impl Request {
    /// The destructive level asks for a reinforced consent (ADR 0031).
    pub fn reinforced(&self) -> bool {
        self.level == Level::Destructive
    }
}

/// The user's answer, or why there is none. Only `Granted` lets the call run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Granted,
    /// The user refused, or closed the window.
    Denied,
    /// No answer within `CONSENT_TIMEOUT`.
    TimedOut,
    /// No compositor to ask, or it went away during the request.
    Unavailable,
}

impl Decision {
    fn key(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::TimedOut => "timed-out",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Who asks the user: the compositor's consent window (A-11b), fakes in the tests.
pub trait Consent: Send {
    /// Asks the user about `request`. Anything but `Granted` refuses the call.
    fn ask<'a>(
        &'a mut self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>>;
}

/// No one to ask: every call beyond read is refused. The broker of a session without the
/// compositor's consent window.
#[derive(Debug, Default)]
pub struct Nobody;

impl Consent for Nobody {
    fn ask<'a>(
        &'a mut self,
        _: &'a Request,
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async { Decision::Unavailable })
    }
}

/// The right to run one call: read, or consented. Made only here; neither `Clone` nor
/// constructible elsewhere, and consumed by the call it allows.
#[derive(Debug)]
pub struct Permit {
    id: u64,
    tool: String,
    arguments: Value,
    level: Level,
}

impl Permit {
    /// Whether it allows `tool`, of `level`, with exactly `arguments`.
    pub fn allows(&self, tool: &str, level: Level, arguments: &Value) -> bool {
        self.tool == tool && self.level == level && &self.arguments == arguments
    }

    /// The request it answers, for the journal line of the outcome.
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// Why a call is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub level: Level,
    pub decision: Decision,
}

/// The broker of a session: who to ask, the journal, the number of the next request.
pub struct Broker {
    consent: Box<dyn Consent>,
    journal: Journal,
    timeout: Duration,
    next: u64,
}

impl Broker {
    /// A broker that asks `consent` and writes to `journal`.
    pub fn new(consent: Box<dyn Consent>, journal: Journal) -> Self {
        Self {
            consent,
            journal,
            timeout: CONSENT_TIMEOUT,
            next: 1,
        }
    }

    /// Nobody to ask, no journal: read only. The default of a tool host.
    pub fn closed() -> Self {
        Self::new(Box::new(Nobody), Journal::none())
    }

    /// Another answer delay, for the tests.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Decides whether `tool` of `level` may run with `arguments`: at once for a read, after
    /// the user's consent otherwise. Every decision is journaled; an action whose decision
    /// cannot be journaled is refused.
    pub async fn authorize(
        &mut self,
        tool: &str,
        level: Level,
        arguments: &Value,
    ) -> Result<Permit, Refusal> {
        let request = Request {
            id: self.next,
            tool: tool.to_owned(),
            level,
            summary: summary(arguments),
        };
        self.next += 1;
        let decision = if level == Level::Read {
            Decision::Granted
        } else {
            match tokio::time::timeout(self.timeout, self.consent.ask(&request)).await {
                Ok(decision) => decision,
                Err(_) => Decision::TimedOut,
            }
        };
        let written = self.journal.decision(&request, decision);
        if let Err(error) = &written {
            tracing::error!(%error, tool, "consent journal not written");
        }
        let decision = match (decision, written) {
            // A read stays free when the journal fails; an action does not run unrecorded.
            (Decision::Granted, Err(_)) if level != Level::Read => Decision::Unavailable,
            (decision, _) => decision,
        };
        if decision != Decision::Granted {
            tracing::warn!(tool, %level, decision = decision.key(), "call refused");
            return Err(Refusal { level, decision });
        }
        Ok(Permit {
            id: request.id,
            tool: tool.to_owned(),
            arguments: arguments.clone(),
            level,
        })
    }

    /// Writes the outcome of the call of request `id`.
    pub fn outcome(&mut self, id: u64, tool: &str, outcome: &str) {
        if let Err(error) = self.journal.outcome(id, tool, outcome) {
            tracing::error!(%error, tool, "consent journal not written");
        }
    }
}

/// The arguments as one line of JSON, cut to `MAX_SUMMARY` on a character boundary.
fn summary(arguments: &Value) -> String {
    let mut text = arguments.to_string();
    if text.len() > MAX_SUMMARY {
        let mut end = MAX_SUMMARY - '…'.len_utf8();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

/// The journal of consents and calls: one JSON object per line, only ever appended to, readable
/// by the user (`$XDG_STATE_HOME/sherlock/journal.jsonl`, 0600).
pub struct Journal {
    file: Option<File>,
}

impl Journal {
    /// No journal: a read still runs; an action is refused (nothing to record it in).
    pub fn none() -> Self {
        Self { file: None }
    }

    /// Opens `path` to append to it: created 0600, never truncated, never through a symbolic
    /// link (a link planted there could make it write elsewhere).
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        Ok(Self { file: Some(file) })
    }

    /// Where the session's journal is: `$XDG_STATE_HOME/sherlock/journal.jsonl`, or under
    /// `~/.local/state`.
    pub fn default_path() -> Option<PathBuf> {
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                let home = std::env::var_os("HOME").map(PathBuf::from)?;
                Some(home.join(".local/state"))
            })?;
        Some(state.join("sherlock/journal.jsonl"))
    }

    fn decision(&mut self, request: &Request, decision: Decision) -> io::Result<()> {
        self.write(json!({
            "time": seconds(),
            "id": request.id,
            "tool": request.tool,
            "level": request.level.to_string(),
            "arguments": request.summary,
            "decision": decision.key(),
        }))
    }

    fn outcome(&mut self, id: u64, tool: &str, outcome: &str) -> io::Result<()> {
        self.write(json!({ "time": seconds(), "id": id, "tool": tool, "outcome": outcome }))
    }

    /// One line, written at once: `serde_json` escapes every line break of the values, so
    /// that what a tool or the model writes cannot forge a line.
    fn write(&mut self, line: Value) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("no journal"))?;
        let mut bytes = line.to_string().into_bytes();
        bytes.push(b'\n');
        file.write_all(&bytes)?;
        file.flush()
    }
}

fn seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers each request with the next decision; records what it was asked.
    struct Fake(Vec<Decision>, Vec<Request>);

    impl Consent for Fake {
        fn ask<'a>(
            &'a mut self,
            request: &'a Request,
        ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            self.1.push(request.clone());
            let decision = if self.0.is_empty() {
                Decision::Denied
            } else {
                self.0.remove(0)
            };
            Box::pin(async move { decision })
        }
    }

    /// Never answers.
    struct Silent;

    impl Consent for Silent {
        fn ask<'a>(
            &'a mut self,
            _: &'a Request,
        ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
            Box::pin(std::future::pending())
        }
    }

    fn journal() -> (PathBuf, Journal) {
        let dir = std::env::temp_dir().join(format!("sherlockd-broker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("journal.jsonl");
        let journal = Journal::open(&path).unwrap();
        (path, journal)
    }

    #[tokio::test]
    async fn only_a_consent_lets_an_action_run_and_every_decision_is_journaled() {
        let (path, journal) = journal();
        let decisions = vec![Decision::Granted, Decision::Denied, Decision::Unavailable];
        let mut broker = Broker::new(Box::new(Fake(decisions, Vec::new())), journal);
        let args = json!({ "path": "/home/x/a\nforged" });
        // Read: free, nobody asked.
        let read = broker.authorize("lire", Level::Read, &args).await.unwrap();
        assert!(read.allows("lire", Level::Read, &args));
        // Action granted: for exactly that call.
        let permit = broker
            .authorize("ranger", Level::Action, &args)
            .await
            .unwrap();
        assert!(permit.allows("ranger", Level::Action, &args));
        assert!(!permit.allows("ranger", Level::Action, &json!({ "path": "/" })));
        assert!(!permit.allows("vider", Level::Action, &args));
        broker.outcome(permit.id(), "ranger", "ok");
        // Refused, or nobody to ask: no permit.
        for decision in [Decision::Denied, Decision::Unavailable] {
            let refused = broker.authorize("vider", Level::Destructive, &args).await;
            assert_eq!(
                refused.unwrap_err(),
                Refusal {
                    level: Level::Destructive,
                    decision
                }
            );
        }
        // Four decisions and one outcome, one line each: the argument's line break escaped.
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[1]["decision"], "granted");
        assert_eq!(lines[2]["outcome"], "ok");
        assert_eq!(lines[3]["decision"], "denied");
        assert_eq!(lines[4]["decision"], "unavailable");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn no_answer_in_time_is_a_refusal_and_the_destructive_level_asks_more() {
        let mut broker =
            Broker::new(Box::new(Silent), Journal::none()).with_timeout(Duration::from_millis(20));
        let refused = broker
            .authorize("vider", Level::Destructive, &json!({}))
            .await;
        // The journal cannot be written either: refused all the same.
        assert!(refused.is_err());
        let request = Request {
            id: 1,
            tool: "vider".into(),
            level: Level::Destructive,
            summary: String::new(),
        };
        assert!(request.reinforced());
        assert!(
            !Request {
                level: Level::Action,
                ..request
            }
            .reinforced()
        );
    }

    #[tokio::test]
    async fn an_action_that_cannot_be_journaled_does_not_run() {
        let consent = Box::new(Fake(vec![Decision::Granted], Vec::new()));
        let mut broker = Broker::new(consent, Journal::none());
        let refused = broker.authorize("ranger", Level::Action, &json!({})).await;
        assert_eq!(refused.unwrap_err().decision, Decision::Unavailable);
        // A read still runs without a journal.
        assert!(
            broker
                .authorize("lire", Level::Read, &json!({}))
                .await
                .is_ok()
        );
    }

    #[test]
    fn the_journal_never_follows_a_planted_link() {
        let dir = std::env::temp_dir().join(format!("sherlockd-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("elsewhere");
        let link = dir.join("journal.jsonl");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Journal::open(&link).is_err());
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_summary_is_bounded() {
        let long = json!({ "text": "é".repeat(MAX_SUMMARY) });
        let cut = summary(&long);
        assert!(cut.len() <= MAX_SUMMARY && cut.ends_with('…'));
    }
}
