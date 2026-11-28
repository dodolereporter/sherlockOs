//! The journal in the panel « Demande à Sherlock » (lot A-19d), as on the Sherlock board:
//! « Tout ce que Sherlock fait est noté ». The consent broker of sherlockd writes each decision
//! and each outcome to `$XDG_STATE_HOME/sherlock/journal.jsonl` (`docs/sherlockd.md`); the panel
//! reads its end, bounded, off the interface thread, and shows the newest first, all of them or
//! those of the conversation on screen.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Bytes read from the end of the journal, at most.
pub const MAX_READ: u64 = 256 * 1024;
/// Entries kept, the newest.
pub const MAX_ENTRIES: usize = 60;

/// A call, as the journal tells it: when it was decided, the tool, its level, the decision,
/// and its outcome once written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub time: i64,
    id: u64,
    pub tool: String,
    pub level: String,
    pub decision: String,
    pub outcome: Option<String>,
}

/// The journal of the user: `$XDG_STATE_HOME/sherlock/journal.jsonl`, or under `~/.local/state`.
pub fn path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".local/state")))?;
    Some(state.join("sherlock/journal.jsonl"))
}

/// The newest entries of the journal at `path`, newest first: its last `MAX_READ` bytes, the
/// line cut by the start of the reading left out. Blocking: off the interface thread.
pub fn read(path: &Path) -> Vec<Entry> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map_or(0, |m| m.len());
    let start = length.saturating_sub(MAX_READ);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(MAX_READ).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines();
    if start > 0 {
        lines.next();
    }
    let mut entries: Vec<Entry> = Vec::new();
    for line in lines {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let field = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
        let (Some(id), Some(tool)) = (value.get("id").and_then(Value::as_u64), field("tool"))
        else {
            continue;
        };
        if let Some(decision) = field("decision") {
            entries.push(Entry {
                time: value.get("time").and_then(Value::as_i64).unwrap_or(0),
                id,
                tool,
                level: field("level").unwrap_or_default(),
                decision,
                outcome: None,
            });
        } else if let Some(outcome) = field("outcome") {
            // The last decision of that call: the numbers start again with each session.
            if let Some(entry) = entries
                .iter_mut()
                .rev()
                .find(|e| e.id == id && e.tool == tool && e.outcome.is_none())
            {
                entry.outcome = Some(outcome);
            }
        }
    }
    entries.reverse();
    entries.truncate(MAX_ENTRIES);
    entries
}

/// The line of an entry, in French, as the board writes them.
pub fn line(entry: &Entry) -> String {
    use sherlock_ui::launcher::sherlock::text;
    let mut said = match entry.decision.as_str() {
        "granted" if entry.level == "lecture" => format!("{} {}", text::JOURNAL_READ, entry.tool),
        "granted" => format!("{}{}", entry.tool, text::JOURNAL_CONSENTED),
        "timed-out" => format!("{}{}", entry.tool, text::JOURNAL_TIMED_OUT),
        "unavailable" => format!("{}{}", entry.tool, text::JOURNAL_NOBODY),
        _ => format!("{}{}", entry.tool, text::JOURNAL_REFUSED),
    };
    if matches!(entry.outcome.as_deref(), Some("error" | "failed")) {
        said.push_str(text::JOURNAL_ERROR);
    }
    said
}

/// The time of an entry, « 21:46 », in the local time zone.
pub fn time(entry: &Entry) -> String {
    match appearance_config::local(entry.time) {
        Ok(tm) => format!("{:02}:{:02}", tm.tm_hour, tm.tm_min),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_the_journal_is_read_newest_first_with_outcomes() {
        let dir = std::env::temp_dir().join(format!("launcher-journal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("journal.jsonl");
        let lines = [
            r#"{"time":10,"id":1,"tool":"list","level":"lecture","arguments":"{}","decision":"granted"}"#,
            r#"{"time":11,"id":1,"tool":"list","outcome":"ok"}"#,
            "not json",
            r#"{"time":20,"id":2,"tool":"restart","level":"action","arguments":"{}","decision":"granted"}"#,
            r#"{"time":21,"id":2,"tool":"restart","outcome":"error","error":"x"}"#,
            r#"{"time":30,"id":1,"tool":"empty","level":"destructif","arguments":"{}","decision":"denied"}"#,
        ];
        std::fs::write(&path, lines.join("\n") + "\n").unwrap();
        let entries = read(&path);
        let tools: Vec<&str> = entries.iter().map(|e| e.tool.as_str()).collect();
        assert_eq!(tools, ["empty", "restart", "list"]);
        assert_eq!(entries[1].outcome.as_deref(), Some("error"));
        assert_eq!(entries[2].outcome.as_deref(), Some("ok"));
        assert!(line(&entries[2]).starts_with("A lu"));
        assert!(line(&entries[1]).contains("avec ton accord"));
        assert!(line(&entries[1]).contains("erreur"));
        assert!(line(&entries[0]).contains("refus"));
        // A journal larger than what is read: its end only, the cut line left out.
        let big = format!("{}\n{}\n", "x".repeat(MAX_READ as usize), lines[0]);
        std::fs::write(&path, big).unwrap();
        assert_eq!(read(&path).len(), 1);
        assert!(read(&dir.join("missing")).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
