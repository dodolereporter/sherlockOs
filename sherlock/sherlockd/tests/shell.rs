//! `sherlockd --ask` (lot A-19), the conversation for the shell, run for real against a fake
//! `claude` found in `PATH`: never the real Claude Code.

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use serde_json::Value;

    const ECHO: &str = "tests/transcripts/echo.jsonl";

    /// A private folder for one test: the fake `claude`, the runtime and the state folders.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sherlockd-shell-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for sub in ["bin", "runtime", "state/sherlock"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        dir
    }

    /// The fake `claude`: logged in; reads the request; writes the init and the tool call, then
    /// the broker's decision in the journal as the proxy would, then the rest of the transcript.
    fn fake_claude(dir: &Path) {
        let transcript = Path::new(env!("CARGO_MANIFEST_DIR")).join(ECHO);
        let journal = dir.join("state/sherlock/journal.jsonl");
        let script = format!(
            "#!/bin/sh\n[ \"$1\" = auth ] && exit 0\ncat > /dev/null\nhead -n 2 '{t}'\n\
             printf '%s\\n' '{{\"id\":1,\"tool\":\"echo\",\"level\":\"lecture\",\"decision\":\"granted\"}}' >> '{j}'\n\
             sleep 1\ntail -n +3 '{t}'\n",
            t = transcript.display(),
            j = journal.display(),
        );
        let path = dir.join("bin/claude");
        std::fs::write(&path, script).unwrap();
        let status = Command::new("chmod").arg("+x").arg(&path).status().unwrap();
        assert!(status.success());
    }

    /// Runs `sherlockd --ask` with `input` on standard input; its events.
    fn ask(dir: &Path, path: &str, input: &str) -> Vec<Value> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sherlockd"))
            .arg("--ask")
            .env_clear()
            .env("PATH", path)
            .env("HOME", dir)
            .env("XDG_RUNTIME_DIR", dir.join("runtime"))
            .env("XDG_STATE_HOME", dir.join("state"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn the_conversation_streams_its_steps_their_states_and_the_answer() {
        let dir = folder("echo");
        fake_claude(&dir);
        let path = format!("{}:/usr/bin:/bin", dir.join("bin").display());
        let input = r#"{"request":"Dis bonjour","history":[["Avant","Réponse"]]}"#;
        let events = ask(&dir, &path, input);
        let kinds: Vec<&str> = events.iter().filter_map(|e| e["event"].as_str()).collect();
        assert_eq!(
            kinds,
            ["started", "step", "state", "state", "text", "answer"],
            "{events:#?}"
        );
        assert_eq!(events[1]["tool"], "echo");
        assert_eq!(events[1]["state"], "waiting");
        // The broker's decision, from the journal, then the result.
        assert_eq!(events[2]["state"], "granted");
        assert_eq!(events[2]["level"], "lecture");
        assert_eq!(events[3]["state"], "done");
        assert!(events[5]["text"].as_str().unwrap().contains("Bonjour"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_claude_code_the_reason_is_given() {
        let dir = folder("missing");
        let events = ask(&dir, "/nonexistent", "Bonjour");
        let last = events.last().unwrap();
        assert_eq!(last["event"], "failed");
        assert_eq!(last["reason"], "not-installed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_request_is_never_taken_from_the_arguments() {
        let output = Command::new(env!("CARGO_BIN_EXE_sherlockd"))
            .args(["--ask", "secret request"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
}
