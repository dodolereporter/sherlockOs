//! A fake greetd: an IPC server on a socket of the test's own, which plays a scenario and
//! records every request. No PAM, no real greetd, no session of the machine is reached.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use greeter::greetd::{Greetd, Prompt, Reply};
    use greeter::login::{Launch, Note, Outcome, log_in};
    use serde_json::{Value, json};
    use system_services::secret::Secret;

    const GOOD: &str = "le-bon-mot-de-passe";

    /// A step of a scenario: what greetd answers to a request, from the request.
    type Script = fn(&Value) -> Value;

    struct Fake {
        dir: PathBuf,
        socket: PathBuf,
        server: Option<JoinHandle<Vec<Value>>>,
    }

    impl Fake {
        /// Listens on a socket in a folder of the test's own, and answers each request of one
        /// connection with `script`, until the greeter leaves. Returns the requests seen.
        fn start(script: Script) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("fake-greetd-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let socket = dir.join("greetd.sock");
            let listener = UnixListener::bind(&socket).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut seen = Vec::new();
                while let Some(request) = read(&mut stream) {
                    let reply = script(&request);
                    seen.push(request);
                    write(&mut stream, &reply);
                }
                seen
            });
            Self {
                dir,
                socket,
                server: Some(server),
            }
        }

        fn connect(&self) -> Greetd {
            Greetd::connect_to(&self.socket).unwrap()
        }

        /// The requests greetd received, once the greeter has left.
        fn requests(mut self) -> Vec<Value> {
            self.server.take().unwrap().join().unwrap()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn read(stream: &mut UnixStream) -> Option<Value> {
        let mut length = [0; 4];
        stream.read_exact(&mut length).ok()?;
        let mut body = vec![0; u32::from_ne_bytes(length) as usize];
        stream.read_exact(&mut body).ok()?;
        Some(serde_json::from_slice(&body).unwrap())
    }

    fn write(stream: &mut UnixStream, reply: &Value) {
        let body = serde_json::to_vec(reply).unwrap();
        stream
            .write_all(&(body.len() as u32).to_ne_bytes())
            .unwrap();
        stream.write_all(&body).unwrap();
    }

    fn kind(request: &Value) -> &str {
        request["type"].as_str().unwrap()
    }

    fn success() -> Value {
        json!({"type": "success"})
    }

    fn secret_prompt() -> Value {
        json!({"type": "auth_message", "auth_message_type": "secret", "auth_message": "Password: "})
    }

    fn auth_error() -> Value {
        json!({"type": "error", "error_type": "auth_error", "description": "pam_authenticate: AUTH_ERR"})
    }

    /// greetd with pam_unix: one secret prompt, then the password is checked.
    fn pam(request: &Value) -> Value {
        match kind(request) {
            "create_session" => secret_prompt(),
            "post_auth_message_response" if request["response"] == GOOD => success(),
            "post_auth_message_response" => auth_error(),
            _ => success(),
        }
    }

    fn launch() -> Launch {
        Launch {
            command: vec!["/usr/local/libexec/sherlockos/sherlockos-session".into()],
            env: vec!["XDG_SESSION_DESKTOP=SherlockOS".into()],
        }
    }

    fn types(requests: &[Value]) -> Vec<&str> {
        requests.iter().map(kind).collect()
    }

    #[test]
    fn the_right_password_starts_the_session() {
        let fake = Fake::start(pam);
        let mut greetd = fake.connect();
        let report = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch()).unwrap();
        assert_eq!(report.outcome, Outcome::Started);
        assert!(report.notes.is_empty());
        drop(greetd);
        let requests = fake.requests();
        assert_eq!(
            types(&requests),
            [
                "create_session",
                "post_auth_message_response",
                "start_session"
            ]
        );
        assert_eq!(requests[0]["username"], "alex");
        assert_eq!(requests[1]["response"], GOOD);
        assert_eq!(
            requests[2]["cmd"],
            json!(["/usr/local/libexec/sherlockos/sherlockos-session"])
        );
        assert_eq!(
            requests[2]["env"],
            json!(["XDG_SESSION_DESKTOP=SherlockOS"])
        );
    }

    #[test]
    fn a_command_with_a_space_is_quoted_for_the_shell_of_greetd() {
        let fake = Fake::start(pam);
        let mut greetd = fake.connect();
        let launch = Launch {
            command: vec!["/opt/mon bureau/start".into(), "--x".into()],
            env: Vec::new(),
        };
        let report = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch).unwrap();
        assert_eq!(report.outcome, Outcome::Started);
        drop(greetd);
        let requests = fake.requests();
        assert_eq!(
            requests[2]["cmd"],
            json!(["'/opt/mon bureau/start'", "--x"])
        );
    }

    #[test]
    fn a_silent_greetd_is_given_up_after_the_delay() {
        let (ours, mut theirs) = UnixStream::pair().unwrap();
        let mut greetd = Greetd::with_timeout(ours, Duration::from_millis(200));
        let since = Instant::now();
        // greetd takes the request, sends a prompt, then never answers the password.
        let server = std::thread::spawn(move || {
            let first = read(&mut theirs).unwrap();
            write(&mut theirs, &secret_prompt());
            let mut seen = vec![first];
            while let Some(request) = read(&mut theirs) {
                seen.push(request);
            }
            seen
        });
        let result = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch());
        assert!(
            matches!(result, Err(greeter::greetd::Error::Timeout(_))),
            "{result:?}"
        );
        assert!(since.elapsed() < Duration::from_secs(5));
        // The session was cancelled, then the connection closed: the server reads its end.
        drop(greetd);
        let requests = server.join().unwrap();
        assert_eq!(
            types(&requests),
            [
                "create_session",
                "post_auth_message_response",
                "cancel_session"
            ]
        );
    }

    #[test]
    fn a_wrong_password_is_refused_and_the_session_cancelled() {
        let fake = Fake::start(pam);
        let mut greetd = fake.connect();
        let wrong = Secret::new("pas-le-bon".into());
        let report = log_in(&mut greetd, "alex", wrong, &launch()).unwrap();
        assert_eq!(report.outcome, Outcome::Refused);
        // The next attempt goes through on the same connection.
        let report = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch()).unwrap();
        assert_eq!(report.outcome, Outcome::Started);
        drop(greetd);
        let requests = fake.requests();
        assert_eq!(
            types(&requests)[..4],
            [
                "create_session",
                "post_auth_message_response",
                "cancel_session",
                "create_session"
            ]
        );
        assert_eq!(
            types(&requests)[4..],
            ["post_auth_message_response", "start_session"]
        );
    }

    #[test]
    fn pam_messages_are_kept_for_the_screen() {
        fn script(request: &Value) -> Value {
            match kind(request) {
                "create_session" => json!({
                    "type": "auth_message",
                    "auth_message_type": "info",
                    "auth_message": "Ton mot de passe expire dans 3 jours"
                }),
                // The info is acknowledged without a response, then comes the password.
                "post_auth_message_response" if request.get("response").is_none() => {
                    secret_prompt()
                }
                other => pam(&json!({"type": other, "response": request["response"]})),
            }
        }
        let fake = Fake::start(script);
        let mut greetd = fake.connect();
        let report = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch()).unwrap();
        assert_eq!(report.outcome, Outcome::Started);
        assert_eq!(
            report.notes,
            [Note {
                text: "Ton mot de passe expire dans 3 jours".into(),
                error: false
            }]
        );
        drop(greetd);
        let requests = fake.requests();
        assert_eq!(requests[1], json!({"type": "post_auth_message_response"}));
        assert_eq!(requests[2]["response"], GOOD);
    }

    #[test]
    fn an_unknown_user_is_refused_like_a_wrong_password() {
        // pam_unix asks for the password of an unknown user too, then refuses it.
        fn unknown(request: &Value) -> Value {
            match kind(request) {
                "create_session" if request["username"] == "personne" => secret_prompt(),
                "post_auth_message_response" => auth_error(),
                _ => success(),
            }
        }
        let fake = Fake::start(unknown);
        let mut greetd = fake.connect();
        let report = log_in(&mut greetd, "personne", Secret::new(GOOD.into()), &launch());
        assert_eq!(report.unwrap().outcome, Outcome::Refused);
        drop(greetd);
        assert_eq!(types(&fake.requests()).last(), Some(&"cancel_session"));

        // A greetd that refuses the user at once.
        fn at_once(request: &Value) -> Value {
            match kind(request) {
                "create_session" => {
                    json!({"type": "error", "error_type": "error", "description": "unknown user"})
                }
                _ => success(),
            }
        }
        let fake = Fake::start(at_once);
        let mut greetd = fake.connect();
        let report = log_in(&mut greetd, "personne", Secret::new(GOOD.into()), &launch());
        assert_eq!(
            report.unwrap().outcome,
            Outcome::Failed("unknown user".into())
        );
        drop(greetd);
        // The password never left.
        let requests = fake.requests();
        assert_eq!(types(&requests), ["create_session", "cancel_session"]);
    }

    #[test]
    fn a_session_that_cannot_start_is_reported_and_cancelled() {
        fn script(request: &Value) -> Value {
            match kind(request) {
                "start_session" => {
                    json!({"type": "error", "error_type": "error", "description": "no such command"})
                }
                _ => pam(request),
            }
        }
        let fake = Fake::start(script);
        let mut greetd = fake.connect();
        let report = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch()).unwrap();
        assert_eq!(report.outcome, Outcome::Failed("no such command".into()));
        drop(greetd);
        assert_eq!(types(&fake.requests()).last(), Some(&"cancel_session"));
    }

    #[test]
    fn a_second_secret_prompt_is_given_up_with_its_question() {
        fn script(request: &Value) -> Value {
            match kind(request) {
                "create_session" => secret_prompt(),
                "post_auth_message_response" => json!({
                    "type": "auth_message",
                    "auth_message_type": "secret",
                    "auth_message": "New password: "
                }),
                _ => success(),
            }
        }
        let fake = Fake::start(script);
        let mut greetd = fake.connect();
        let report = log_in(&mut greetd, "alex", Secret::new(GOOD.into()), &launch()).unwrap();
        assert_eq!(
            report.outcome,
            Outcome::Unsupported("New password: ".into())
        );
        drop(greetd);
        let requests = fake.requests();
        assert_eq!(types(&requests).last(), Some(&"cancel_session"));
        // The password went once only.
        let answers = requests.iter().filter(|r| r.get("response").is_some());
        assert_eq!(answers.count(), 1);
    }

    #[test]
    fn a_greeter_that_leaves_mid_login_cancels_the_session() {
        let fake = Fake::start(pam);
        let mut greetd = fake.connect();
        let reply = greetd.create_session("alex").unwrap();
        assert_eq!(
            reply,
            Reply::AuthMessage(Prompt::Secret, "Password: ".into())
        );
        // The screen stops here: its connection goes with it.
        drop(greetd);
        assert_eq!(
            types(&fake.requests()),
            ["create_session", "cancel_session"]
        );
    }

    #[test]
    fn without_greetd_the_greeter_says_so() {
        let missing = std::env::temp_dir().join(format!("no-greetd-{}", std::process::id()));
        assert!(Greetd::connect_to(&missing).is_err());
    }
}
