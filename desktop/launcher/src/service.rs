//! Resident service (lot P-02): the program stays in memory from the start of the session, its
//! surface shown or hidden at once, as Spotlight and Launchpad do on macOS. Starting a renderer,
//! its pipelines and the fonts took about 200 ms on every press when the program was started on
//! demand.
//!
//! The service listens on `$XDG_RUNTIME_DIR/<name>.sock`. Started again by a key or a button,
//! the program finds the socket, sends one line (`toggle`) and stops: the second press hides
//! what the first one showed. Without a service listening, the program runs on its own, as
//! before. The runtime folder belongs to the user alone (mode 0700): only the user's programs
//! reach the socket. A lock file (`<name>.service.lock`) keeps one service at a time; the
//! kernel drops it with its holder, crash included, and a socket left behind is replaced.

use std::fs::{File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use iced::futures::stream::{self, StreamExt};

/// The launcher's service.
pub const LAUNCHER: &str = "launcher";
/// The applications view's service, which the launcher hides as it opens, and the other way.
pub const APPLICATIONS: &str = "sherlockos-apps";

/// How long a client waits for the service, and the service for a client's line.
pub const TIMEOUT: Duration = Duration::from_secs(1);

/// The longest line read from a client.
const MAX_LINE: u64 = 64;

/// What a client asks the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Request {
    /// Shows the surface if it is hidden, hides it otherwise.
    Toggle,
    /// Hides the surface if it is shown: another overlay of the shell opens, or four fingers
    /// spread out over the applications view (lot D-41c).
    Hide,
    /// Shows the surface if it is hidden, leaves it otherwise: four fingers pinching in.
    Show,
}

impl Request {
    pub fn word(self) -> &'static str {
        match self {
            Self::Toggle => "toggle",
            Self::Hide => "hide",
            Self::Show => "show",
        }
    }

    pub fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "toggle" => Some(Self::Toggle),
            "hide" => Some(Self::Hide),
            "show" => Some(Self::Show),
            _ => None,
        }
    }
}

/// The socket of the service `name` in `dir`.
pub fn socket(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.sock"))
}

/// Asks the service listening at `path`; `Ok` once it answered. An error when nobody listens
/// (`NotFound`, `ConnectionRefused`): the caller then runs on its own.
pub fn send(path: &Path, request: Request) -> io::Result<()> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.write_all(format!("{}\n", request.word()).as_bytes())?;
    let mut reply = String::new();
    stream.take(MAX_LINE).read_to_string(&mut reply)?;
    if reply.trim() == "ok" {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("the service answered {reply:?}"),
        ))
    }
}

/// A service listening: its socket, and the lock that keeps it alone.
#[derive(Debug)]
pub struct Service {
    listener: UnixListener,
    path: PathBuf,
    /// Held as long as the service: the kernel drops it with the process.
    lock: File,
}

/// Starts listening as the service `name` in `dir`; `None` when another service holds the lock.
pub fn listen(dir: &Path, name: &str) -> io::Result<Option<Service>> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(format!("{name}.service.lock")))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(error)) => return Err(error),
    }
    let path = socket(dir, name);
    // Left by a service that died: the lock is ours, so nobody listens there any more.
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(&path)?;
    Ok(Some(Service {
        listener,
        path,
        lock,
    }))
}

impl Service {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Answers the clients on a thread of its own; their requests arrive in the inbox.
    pub fn spawn(self) -> io::Result<Inbox> {
        let (sender, receiver) = mpsc::unbounded();
        let Self {
            listener,
            path,
            lock,
        } = self;
        std::thread::Builder::new()
            .name("service".into())
            .spawn(move || {
                // Held as long as the thread, that is the process.
                let _held = lock;
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    match answer(stream) {
                        Ok(Some(request)) => {
                            if sender.unbounded_send(request).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => tracing::warn!(%error, "service client"),
                    }
                }
                tracing::warn!(path = %path.display(), "the service stopped listening");
            })?;
        Ok(Inbox(Arc::new(Mutex::new(Some(receiver)))))
    }
}

/// Reads one line from a client and acknowledges it.
fn answer(stream: UnixStream) -> io::Result<Option<Request>> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_LINE)).read_line(&mut line)?;
    let request = Request::parse(&line);
    let reply: &[u8] = if request.is_some() {
        b"ok\n"
    } else {
        b"unknown\n"
    };
    (&stream).write_all(reply)?;
    Ok(request)
}

/// The requests of the clients, handed once to the subscription.
#[derive(Debug, Clone)]
pub struct Inbox(Arc<Mutex<Option<mpsc::UnboundedReceiver<Request>>>>);

impl Inbox {
    /// An inbox fed by `receiver` (the tests).
    pub fn from_receiver(receiver: mpsc::UnboundedReceiver<Request>) -> Self {
        Self(Arc::new(Mutex::new(Some(receiver))))
    }

    /// The requests, for `Subscription::run_with`: the first call takes them.
    pub fn stream(&self) -> impl Stream<Item = Request> + use<> {
        match self.0.lock().ok().and_then(|mut r| r.take()) {
            Some(receiver) => receiver.boxed(),
            None => stream::empty().boxed(),
        }
    }
}

impl Hash for Inbox {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "service inbox".hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::executor::block_on;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("service-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("folder");
        dir
    }

    #[test]
    fn every_request_reaches_the_service_once_acknowledged() {
        let dir = temp("requests");
        let service = listen(&dir, "test").expect("listen").expect("first");
        let path = service.path().to_owned();
        // A second service finds the first one there.
        assert!(listen(&dir, "test").expect("listen").is_none());
        let inbox = service.spawn().expect("thread");
        send(&path, Request::Toggle).expect("toggle");
        send(&path, Request::Toggle).expect("second toggle");
        send(&path, Request::Hide).expect("hide");
        let mut requests = inbox.stream();
        let got: Vec<Request> = block_on(async {
            let mut got = Vec::new();
            for _ in 0..3 {
                got.extend(requests.next().await);
            }
            got
        });
        assert_eq!(got, [Request::Toggle, Request::Toggle, Request::Hide]);
        // The stream is handed once.
        assert!(block_on(inbox.stream().next()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nobody_listening_is_an_error_and_a_stale_socket_is_replaced() {
        let dir = temp("stale");
        let path = socket(&dir, "test");
        assert!(send(&path, Request::Toggle).is_err());
        // A socket left by a service that died: nobody accepts there.
        drop(UnixListener::bind(&path).expect("bound"));
        assert!(send(&path, Request::Toggle).is_err());
        let service = listen(&dir, "test").expect("listen").expect("first");
        let _inbox = service.spawn().expect("thread");
        send(&path, Request::Toggle).expect("toggle");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_line_is_refused() {
        let dir = temp("unknown");
        let service = listen(&dir, "test").expect("listen").expect("first");
        let path = service.path().to_owned();
        let _inbox = service.spawn().expect("thread");
        let mut stream = UnixStream::connect(&path).expect("connected");
        stream.write_all(b"quit\n").expect("written");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("reply");
        assert_eq!(reply, "unknown\n");
        assert_eq!(Request::parse(" toggle \n"), Some(Request::Toggle));
        assert_eq!(Request::parse("TOGGLE"), None);
        for request in [Request::Toggle, Request::Hide, Request::Show] {
            assert_eq!(Request::parse(request.word()), Some(request));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
