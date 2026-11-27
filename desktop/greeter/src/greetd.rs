//! A client of greetd's IPC (`greetd-ipc(7)`), on the socket of `GREETD_SOCK`. Each message is
//! a JSON object behind its length, a 32-bit unsigned integer in the machine's byte order.
//!
//! The requests are written here rather than by serde: the answer to a secret prompt goes into
//! a buffer of its exact final size, overwritten with zeros once sent (`zeroize`), so that no
//! growing buffer leaves a copy of the password behind. Nothing here logs a request.
//!
//! The replies are read with a bound (`MAX_REPLY`): greetd runs as root, but a reply that size
//! would be a broken socket, not a message. And with a delay (`REPLY_TIMEOUT`): a greetd that
//! does not answer leaves the screen on its spinner for that long at most, then the session
//! being created is cancelled and the connection closed; the next attempt opens a new one.
//!
//! greetd runs the session's command with `/bin/sh -c "exec " + cmd.join(" ")`
//! (`greetd/src/session/worker.rs`): `start_session` quotes each word for `sh` (`shell_word`),
//! so that a word is never split again nor read by the shell.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use system_services::secret::Secret;
use zeroize::Zeroizing;

/// Longest reply read from greetd, in bytes.
pub const MAX_REPLY: u32 = 64 * 1024;

/// The variable that greetd gives its greeter, with the path of its socket.
pub const SOCKET_VAR: &str = "GREETD_SOCK";

/// Longest wait for a reply of greetd, or for it to take a request. Beyond PAM's own delays (2
/// to 3 s after a wrong password, `pam_faildelay`), and those of a slow module.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// Wait for greetd to take the cancellation, once it did not answer in time.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("GREETD_SOCK is not set: the login screen runs under greetd only")]
    NoSocket,
    #[error("greetd's socket: {0}")]
    Io(#[from] std::io::Error),
    #[error("greetd's reply is {0} bytes long, more than allowed")]
    TooLong(u32),
    #[error("greetd's reply is not understood: {0}")]
    Malformed(String),
    #[error("greetd did not answer within {} s", .0.as_secs())]
    Timeout(Duration),
}

/// What kind of prompt PAM sends through greetd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// A question whose answer may show (a user name).
    Visible,
    /// A question whose answer must not show: the password.
    Secret,
    /// A message to show, which asks for nothing.
    Info,
    /// An error to show, which asks for nothing.
    Error,
}

/// What greetd says about the last request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Success,
    /// The authentication failed (`auth_error`): a wrong password, or an unknown user, which
    /// PAM does not tell apart.
    AuthError(String),
    /// Anything else that failed (`error`).
    Error(String),
    AuthMessage(Prompt, String),
}

/// A connection to greetd. A session being created when it is dropped is cancelled, so that
/// greetd is never left waiting for an answer that will not come.
#[derive(Debug)]
pub struct Greetd {
    stream: UnixStream,
    /// A session was created and neither started nor cancelled.
    open: bool,
    /// Longest wait for each reply.
    timeout: Duration,
}

impl Greetd {
    /// Connects to the socket of `GREETD_SOCK`.
    pub fn connect() -> Result<Self, Error> {
        let path = std::env::var_os(SOCKET_VAR).ok_or(Error::NoSocket)?;
        Self::connect_to(Path::new(&path))
    }

    pub fn connect_to(path: &Path) -> Result<Self, Error> {
        Ok(Self::from_stream(UnixStream::connect(path)?))
    }

    pub fn from_stream(stream: UnixStream) -> Self {
        Self::with_timeout(stream, REPLY_TIMEOUT)
    }

    /// The connection on `stream`, waiting `timeout` at most for each reply.
    pub fn with_timeout(stream: UnixStream, timeout: Duration) -> Self {
        // Fails only for a zero delay.
        if let Err(err) = stream
            .set_read_timeout(Some(timeout))
            .and_then(|()| stream.set_write_timeout(Some(timeout)))
        {
            tracing::warn!("greetd: no delay on the socket: {err}");
        }
        Self {
            stream,
            open: false,
            timeout,
        }
    }

    /// Starts the authentication of `user`: greetd answers with a prompt, or an error.
    pub fn create_session(&mut self, user: &str) -> Result<Reply, Error> {
        let mut out = Request::new("create_session");
        out.field("username", user);
        self.open = true;
        self.exchange(out)
    }

    /// Answers the last prompt: the password to a secret prompt, nothing to a message.
    pub fn answer(&mut self, response: Option<&Secret>) -> Result<Reply, Error> {
        let mut out = Request::new("post_auth_message_response");
        if let Some(response) = response {
            out.secret_field("response", response.as_str());
        }
        self.exchange(out)
    }

