//! What the screen remembers between two logins: the last user who logged in, shown in the
//! middle and selected, and the session each user chose. One small file,
//! `/var/cache/sherlockos-greeter/state`, owned by greetd's user (`just install-greeter` creates
//! the folder), or in `SHERLOCKOS_GREETER_STATE`:
//!
//! ```text
//! last-user=alex
//! session.alex=sherlockos
//! ```
//!
//! Read with a bound, written whole to a new file then renamed, so that a cut never leaves half a
//! file. A name with a line break, `=` or nothing in it is never written. Without the folder, the
//! screen still works: it remembers nothing.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::users::read_bounded;

/// The folder of the file, and the variable that replaces it.
pub const FOLDER: &str = "/var/cache/sherlockos-greeter";
pub const FOLDER_VAR: &str = "SHERLOCKOS_GREETER_STATE";
const FILE: &str = "state";
/// The most users remembered: the oldest go first.
pub const MAX_USERS: usize = 32;

/// What the screen remembers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Memory {
    pub last_user: Option<String>,
    /// The session of each user, the most recent last.
    sessions: Vec<(String, String)>,
}

/// A user or session name that can go in the file.
fn writable(name: &str) -> bool {
    !name.is_empty() && name.len() <= 256 && !name.contains(['\n', '\r', '='])
}

impl Memory {
    /// Reads the text of the file; lines it does not understand are skipped.
    pub fn parse(text: &str) -> Self {
        let mut memory = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !writable(value) {
                continue;
            }
            if key == "last-user" {
                memory.last_user = Some(value.to_owned());
            } else if let Some(user) = key.strip_prefix("session.").filter(|u| writable(u)) {
                memory.set_session(user, value);
            }
        }
        memory
    }

    pub fn text(&self) -> String {
        let mut text = String::new();
        if let Some(user) = &self.last_user {
            text.push_str(&format!("last-user={user}\n"));
        }
        for (user, session) in &self.sessions {
            text.push_str(&format!("session.{user}={session}\n"));
        }
        text
    }

    /// The session `user` chose last time.
    pub fn session(&self, user: &str) -> Option<&str> {
        self.sessions
            .iter()
            .find(|(u, _)| u == user)
            .map(|(_, s)| s.as_str())
    }

    fn set_session(&mut self, user: &str, session: &str) {
        self.sessions.retain(|(u, _)| u != user);
        self.sessions.push((user.to_owned(), session.to_owned()));
        if self.sessions.len() > MAX_USERS {
            self.sessions.remove(0);
        }
    }

    /// `user` logged in with `session`.
    pub fn logged_in(&mut self, user: &str, session: &str) {
        if writable(user) {
            self.last_user = Some(user.to_owned());
            if writable(session) {
                self.set_session(user, session);
            }
        }
    }
}

/// The file of the memory.
pub fn path() -> PathBuf {
    let folder = std::env::var_os(FOLDER_VAR).map_or_else(|| PathBuf::from(FOLDER), PathBuf::from);
    folder.join(FILE)
}

/// Reads the memory at `path`; empty without it.
pub fn load(path: &Path) -> Memory {
    read_bounded(path)
        .map(|text| Memory::parse(&text))
        .unwrap_or_default()
}

/// Writes `memory` at `path`, through a new file renamed over the old one.
pub fn save(path: &Path, memory: &Memory) -> std::io::Result<()> {
    let partial = path.with_extension("new");
    let mut file = std::fs::File::create(&partial)?;
    file.write_all(memory.text().as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_user_and_each_session_are_remembered() {
        let mut memory = Memory::default();
        memory.logged_in("alex", "sherlockos");
        memory.logged_in("invite", "plasma");
        assert_eq!(memory.last_user.as_deref(), Some("invite"));
        assert_eq!(memory.session("alex"), Some("sherlockos"));
        assert_eq!(memory.session("invite"), Some("plasma"));
        memory.logged_in("alex", "plasma");
        let read = Memory::parse(&memory.text());
        assert_eq!(read, memory);
        assert_eq!(read.last_user.as_deref(), Some("alex"));
        assert_eq!(read.session("alex"), Some("plasma"));
    }

    #[test]
    fn names_that_would_break_the_file_are_not_written() {
        let mut memory = Memory::default();
        memory.logged_in("alex\nlast-user=root", "sherlockos");
        memory.logged_in("a=b", "sherlockos");
        memory.logged_in("", "sherlockos");
        assert_eq!(memory, Memory::default());
        memory.logged_in("alex", "x\ny");
        assert_eq!(memory.session("alex"), None);
        let read = Memory::parse("junk\nlast-user=\nsession.=x\nsession.alex=sherlockos\n");
        assert_eq!(read.last_user, None);
        assert_eq!(read.session("alex"), Some("sherlockos"));
    }

    #[test]
    fn the_oldest_users_are_forgotten() {
        let mut memory = Memory::default();
        for i in 0..MAX_USERS + 3 {
            memory.logged_in(&format!("user{i}"), "sherlockos");
        }
        assert_eq!(memory.session("user0"), None);
        assert!(memory.session(&format!("user{}", MAX_USERS + 2)).is_some());
    }

    #[test]
    fn the_file_is_replaced_whole() {
        let dir = std::env::temp_dir().join(format!("greeter-memory-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state");
        assert_eq!(load(&path), Memory::default());
        let mut memory = Memory::default();
        memory.logged_in("alex", "sherlockos");
        save(&path, &memory).unwrap();
        assert_eq!(load(&path), memory);
        assert!(!path.with_extension("new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
