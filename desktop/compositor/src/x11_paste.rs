//! What a Wayland client pastes from an X11 application, in the types other than text: an
//! image, a list of files (`text/uri-list`), HTML.
//!
//! smithay 0.7 (`X11Wm::send_selection`) looks the atom of such a type up in a property that
//! nothing writes, so it never reached Wayland: an image copied in Discord could not be pasted
//! in Chrome. The compositor reads it itself, as an X11 client of XWayland: in a thread of its
//! own, it asks the owner for the type, and writes what comes, INCR included, into the Wayland
//! client's pipe. The loop is never blocked. Text keeps going through smithay, which works.
//!
//! Every transfer is bounded (lot D-29b): `MAX_PASTES` at once, each one a thread and a
//! connection to XWayland, which only accepts so many clients; `MAX_BYTES` in all; and the
//! client's pipe written without blocking, each piece within `PATIENCE`, the whole within
//! `LONGEST`. A client that asks for pastes and never reads them makes them fail in turn,
//! and each one ends with its thread, its connection and its window.

use std::{
    io,
    os::fd::OwnedFd,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

use smithay::{
    reexports::rustix::{
        self,
        event::{PollFd, PollFlags, Timespec},
    },
    wayland::selection::SelectionTarget,
};
use x11rb::{
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE,
    connection::Connection as _,
    errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError},
    protocol::{
        Event,
        xproto::{
            Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, Property, WindowClass,
        },
    },
    rust_connection::RustConnection,
};

/// Longest wait for the owner of the selection, for the answer and for each piece of an INCR
/// transfer, as the X11 clipboard tools allow.
/// Also the longest wait for the Wayland client to take each piece from its pipe.
const PATIENCE: Duration = Duration::from_secs(5);

/// Longest transfer, from the request to the last byte.
const LONGEST: Duration = Duration::from_secs(60);

/// Transfers at once; one more is refused, the client's pipe closed at once.
const MAX_PASTES: usize = 4;

/// Most bytes pasted by one transfer: a picture of several screens fits.
const MAX_BYTES: usize = 256 * 1024 * 1024;

/// The same, in the 4-byte units of `GetProperty`: the most read of one piece.
const MAX_WORDS: u32 = 64 * 1024 * 1024;

/// Transfers under way.
static PASTES: AtomicUsize = AtomicUsize::new(0);