    /// Asks greetd to start `command`, with `env` added to what PAM gives, once this greeter
    /// has left.
    pub fn start_session(&mut self, command: &[String], env: &[String]) -> Result<Reply, Error> {
        let mut out = Request::new("start_session");
        let words: Vec<String> = command.iter().map(|word| shell_word(word)).collect();
        out.list("cmd", &words);
        out.list("env", env);
        let reply = self.exchange(out)?;
        if reply == Reply::Success {
            self.open = false;
        }
        Ok(reply)
    }

    /// Gives up the session being created.
    pub fn cancel_session(&mut self) -> Result<Reply, Error> {
        self.open = false;
        self.exchange(Request::new("cancel_session"))
    }

    fn exchange(&mut self, request: Request) -> Result<Reply, Error> {
        let result = self.send(request).and_then(|()| self.receive());
        match result {
            Err(Error::Io(err))
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                self.give_up();
                Err(Error::Timeout(self.timeout))
            }
            other => other,
        }
    }

    /// greetd did not answer in time: the session being created is cancelled, without waiting
    /// for the reply, and the connection is closed. A late reply would come out of step.
    fn give_up(&mut self) {
        if std::mem::take(&mut self.open) {
            let _ = self.stream.set_write_timeout(Some(CANCEL_TIMEOUT));
            if let Ok(frame) = Request::new("cancel_session").frame() {
                let _ = self.stream.write_all(&frame);
            }
        }
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn send(&mut self, request: Request) -> Result<(), Error> {
        let frame = request.frame()?;
        self.stream.write_all(&frame)?;
        Ok(())
    }

    fn receive(&mut self) -> Result<Reply, Error> {
        let mut length = [0; 4];
        self.stream.read_exact(&mut length)?;
        let length = u32::from_ne_bytes(length);
        if length > MAX_REPLY {
            return Err(Error::TooLong(length));
        }
        let mut body = vec![0; length as usize];
        self.stream.read_exact(&mut body)?;
        parse(&body)
    }
}

impl Drop for Greetd {
    fn drop(&mut self) {
        if self.open {
            let _ = self.cancel_session();
        }
    }
}

/// A request being written: a JSON object, in a buffer overwritten with zeros when dropped.
struct Request {
    /// The fields, each `"name":value`, the secret field last.
    body: Zeroizing<Vec<u8>>,
    /// The answer to a secret prompt, escaped later straight into the frame.
    secret: Option<(&'static str, Zeroizing<String>)>,
}

impl Request {
    fn new(kind: &str) -> Self {
        let mut body = Zeroizing::new(Vec::new());
        body.extend_from_slice(b"\"type\":");
        push_string(&mut body, kind);
        Self { body, secret: None }
    }

    fn field(&mut self, name: &str, value: &str) {
        self.name(name);
        push_string(&mut self.body, value);
    }

    fn list(&mut self, name: &str, values: &[String]) {
        self.name(name);
        self.body.push(b'[');
        for (i, value) in values.iter().enumerate() {
            if i > 0 {
                self.body.push(b',');
            }
            push_string(&mut self.body, value);
        }
        self.body.push(b']');
    }

    fn secret_field(&mut self, name: &'static str, value: &str) {
        // A copy of exactly that size, wiped when the request goes.
        let mut copy = Zeroizing::new(String::with_capacity(value.len()));
        copy.push_str(value);
        self.secret = Some((name, copy));
    }

    fn name(&mut self, name: &str) {
        self.body.push(b',');
        push_string(&mut self.body, name);
        self.body.push(b':');
    }

    /// The length and the object, in one buffer allocated once at its final size.
    fn frame(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let secret_len = self
            .secret
            .as_ref()
            .map_or(0, |(name, value)| 4 + name.len() + escaped_len(value));
        let len = 2 + self.body.len() + secret_len;
        let header = u32::try_from(len).map_err(|_| Error::Malformed("request too long".into()))?;
        let mut frame = Zeroizing::new(Vec::with_capacity(4 + len));
        frame.extend_from_slice(&header.to_ne_bytes());
        frame.push(b'{');
        frame.extend_from_slice(&self.body);
        if let Some((name, value)) = &self.secret {
            frame.push(b',');
            push_string(&mut frame, name);
            frame.push(b':');
            push_string(&mut frame, value);
        }
        frame.push(b'}');
        debug_assert_eq!(frame.len(), frame.capacity());
        Ok(frame)
    }
}

/// `word` as one word for `sh`: as it is when no character of it means anything to the shell,
/// else between single quotes, a quote in it written `'\''`.
pub fn shell_word(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_-./:=@%+,".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        return word.to_owned();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The length of `value` once escaped and quoted by `push_string`.
fn escaped_len(value: &str) -> usize {
    2 + value
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\r' | '\t' => 2,
            c if u32::from(c) < 0x20 => 6,
            c => c.len_utf8(),
        })
        .sum::<usize>()
}

