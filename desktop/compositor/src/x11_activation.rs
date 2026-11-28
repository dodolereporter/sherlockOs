//! `_NET_ACTIVE_WINDOW` from X11 applications (lot Q-52). smithay 0.7's window manager does
//! not read this client message: Discord, Steam or AnyDesk under XWayland asked for their
//! window in vain. The compositor hears it itself, as an X11 client of XWayland that watches
//! the root window's substructure (an application sends the message there with that mask),
//! read in the loop without blocking. `activation` decides.

use std::{io, os::fd::OwnedFd};

use smithay::reexports::{
    calloop::{Interest, Mode, PostAction, generic::Generic},
    rustix,
};
use x11rb::{
    connection::Connection as _,
    errors::{ConnectError, ConnectionError, ReplyError},
    protocol::{
        Event,
        xproto::{Atom, ChangeWindowAttributesAux, ConnectionExt as _, EventMask},
    },
    rust_connection::RustConnection,
};

use crate::State;

/// Requests read in one turn of the loop at most: a client that floods the root window cannot
/// hold the loop.
const PER_TURN: usize = 64;

/// The connection that hears the requests.
pub(crate) struct Listener {
    conn: RustConnection,
    active_window: Atom,
}

/// What may go wrong while connecting.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("connecting to XWayland: {0}")]
    Connect(#[from] ConnectError),
    #[error("talking to XWayland: {0}")]
    Connection(#[from] ConnectionError),
    #[error("an answer of XWayland: {0}")]
    Reply(#[from] ReplyError),
    #[error("the connection's socket: {0}")]
    Io(#[from] io::Error),
}

impl Listener {
    fn connect(display: u32) -> Result<Self, Error> {
        let (conn, screen) = RustConnection::connect(Some(&format!(":{display}")))?;
        let root = conn.setup().roots[screen].root;
        let active_window = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let aux = ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY);
        conn.change_window_attributes(root, &aux)?.check()?;
        Ok(Self {
            conn,
            active_window,
        })
    }

    fn fd(&self) -> io::Result<OwnedFd> {
        Ok(rustix::io::dup(self.conn.stream())?)
    }

    /// The requests that came: window and source of each. `Err` once the connection is lost.
    fn requests(&self) -> Result<Vec<(u32, u32)>, ConnectionError> {
        let mut requests = Vec::new();
        for _ in 0..PER_TURN {
            match self.conn.poll_for_event()? {
                Some(Event::ClientMessage(message)) if message.type_ == self.active_window => {
                    let source = message.data.as_data32()[0];
                    requests.push((message.window, source));
                }
                Some(_) => {}
                None => break,
            }
        }
        Ok(requests)
    }
}

impl State {
    /// Hears `_NET_ACTIVE_WINDOW` on XWayland's display `display`, for as long as it answers.
    pub(crate) fn listen_x11_activation(&mut self, display: u32) {
        let listener = match Listener::connect(display) {
            Ok(listener) => listener,
            Err(err) => {
                tracing::warn!("X11 applications cannot activate their windows: {err}");
                return;
            }
        };
        let fd = match listener.fd() {
            Ok(fd) => fd,
            Err(err) => {
                tracing::warn!("X11 applications cannot activate their windows: {err}");
                return;
            }
        };
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        let inserted =
            self.loop_handle
                .insert_source(source, move |_, _, state| match listener.requests() {
                    Ok(requests) => {
                        for (window, source) in requests {
                            if let Some(surface) = state.x11_surface_by_id(window) {
                                state.x11_activation(&surface, source);
                            }
                        }
                        Ok(PostAction::Continue)
                    }
                    Err(err) => {
                        tracing::debug!("XWayland gone for the activation requests: {err}");
                        Ok(PostAction::Remove)
                    }
                });
        if let Err(err) = inserted {
            tracing::warn!("X11 applications cannot activate their windows: {err}");
        }
    }
}