/// The types smithay reads itself: the text ones, which it maps to X11 atoms of its own.
pub(crate) fn read_by_smithay(mime_type: &str) -> bool {
    matches!(mime_type, "text/plain;charset=utf-8" | "text/plain")
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("connection to XWayland: {0}")]
    Connect(#[from] ConnectError),
    #[error("X11: {0}")]
    Connection(#[from] ConnectionError),
    #[error("X11: {0}")]
    Reply(#[from] ReplyError),
    #[error("X11: {0}")]
    ReplyOrId(#[from] ReplyOrIdError),
    #[error("pipe of the Wayland client: {0}")]
    Pipe(#[from] io::Error),
    #[error("refused by the owner")]
    Refused,
    #[error("no answer from the owner")]
    Silent,
    #[error("the Wayland client does not read its pipe")]
    Stalled,
    #[error("longer than allowed")]
    TooLong,
}

/// A place among the `MAX_PASTES` transfers, given back when dropped.
struct Place;

impl Place {
    fn take() -> Option<Self> {
        PASTES
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_PASTES).then_some(n + 1)
            })
            .ok()
            .map(|_| Self)
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        PASTES.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Reads `selection` of the X server `:display` as `mime_type`, into `fd`, in a thread.
/// Beyond `MAX_PASTES` transfers under way, `fd` is closed at once: the client reads nothing.
pub(crate) fn paste(display: u32, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
    let Some(place) = Place::take() else {
        tracing::debug!(
            ?selection,
            mime_type,
            "X11 selection not pasted: too many at once"
        );
        return;
    };
    let spawned = thread::Builder::new()
        .name("x11-paste".into())
        .spawn(move || {
            let _place = place;
            if let Err(err) = transfer(display, selection, &mime_type, fd) {
                tracing::debug!(?selection, mime_type, "X11 selection not pasted: {err}");
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("X11 selection not pasted: {err}");
    }
}

fn transfer(
    display: u32,
    selection: SelectionTarget,
    mime_type: &str,
    fd: OwnedFd,
) -> Result<(), Error> {
    // Written without blocking, with a delay (`Pipe::write_all`).
    rustix::fs::fcntl_getfl(&fd)
        .and_then(|flags| rustix::fs::fcntl_setfl(&fd, flags | rustix::fs::OFlags::NONBLOCK))
        .map_err(io::Error::from)?;
    let mut pipe = Pipe {
        fd,
        written: 0,
        deadline: Instant::now() + LONGEST,
        patience: PATIENCE,
    };
    let (conn, screen) = RustConnection::connect(Some(&format!(":{display}")))?;
    let root = conn.setup().roots[screen].root;
    let window = conn.generate_id()?;
    let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    let class = WindowClass::INPUT_ONLY;
    let depth = COPY_DEPTH_FROM_PARENT;
    conn.create_window(
        depth,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        class,
        COPY_FROM_PARENT,
        &aux,
    )?;
    let name = match selection {
        SelectionTarget::Clipboard => "CLIPBOARD",
        SelectionTarget::Primary => "PRIMARY",
    };
    let atom = |name: &str| -> Result<Atom, Error> {
        Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
    };
    let (selection, target) = (atom(name)?, atom(mime_type)?);
    let (property, incr) = (atom("_SHERLOCKOS_PASTE")?, atom("INCR")?);
    conn.convert_selection(window, selection, target, property, CURRENT_TIME)?;
    conn.flush()?;
    let notified = next_event(&conn, |event| match event {
        Event::SelectionNotify(n) => Some(n.property),
        _ => None,
    })?;
    if notified == NONE {
        return Err(Error::Refused);
    }
    // Reading the property with `delete` asks the owner for the next piece of INCR.
    let take = || -> Result<_, Error> {
        let any = AtomEnum::ANY;
        Ok(conn
            .get_property(true, window, property, any, 0, MAX_WORDS)?
            .reply()?)
    };
    let first = take()?;
    if first.type_ != incr {
        pipe.write_all(&first.value)?;
        return Ok(());
    }
    loop {
        next_event(&conn, |event| match event {
            Event::PropertyNotify(n) if n.atom == property && n.state == Property::NEW_VALUE => {
                Some(())
            }
            _ => None,
        })?;
        let piece = take()?.value;
        // The empty piece ends the transfer.
        if piece.is_empty() {
            return Ok(());
        }
        pipe.write_all(&piece)?;
    }
}

/// The Wayland client's end, written without blocking.
struct Pipe {
    fd: OwnedFd,
    /// Bytes written so far, up to `MAX_BYTES`.
    written: usize,
    /// End of the whole transfer.
    deadline: Instant,
    /// Longest wait for the pipe to take more.
    patience: Duration,
}

impl Pipe {
    /// Writes `bytes`, waiting `patience` at most each time the pipe is full.
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<(), Error> {
        self.written = self.written.saturating_add(bytes.len());
        if self.written > MAX_BYTES || Instant::now() > self.deadline {
            return Err(Error::TooLong);
        }
        while !bytes.is_empty() {
            match rustix::io::write(&self.fd, bytes) {
                Ok(n) => bytes = &bytes[n..],
                Err(rustix::io::Errno::INTR) => {}
                Err(rustix::io::Errno::AGAIN) => self.wait()?,
                Err(err) => return Err(io::Error::from(err).into()),
            }
        }
        Ok(())
    }

    /// Until the pipe takes more, within `patience`.
    fn wait(&self) -> Result<(), Error> {
        let patience = Timespec::try_from(self.patience).map_err(|_| Error::Stalled)?;
        let mut fds = [PollFd::new(&self.fd, PollFlags::OUT)];
        match rustix::event::poll(&mut fds, Some(&patience)) {
            Ok(0) => Err(Error::Stalled),
            Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
            Err(err) => Err(io::Error::from(err).into()),
        }
    }
}

/// The first event that `wanted` keeps, within `PATIENCE`.
fn next_event<T>(conn: &RustConnection, wanted: impl Fn(&Event) -> Option<T>) -> Result<T, Error> {
    let deadline = Instant::now() + PATIENCE;
    loop {
        match conn.poll_for_event()? {
            Some(event) => {
                if let Some(found) = wanted(&event) {
                    return Ok(found);
                }
            }
            None if Instant::now() > deadline => return Err(Error::Silent),
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_text_is_left_to_smithay() {
        assert!(read_by_smithay("text/plain;charset=utf-8"));
        assert!(read_by_smithay("text/plain"));
        assert!(!read_by_smithay("image/png"));
        assert!(!read_by_smithay("text/uri-list"));
        assert!(!read_by_smithay("text/html"));
    }

    #[test]
    fn a_client_that_does_not_read_stalls_the_transfer_within_the_delay() {
        let (reader, writer) = rustix::pipe::pipe().unwrap();
        rustix::fs::fcntl_setfl(&writer, rustix::fs::OFlags::NONBLOCK).unwrap();
        let mut pipe = Pipe {
            fd: writer,
            written: 0,
            deadline: Instant::now() + LONGEST,
            patience: Duration::from_millis(50),
        };
        pipe.write_all(b"small").unwrap();
        let since = Instant::now();
        // More than a pipe holds (64 KiB by default), nobody reading.
        let result = pipe.write_all(&vec![7; 1024 * 1024]);
        assert!(matches!(result, Err(Error::Stalled)), "{result:?}");
        assert!(since.elapsed() < Duration::from_secs(2));
        // What was written is there to read.
        let mut start = [0; 5];
        rustix::io::read(&reader, &mut start).unwrap();
        assert_eq!(&start, b"small");
    }

    #[test]
    fn a_transfer_beyond_the_bound_is_refused() {
        let (_reader, writer) = rustix::pipe::pipe().unwrap();
        let mut pipe = Pipe {
            fd: writer,
            written: MAX_BYTES,
            deadline: Instant::now() + LONGEST,
            patience: PATIENCE,
        };
        assert!(matches!(pipe.write_all(b"x"), Err(Error::TooLong)));
        pipe.written = 0;
        pipe.deadline = Instant::now() - Duration::from_millis(1);
        assert!(matches!(pipe.write_all(b"x"), Err(Error::TooLong)));
    }

    #[test]
    fn only_so_many_transfers_at_once() {
        let places: Vec<Place> = std::iter::from_fn(Place::take)
            .take(MAX_PASTES + 1)
            .collect();
        // Other tests of this file take no place: all of them are this test's.
        assert_eq!(places.len(), MAX_PASTES);
        assert!(Place::take().is_none());
        drop(places);
        assert!(Place::take().is_some());
    }
}