/// Appends `value` as a JSON string.
fn push_string(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    let mut buffer = [0; 4];
    for c in value.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if u32::from(c) < 0x20 => {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let n = u32::from(c) as usize;
                out.extend_from_slice(b"\\u00");
                out.push(HEX[n >> 4]);
                out.push(HEX[n & 0xf]);
            }
            c => out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes()),
        }
    }
    buffer.fill(0);
    out.push(b'"');
}

/// Reads a reply of greetd.
pub fn parse(body: &[u8]) -> Result<Reply, Error> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|err| Error::Malformed(err.to_string()))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match value.get("type").and_then(serde_json::Value::as_str) {
        Some("success") => Ok(Reply::Success),
        Some("error") => match value.get("error_type").and_then(serde_json::Value::as_str) {
            Some("auth_error") => Ok(Reply::AuthError(text("description"))),
            _ => Ok(Reply::Error(text("description"))),
        },
        Some("auth_message") => {
            let prompt = match value.get("auth_message_type").and_then(|v| v.as_str()) {
                Some("visible") => Prompt::Visible,
                Some("secret") => Prompt::Secret,
                Some("info") => Prompt::Info,
                Some("error") => Prompt::Error,
                other => return Err(Error::Malformed(format!("auth_message_type {other:?}"))),
            };
            Ok(Reply::AuthMessage(prompt, text("auth_message")))
        }
        other => Err(Error::Malformed(format!("type {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(request: &Request) -> serde_json::Value {
        let frame = request.frame().unwrap();
        let length = u32::from_ne_bytes([frame[0], frame[1], frame[2], frame[3]]);
        assert_eq!(length as usize, frame.len() - 4);
        serde_json::from_slice(&frame[4..]).unwrap()
    }

    #[test]
    fn requests_are_the_json_of_greetd_ipc() {
        let mut create = Request::new("create_session");
        create.field("username", "alex");
        assert_eq!(
            body(&create),
            serde_json::json!({"type": "create_session", "username": "alex"})
        );
        let mut start = Request::new("start_session");
        start.list("cmd", &["sherlockos-session".into(), "--x".into()]);
        start.list("env", &[]);
        assert_eq!(
            body(&start),
            serde_json::json!({"type": "start_session", "cmd": ["sherlockos-session", "--x"], "env": []})
        );
    }

    #[test]
    fn a_secret_is_escaped_into_a_frame_of_its_exact_size() {
        for password in ["simple", "a\"b\\c", "tab\tnew\nline\u{1}", "é€𝄞", ""] {
            let mut answer = Request::new("post_auth_message_response");
            answer.secret_field("response", password);
            assert_eq!(
                body(&answer),
                serde_json::json!({"type": "post_auth_message_response", "response": password})
            );
        }
    }

    #[test]
    fn each_word_reaches_the_session_whole_through_sh() {
        assert_eq!(
            shell_word("/usr/local/libexec/sherlockos/sherlockos-session"),
            "/usr/local/libexec/sherlockos/sherlockos-session"
        );
        assert_eq!(
            shell_word("/opt/mon bureau/start"),
            "'/opt/mon bureau/start'"
        );
        assert_eq!(shell_word("it's"), r"'it'\''s'");
        assert_eq!(shell_word(""), "''");
        // What greetd runs: `sh -c "exec " + cmd.join(" ")`, here with printf as the session.
        let words = [
            "printf",
            "[%s]",
            "/opt/mon bureau/start",
            "$HOME;*'\"&|`id`\\",
            "~",
            "",
            "a\nb",
        ];
        let quoted: Vec<String> = words.iter().map(|w| shell_word(w)).collect();
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("exec {}", quoted.join(" ")))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        let expected: String = words[2..].iter().map(|w| format!("[{w}]")).collect();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }

    #[test]
    fn replies_are_read() {
        let parsed = |json: &str| parse(json.as_bytes()).unwrap();
        assert_eq!(parsed(r#"{"type":"success"}"#), Reply::Success);
        assert_eq!(
            parsed(r#"{"type":"error","error_type":"auth_error","description":"no"}"#),
            Reply::AuthError("no".into())
        );
        assert_eq!(
            parsed(r#"{"type":"error","error_type":"error","description":"busy"}"#),
            Reply::Error("busy".into())
        );
        assert_eq!(
            parsed(
                r#"{"type":"auth_message","auth_message_type":"secret","auth_message":"Password: "}"#
            ),
            Reply::AuthMessage(Prompt::Secret, "Password: ".into())
        );
        for broken in [
            r#"{"type":"what"}"#,
            "not json",
            r#"{"type":"auth_message"}"#,
        ] {
            assert!(parse(broken.as_bytes()).is_err(), "{broken}");
        }
    }
}
