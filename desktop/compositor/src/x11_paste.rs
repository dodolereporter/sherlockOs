//! What a Wayland client pastes from an X11 application, in the types other than text: an
//! image, a list of files (`text/uri-list`), HTML.
//!
//! smithay 0.7 (`X11Wm::send_selection`) looks the atom of such a type up in a property that
//! nothing writes, so it never reached Wayland: an image copied in Discord could not be pasted
//! in Chrome. The compositor reads it itself, as an X11 client of XWayland: in a thread of its
//! own, it asks the owner for the type, and writes what comes, INCR included, into the Wayland
//! client's pipe. The loop is never blocked. Text keeps going through smithay, which works.

use std::{
    fs::File,
    io::{self, Write as _},
    os::fd::OwnedFd,
    thread,
    time::{Duration, Instant},
};

use smithay::{reexports::rustix, wayland::selection::SelectionTarget};
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
const PATIENCE: Duration = Duration::from_secs(5);

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
}

/// Reads `selection` of the X server `:display` as `mime_type`, into `fd`, in a thread.
pub(crate) fn paste(display: u32, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
    let spawned = thread::Builder::new()
        .name("x11-paste".into())
        .spawn(move || {
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
    // The client's end may be non-blocking: this thread writes plainly, and waits.
    rustix::fs::fcntl_getfl(&fd)
        .and_then(|flags| rustix::fs::fcntl_setfl(&fd, flags - rustix::fs::OFlags::NONBLOCK))
        .map_err(io::Error::from)?;
    let mut pipe = File::from(fd);
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
            .get_property(true, window, property, any, 0, u32::MAX / 4)?
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
}
