//! Asking the user through the compositor's consent window (lot A-11c, ADR 0031): the
//! [`Consent`] of a session.
//!
//! One connection per request to the compositor's consent socket, `<Wayland socket>-consent`:
//! the request written as one line of JSON, the answer read as one. The connection goes with the
//! request: when the broker stops waiting (`CONSENT_TIMEOUT`), the future that holds it is
//! dropped, the socket closes, and the compositor withdraws the window. The socket's peer must
//! be the compositor of this user (`SO_PEERCRED`, `/proc/<pid>/exe`): a process that took the
//! path first gets no question, and grants nothing.
//!
//! Every failure is `Decision::Unavailable`: no socket, a peer that is not the compositor, a
//! connection lost, an answer that cannot be read. Only `"granted"` for this request's id
//! grants.

use std::future::Future;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::broker::{Consent, Decision, Request};
use crate::mcp::Level;

/// Bytes of an answer at most: more is not an answer.
const MAX_ANSWER: usize = 1024;
/// The compositor's program name.
const COMPOSITOR: &str = "compositor";

/// The compositor's consent window, at `path`.
#[derive(Debug, Clone)]
pub struct CompositorWindow {
    path: PathBuf,
    /// The program the socket's peer must be (`compositor`; anything in the tests).
    peer: Option<String>,
}

impl CompositorWindow {
    /// The window of the compositor whose socket is at `path`.
    pub fn at(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            peer: Some(COMPOSITOR.to_owned()),
        }
    }

    /// The window of the session: next to the Wayland socket of `XDG_RUNTIME_DIR` and
    /// `WAYLAND_DISPLAY`. `None` outside a Wayland session.
    pub fn of_session() -> Option<Self> {
        let display = PathBuf::from(std::env::var_os("WAYLAND_DISPLAY")?);
        let socket = if display.is_absolute() {
            display
        } else {
            PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(display)
        };
        let mut name = socket.file_name()?.to_owned();
        name.push("-consent");
        Some(Self::at(&socket.with_file_name(name)))
    }

    /// Where its socket is.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The compositor's windows socket, next to this one: `<Wayland socket>-sherlock`, for
    /// the windows server (A-12).
    pub fn windows_socket(&self) -> Option<PathBuf> {
        let name = self.path.file_name()?.to_str()?.strip_suffix("-consent")?;
        Some(self.path.with_file_name(format!("{name}-sherlock")))
    }

    /// Any process of this user may answer: for the tests, whose fake compositor is the test.
    pub fn trusting_any_program(mut self) -> Self {
        self.peer = None;
        self
    }

    async fn ask_once(&self, request: &Request) -> Option<Decision> {
        let level = match request.level {
            // A read is never asked: refused if it ever is.
            Level::Read => return Some(Decision::Denied),
            Level::Action => "action",
            Level::Destructive => "destructive",
        };
        let stream = UnixStream::connect(&self.path).await.ok()?;
        if !peer_is(&stream, self.peer.as_deref()) {
            tracing::error!(path = ?self.path, "consent socket held by another program");
            return None;
        }
        let (read, mut write) = stream.into_split();
        let line = json!({
            "id": request.id,
            "tool": request.tool,
            "level": level,
            "arguments": request.shown,
        });
        write.write_all(format!("{line}\n").as_bytes()).await.ok()?;
        let mut reader = BufReader::new(read).take(MAX_ANSWER as u64);
        let mut answer = String::new();
        reader.read_line(&mut answer).await.ok()?;
        let answer: Value = serde_json::from_str(answer.trim()).ok()?;
        if answer["id"].as_u64() != Some(request.id) {
            return None;
        }
        Some(match answer["decision"].as_str() {
            Some("granted") => Decision::Granted,
            _ => Decision::Denied,
        })
    }
}

impl Consent for CompositorWindow {
    fn ask<'a>(
        &'a mut self,
        request: &'a Request,
    ) -> Pin<Box<dyn Future<Output = Decision> + Send + 'a>> {
        Box::pin(async move {
            self.ask_once(request)
                .await
                .unwrap_or(Decision::Unavailable)
        })
    }
}

/// Whether the peer of `stream` is a process of this user, running `program` when given: the
/// consent socket here, the windows socket for the windows server (A-12).
pub(crate) fn peer_is(stream: &impl AsRawFd, program: Option<&str>) -> bool {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `credentials` and `length` are valid for writing, and `length` is the size of
    // `credentials`, as `SO_PEERCRED` requires; the descriptor is the stream's, open.
    let read = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &mut length,
        )
    };
    // SAFETY: `getuid` has no precondition and cannot fail.
    if read != 0 || credentials.uid != unsafe { libc::getuid() } {
        return false;
    }
    let Some(program) = program else {
        return true;
    };
    let exe = std::fs::read_link(format!("/proc/{}/exe", credentials.pid)).ok();
    let name = exe
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    name.is_some_and(|n| n.strip_suffix(" (deleted)").unwrap_or(n) == program)
}
