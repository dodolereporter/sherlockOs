//! One Control Room at a time (lot D-26n): the first one listens on a socket of the user's
//! runtime folder; a second `control-room` (from the bar, `--page network`) hands it the page
//! to show and leaves. The socket lives in `$XDG_RUNTIME_DIR`, which only the user can open;
//! what comes through it is one short line, a page's id, and anything else is ignored.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc;

use crate::search::Page;

/// Name of the socket in the runtime folder (ADR 0022: the OS's name in a path).
pub const SOCKET: &str = "sherlockos-control-room.sock";
/// The longest line read: a page's id is a word.
const MAX_LINE: u64 = 64;
/// How long a second instance waits for the first one.
const TIMEOUT: Duration = Duration::from_secs(2);

/// The pages asked by other instances, until the application takes them.
static ASKED: OnceLock<Mutex<Option<mpsc::UnboundedReceiver<Page>>>> = OnceLock::new();

/// What starting found.
pub enum Start {
    /// This is the first Control Room: it listens.
    First,
    /// Another one runs, and was asked for the page: this one leaves.
    Handed,
}

/// `$XDG_RUNTIME_DIR/sherlockos-control-room.sock`, if the runtime folder is known.
pub fn socket_path() -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)?;
    dir.is_absolute().then(|| dir.join(SOCKET))
}

/// Hands `page` to a Control Room already running at `path`, or becomes the one that listens
/// there. Without a runtime folder, every instance is the first.
pub fn start(path: Option<&Path>, page: Option<Page>) -> Start {
    let Some(path) = path else {
        return Start::First;
    };
    if let Ok(mut stream) = UnixStream::connect(path) {
        let _ = stream.set_write_timeout(Some(TIMEOUT));
        let id = page.map_or("", Page::id);
        if stream.write_all(format!("{id}\n").as_bytes()).is_ok() {
            return Start::Handed;
        }
    }
    // Nobody answers: a socket left by a Control Room that died.
    let _ = std::fs::remove_file(path);
    match UnixListener::bind(path) {
        Ok(listener) => listen(listener),
        Err(error) => tracing::warn!(%error, "another Control Room cannot reach this one"),
    }
    Start::First
}

fn listen(listener: UnixListener) {
    let (sender, receiver) = mpsc::unbounded();
    let _ = ASKED.set(Mutex::new(Some(receiver)));
    let started = std::thread::Builder::new()
        .name("control-room-instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(TIMEOUT));
                if let Some(page) = read_page(stream)
                    && sender.unbounded_send(page).is_err()
                {
                    return;
                }
            }
        });
    if let Err(error) = started {
        tracing::warn!(%error, "another Control Room cannot reach this one");
    }
}

/// The page of one line, bounded; an empty line or an unknown id: the first page.
fn read_page(stream: impl Read) -> Option<Page> {
    let mut line = String::new();
    BufReader::new(stream.take(MAX_LINE))
        .read_line(&mut line)
        .ok()?;
    Some(Page::from_id(line.trim()).unwrap_or(Page::Appearance))
}

/// The pages asked by other instances; nothing when this one does not listen.
pub fn asked() -> impl Stream<Item = Page> {
    let taken = ASKED
        .get()
        .and_then(|m| m.lock().unwrap_or_else(PoisonError::into_inner).take());
    match taken {
        Some(receiver) => iced::futures::stream::StreamExt::boxed(receiver),
        None => iced::futures::stream::StreamExt::boxed(iced::futures::stream::pending()),
    }
}

/// The page asked on the command line: `--page <id>`.
pub fn page_argument(args: impl IntoIterator<Item = String>) -> Option<Page> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--page" {
            return args.next().and_then(|id| Page::from_id(&id));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_read_from_one_short_line() {
        assert_eq!(read_page(&b"network\n"[..]), Some(Page::Network));
        assert_eq!(read_page(&b"\n"[..]), Some(Page::Appearance));
        assert_eq!(read_page(&b"nowhere\n"[..]), Some(Page::Appearance));
        let long = format!("{}network\n", " ".repeat(100));
        assert_eq!(read_page(long.as_bytes()), Some(Page::Appearance));
        let args = ["control-room", "--page", "network"].map(String::from);
        assert_eq!(page_argument(args), Some(Page::Network));
        assert_eq!(page_argument(["--page"].map(String::from)), None);
    }

    #[test]
    fn a_second_instance_hands_its_page_to_the_first() {
        let dir = std::env::temp_dir().join(format!("instance-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(SOCKET);
        assert!(matches!(start(Some(&path), None), Start::First));
        assert!(matches!(
            start(Some(&path), Some(Page::Network)),
            Start::Handed
        ));
        let mut receiver = ASKED.get().unwrap().lock().unwrap().take().unwrap();
        let mut page = None;
        for _ in 0..200 {
            if let Ok(Some(asked)) = receiver.try_next() {
                page = Some(asked);
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(page, Some(Page::Network));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
