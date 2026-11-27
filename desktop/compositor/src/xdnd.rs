//! Drag and drop from a Wayland client to an X11 window: the compositor speaks XDND to the X11
//! window on behalf of the Wayland source (lot D-29c).
//!
//! smithay 0.7 has no XDND. The compositor is an X11 client of XWayland of its own (as for the
//! pastes of `x11_paste.rs`), with an invisible window that plays the XDND source: it owns
//! `XdndSelection`, sends `XdndEnter`, `XdndPosition`, `XdndLeave` and `XdndDrop` to the X11
//! window under the pointer, and hands the data to the window it dropped on, read from the
//! Wayland source through a pipe, in a thread. Its connection is a source of the event loop: it
//! never waits on XWayland but for a few short round trips (an atom, a property).
//!
//! Only the X11 client under the pointer may read the drag (the windows of its connection):
//! any other one is refused, like a Wayland client that has no offer. The data are bounded
//! (`LIMIT`), and so is each wait (`PATIENCE`).

use std::{
    fs::File,
    io::{self, Read as _},
    os::fd::{AsFd as _, OwnedFd},
    thread,
    time::Duration,
};

use smithay::reexports::{
    calloop::channel::Sender,
    rustix::{
        self,
        event::{PollFd, PollFlags, Timespec, poll},
    },
};
use x11rb::{
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE,
    connection::{Connection as _, RequestConnection as _},
    errors::{ConnectError, ConnectionError, ReplyError, ReplyOrIdError},
    protocol::{
        Event,
        xproto::{
            Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _,
            CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT,
            SelectionNotifyEvent, Window as XWindow, WindowClass,
        },
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

/// Version of XDND spoken: the last one, which tells whether the drop was accepted.
const VERSION: u32 = 5;
/// Oldest version of the target accepted: the first with `XdndTypeList` and `XdndActionList`.
const OLDEST: u32 = 3;
/// Most bytes handed to an X11 window for one type: a photo, a long list of files.
pub(crate) const LIMIT: usize = 64 << 20;
/// Longest wait for the Wayland source to write the next piece.
const PATIENCE: Duration = Duration::from_secs(5);
/// Size of each piece of an INCR transfer.
const CHUNK: usize = 256 << 10;
/// Longest wait for the `TARGETS` of an X11 drag that has no `XdndTypeList`.
const ASK_PATIENCE: Duration = Duration::from_millis(300);
/// Most types taken from an X11 drag.
const MAX_TYPES: usize = 64;

x11rb::atom_manager! {
    pub(crate) Atoms: AtomsCookie {
        XdndAware,
        XdndProxy,
        XdndEnter,
        XdndPosition,
        XdndStatus,
        XdndLeave,
        XdndDrop,
        XdndFinished,
        XdndSelection,
        XdndTypeList,
        XdndActionCopy,
        XdndActionMove,
        XdndActionLink,
        XdndActionAsk,
        TARGETS,
        INCR,
        UTF8_STRING,
        TEXT,
        STRING,
        _SHERLOCKOS_DND,
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("connection to XWayland: {0}")]
    Connect(#[from] ConnectError),
    #[error("X11: {0}")]
    Connection(#[from] ConnectionError),
    #[error("X11: {0}")]
    Reply(#[from] ReplyError),
    #[error("X11: {0}")]
    ReplyOrId(#[from] ReplyOrIdError),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// What the user asks the drop to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Copy,
    Move,
    Link,
    Ask,
}

/// What the X11 side tells the compositor.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Note {
    /// The window under the pointer accepts the drop, or not, with that action.
    Status {
        accepted: bool,
        action: Option<Action>,
    },
    /// The window dropped on is done: it took the data, or not.
    Finished {
        accepted: bool,
        action: Option<Action>,
    },
    /// The window under the pointer (or dropped on) reads the data as `mime_type`.
    Read(Request),
}

/// A read of the drag by the X11 window: answered by `Xdnd::answer`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    requestor: XWindow,
    property: Atom,
    target: Atom,
    time: u32,
    /// The Wayland type asked for.
    pub mime_type: String,
}

/// The drag over an X11 window.
#[derive(Debug)]
struct Over {
    /// The window under the pointer, named in the messages.
    window: XWindow,
    /// Where the messages go: the window, or its `XdndProxy`.
    to: XWindow,
    version: u32,
    /// A position was sent and its status is awaited: the next one waits (XDND rule).
    waiting: bool,
    /// The last position not sent yet, for that reason.
    next: Option<(i16, i16, Action)>,
    accepted: bool,
    dropped: bool,
}

/// An INCR transfer under way: the requestor deletes each piece to get the next one.
struct Incr {
    requestor: XWindow,
    property: Atom,
    target: Atom,
    data: Vec<u8>,
    sent: usize,
}

/// The XDND source of the compositor.
pub(crate) struct Xdnd {
    conn: RustConnection,
    window: XWindow,
    atoms: Atoms,
    /// Bits of a window identifier that are the X11 client's.
    client_mask: u32,
    /// The Wayland types of the drag, and their atoms.
    types: Vec<(String, Atom)>,
    over: Option<Over>,
    transfers: Vec<Incr>,
    /// Events read while waiting for an answer, handled by `events`.
    backlog: Vec<Event>,
    /// The connection to XWayland still works.
    alive: bool,
}

impl Xdnd {
    /// Connects to the X server `:display` and makes the source window.
    pub(crate) fn connect(display: u32) -> Result<Self, Error> {
        let (conn, screen) = RustConnection::connect(Some(&format!(":{display}")))?;
        let setup = conn.setup();
        let root = setup.roots[screen].root;
        let client_mask = !setup.resource_id_mask;
        let window = conn.generate_id()?;
        let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            COPY_FROM_PARENT,
            &aux,
        )?;
        let atoms = Atoms::new(&conn)?.reply()?;
        conn.flush()?;
        Ok(Self {
            conn,
            window,
            atoms,
            client_mask,
            types: Vec::new(),
            over: None,
            transfers: Vec::new(),
            backlog: Vec::new(),
            alive: true,
        })
    }

    /// A copy of the connection's socket, for the event loop: readable when XWayland speaks.
    pub(crate) fn fd(&self) -> io::Result<OwnedFd> {
        Ok(rustix::io::dup(self.conn.stream())?)
    }

    /// The connection to XWayland still works.
    pub(crate) fn alive(&self) -> bool {
        self.alive
    }

    /// A new drag offers `mime_types`: they become the targets of `XdndSelection`.
    pub(crate) fn start(&mut self, mime_types: &[String]) -> Result<(), Error> {
        self.leave()?;
        self.types.clear();
        for mime in mime_types {
            let atom = self.conn.intern_atom(false, mime.as_bytes())?.reply()?.atom;
            self.types.push((mime.clone(), atom));
        }
        let list: Vec<Atom> = self.types.iter().map(|t| t.1).collect();
        let (window, property) = (self.window, self.atoms.XdndTypeList);
        self.conn
            .change_property32(PropMode::REPLACE, window, property, AtomEnum::ATOM, &list)?;
        let selection = self.atoms.XdndSelection;
        self.conn
            .set_selection_owner(window, selection, CURRENT_TIME)?;
        self.conn.flush()?;
        Ok(())
    }

    /// The drag is over, whatever its end: the selection is released.
    pub(crate) fn end(&mut self) -> Result<(), Error> {
        self.over = None;
        self.types.clear();
        self.transfers.clear();
        let selection = self.atoms.XdndSelection;
        self.conn
            .set_selection_owner(NONE, selection, CURRENT_TIME)?;
        self.conn.flush()?;
        Ok(())
    }

    /// The pointer is at `root` (X11 coordinates) over `window`, or over no X11 window.
    /// `false`: that window does not speak XDND.
    pub(crate) fn hover(
        &mut self,
        window: Option<XWindow>,
        root: (i16, i16),
        action: Action,
    ) -> Result<bool, Error> {
        if self.over.as_ref().map(|o| o.window) != window {
            self.leave()?;
            let Some(window) = window else {
                return Ok(false);
            };
            let Some((to, version)) = self.aware(window)? else {
                tracing::debug!(window, "X11 window without XDND");
                return Ok(false);
            };
            tracing::debug!(window, version, "XDND enter");
            let types: Vec<u32> = self.types.iter().map(|t| t.1).collect();
            let more = u32::from(types.len() > 3);
            let mut data = [self.window, (version << 24) | more, NONE, NONE, NONE];
            for (slot, atom) in data[2..].iter_mut().zip(&types) {
                *slot = *atom;
            }
            self.send(to, window, self.atoms.XdndEnter, data)?;
            self.over = Some(Over {
                window,
                to,
                version,
                waiting: false,
                next: None,
                accepted: false,
                dropped: false,
            });
        }
        if let Some(over) = self.over.as_mut() {
            over.next = Some((root.0, root.1, action));
        }
        self.position()?;
        Ok(true)
    }

    /// Sends the last position, unless a status is awaited.
    fn position(&mut self) -> Result<(), Error> {
        let Some(over) = self.over.as_mut().filter(|o| !o.waiting && !o.dropped) else {
            return Ok(());
        };
        let Some((x, y, action)) = over.next.take() else {
            return Ok(());
        };
        over.waiting = true;
        let (to, window) = (over.to, over.window);
        let at = (u32::from(x as u16) << 16) | u32::from(y as u16);
        let action = self.action_atom(action);
        let data = [self.window, 0, at, CURRENT_TIME, action];
        self.send(to, window, self.atoms.XdndPosition, data)
    }

    /// The pointer left the X11 window, or the drag was cancelled.
    pub(crate) fn leave(&mut self) -> Result<(), Error> {
        if let Some(over) = self.over.take()
            && !over.dropped
        {
            let data = [self.window, 0, 0, 0, 0];
            self.send(over.to, over.window, self.atoms.XdndLeave, data)?;
        }
        Ok(())
    }

    /// Drops on the X11 window under the pointer, if it accepted. `false`: nothing was dropped
    /// (the window has left).
    pub(crate) fn drop_here(&mut self) -> Result<bool, Error> {
        let Some(over) = self.over.as_mut().filter(|o| o.accepted) else {
            self.leave()?;
            return Ok(false);
        };
        over.dropped = true;
        let (to, window) = (over.to, over.window);
        let data = [self.window, 0, CURRENT_TIME, 0, 0];
        self.send(to, window, self.atoms.XdndDrop, data)?;
        Ok(true)
    }

    /// Where the messages for `window` go, and the version spoken, if it speaks XDND.
    fn aware(&mut self, window: XWindow) -> Result<Option<(XWindow, u32)>, Error> {
        let proxy = self
            .conn
            .get_property(false, window, self.atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)?
            .reply()?;
        // A proxy counts only if it names itself (XDND rule).
        let mut to = window;
        if let Some(p) = proxy.value32().and_then(|mut v| v.next()) {
            let own = self
                .conn
                .get_property(false, p, self.atoms.XdndProxy, AtomEnum::WINDOW, 0, 1)
                .map(|c| c.reply());
            if let Ok(Ok(own)) = own
                && own.value32().and_then(|mut v| v.next()) == Some(p)
            {
                to = p;
            }
        }
        let aware = self
            .conn
            .get_property(false, to, self.atoms.XdndAware, AtomEnum::ATOM, 0, 1)?
            .reply()?;
        let theirs = aware.value32().and_then(|mut v| v.next());
        Ok(theirs
            .filter(|v| *v >= OLDEST)
            .map(|v| (to, v.min(VERSION))))
    }

    fn send(&self, to: XWindow, window: XWindow, kind: Atom, data: [u32; 5]) -> Result<(), Error> {
        let message = ClientMessageEvent::new(32, window, kind, data);
        self.conn
            .send_event(false, to, EventMask::NO_EVENT, message)?;
        self.conn.flush()?;
        Ok(())
    }

    fn action_atom(&self, action: Action) -> Atom {
        match action {
            Action::Copy => self.atoms.XdndActionCopy,
            Action::Move => self.atoms.XdndActionMove,
            Action::Link => self.atoms.XdndActionLink,
            Action::Ask => self.atoms.XdndActionAsk,
        }
    }

    fn action_of(&self, atom: Atom) -> Option<Action> {
        let a = &self.atoms;
        [
            (a.XdndActionCopy, Action::Copy),
            (a.XdndActionMove, Action::Move),
            (a.XdndActionLink, Action::Link),
            (a.XdndActionAsk, Action::Ask),
        ]
        .into_iter()
        .find_map(|(known, action)| (known == atom).then_some(action))
    }

    /// The window `requestor` belongs to the X11 client under the pointer.
    fn of_target(&self, requestor: XWindow) -> bool {
        self.over.as_ref().is_some_and(|o| {
            [o.window, o.to]
                .iter()
                .any(|w| (w & self.client_mask) == (requestor & self.client_mask))
        })
    }

    /// The Wayland type that `target` asks for, if the drag offers it.
    fn mime_of(&self, target: Atom) -> Option<String> {
        if let Some((mime, _)) = self.types.iter().find(|t| t.1 == target) {
            return Some(mime.clone());
        }
        let a = &self.atoms;
        if [a.UTF8_STRING, a.TEXT, a.STRING].contains(&target) {
            let text = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];
            return text
                .iter()
                .find(|t| self.types.iter().any(|(m, _)| m == *t))
                .map(|t| t.to_string());
        }
        None
    }

    /// Reads what XWayland sent: the notes for the compositor. The requests the drag cannot
    /// answer are refused here.
    pub(crate) fn events(&mut self) -> Vec<Note> {
        let mut notes = Vec::new();
        for event in std::mem::take(&mut self.backlog) {
            if let Err(err) = self.event(event, &mut notes) {
                tracing::debug!("XDND: {err}");
            }
        }
        loop {
            let event = match self.conn.poll_for_event() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(err) => {
                    tracing::warn!("XDND connection: {err}");
                    self.alive = false;
                    break;
                }
            };
            if let Err(err) = self.event(event, &mut notes) {
                tracing::debug!("XDND: {err}");
            }
        }
        notes
    }

    fn event(&mut self, event: Event, notes: &mut Vec<Note>) -> Result<(), Error> {
        match event {
            Event::ClientMessage(m) if m.format == 32 => {
                let data = m.data.as_data32();
                let from = self.over.as_ref().map(|o| o.window);
                // Only the window under the pointer is listened to.
                if from != Some(data[0]) {
                    return Ok(());
                }
                if m.type_ == self.atoms.XdndStatus {
                    let accepted = data[1] & 1 == 1;
                    let action = self.action_of(data[4]);
                    if let Some(over) = self.over.as_mut() {
                        over.waiting = false;
                        over.accepted = accepted;
                    }
                    notes.push(Note::Status { accepted, action });
                    self.position()?;
                } else if m.type_ == self.atoms.XdndFinished {
                    let version = self.over.as_ref().map_or(VERSION, |o| o.version);
                    // Before version 5, a finished drop says nothing more: it was taken.
                    let accepted = version < 5 || data[1] & 1 == 1;
                    let action = self.action_of(data[2]);
                    notes.push(Note::Finished { accepted, action });
                }
            }
            Event::SelectionRequest(req) if req.selection == self.atoms.XdndSelection => {
                let property = if req.property == NONE {
                    req.target
                } else {
                    req.property
                };
                let answered = if !self.of_target(req.requestor) {
                    tracing::debug!(
                        requestor = req.requestor,
                        "XDND read refused: not the window under the pointer"
                    );
                    false
                } else if req.target == self.atoms.TARGETS {
                    let mut all = vec![self.atoms.TARGETS];
                    all.extend(self.types.iter().map(|t| t.1));
                    self.conn.change_property32(
                        PropMode::REPLACE,
                        req.requestor,
                        property,
                        AtomEnum::ATOM,
                        &all,
                    )?;
                    true
                } else if let Some(mime_type) = self.mime_of(req.target) {
                    notes.push(Note::Read(Request {
                        requestor: req.requestor,
                        property,
                        target: req.target,
                        time: req.time,
                        mime_type,
                    }));
                    return Ok(());
                } else {
                    false
                };
                let property = if answered { property } else { NONE };
                self.notify(req.requestor, req.target, property, req.time)?;
            }
            Event::PropertyNotify(n) if n.state == Property::DELETE => {
                self.next_piece(n.window, n.atom)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn notify(
        &self,
        requestor: XWindow,
        target: Atom,
        property: Atom,
        time: u32,
    ) -> Result<(), Error> {
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time,
            requestor,
            selection: self.atoms.XdndSelection,
            target,
            property,
        };
        self.conn
            .send_event(false, requestor, EventMask::NO_EVENT, notify)?;
        self.conn.flush()?;
        Ok(())
    }

    /// The types of the drag of an X11 application, as Wayland types, if an X11 client owns
    /// `XdndSelection`: from its `XdndTypeList`, else from the selection's `TARGETS` (asked, and
    /// awaited `ASK_PATIENCE` at most). At most `MAX_TYPES`.
    pub(crate) fn foreign_drag(&mut self) -> Result<Option<Vec<String>>, Error> {
        let selection = self.atoms.XdndSelection;
        let owner = self.conn.get_selection_owner(selection)?.reply()?.owner;
        if owner == NONE || owner == self.window {
            return Ok(None);
        }
        let list = self
            .conn
            .get_property(
                false,
                owner,
                self.atoms.XdndTypeList,
                AtomEnum::ATOM,
                0,
                256,
            )?
            .reply()?;
        let mut atoms: Vec<Atom> = list.value32().map(Iterator::collect).unwrap_or_default();
        if atoms.is_empty() {
            atoms = self.ask_targets()?;
        }
        let mut types = Vec::new();
        for atom in atoms.into_iter().take(MAX_TYPES) {
            let name = self.conn.get_atom_name(atom)?.reply()?.name;
            let name = String::from_utf8_lossy(&name).into_owned();
            let mime = match name.as_str() {
                "UTF8_STRING" => "text/plain;charset=utf-8".to_string(),
                n if n.contains('/') => name,
                _ => continue,
            };
            if !types.contains(&mime) {
                types.push(mime);
            }
        }
        Ok(Some(types))
    }

    /// The `TARGETS` of `XdndSelection`, asked to its owner. The other events that come
    /// meanwhile are kept for `events`.
    fn ask_targets(&mut self) -> Result<Vec<Atom>, Error> {
        let (selection, property) = (self.atoms.XdndSelection, self.atoms._SHERLOCKOS_DND);
        self.conn.convert_selection(
            self.window,
            selection,
            self.atoms.TARGETS,
            property,
            CURRENT_TIME,
        )?;
        self.conn.flush()?;
        let deadline = std::time::Instant::now() + ASK_PATIENCE;
        loop {
            match self.conn.poll_for_event()? {
                Some(Event::SelectionNotify(n)) if n.selection == selection => {
                    if n.property == NONE {
                        return Ok(Vec::new());
                    }
                    let reply = self
                        .conn
                        .get_property(true, self.window, property, AtomEnum::ATOM, 0, 256)?
                        .reply()?;
                    let targets = reply.value32().map(Iterator::collect).unwrap_or_default();
                    return Ok(targets);
                }
                Some(other) => self.backlog.push(other),
                None if std::time::Instant::now() > deadline => return Ok(Vec::new()),
                None => thread::sleep(Duration::from_millis(1)),
            }
        }
    }

    /// The X11 application that dragged gets its button released (XTEST, inside XWayland): it
    /// ends its drag.
    pub(crate) fn release_button(&self, button: u8) -> Result<(), Error> {
        use x11rb::protocol::xtest::ConnectionExt as _;
        const BUTTON_RELEASE: u8 = 5;
        self.conn
            .xtest_fake_input(BUTTON_RELEASE, button, CURRENT_TIME, NONE, 0, 0, 0)?;
        self.conn.flush()?;
        Ok(())
    }

    /// Answers a read with `data`, or refuses it (`None`). Large data go by INCR.
    pub(crate) fn answer(&mut self, request: Request, data: Option<Vec<u8>>) -> Result<(), Error> {
        let Request {
            requestor,
            property,
            target,
            time,
            ..
        } = request;
        let Some(data) = data.filter(|_| self.of_target(requestor)) else {
            return self.notify(requestor, target, NONE, time);
        };
        let direct = self
            .conn
            .maximum_request_bytes()
            .saturating_sub(1024)
            .min(CHUNK * 4);
        if data.len() <= direct {
            self.conn
                .change_property8(PropMode::REPLACE, requestor, property, target, &data)?;
        } else {
            let events = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
            self.conn.change_window_attributes(requestor, &events)?;
            let len = [u32::try_from(data.len()).unwrap_or(u32::MAX)];
            let incr = self.atoms.INCR;
            self.conn
                .change_property32(PropMode::REPLACE, requestor, property, incr, &len)?;
            self.transfers
                .retain(|t| (t.requestor, t.property) != (requestor, property));
            self.transfers.push(Incr {
                requestor,
                property,
                target,
                data,
                sent: 0,
            });
        }
        self.notify(requestor, target, property, time)
    }

    /// The requestor took a piece of an INCR transfer: the next one, the empty one at the end.
    fn next_piece(&mut self, requestor: XWindow, property: Atom) -> Result<(), Error> {
        let Some(i) = self
            .transfers
            .iter()
            .position(|t| t.requestor == requestor && t.property == property)
        else {
            return Ok(());
        };
        let t = &mut self.transfers[i];
        let end = (t.sent + CHUNK).min(t.data.len());
        let piece = t.data[t.sent..end].to_vec();
        let target = t.target;
        if piece.is_empty() {
            self.transfers.remove(i);
        } else {
            t.sent = end;
        }
        self.conn
            .change_property8(PropMode::REPLACE, requestor, property, target, &piece)?;
        self.conn.flush()?;
        Ok(())
    }
}

/// Reads at most `LIMIT` bytes from the Wayland source's pipe in a thread, waiting at most
/// `PATIENCE` for each piece, and sends them with `request` (`None` if it failed).
pub(crate) fn read_source(
    request: Request,
    pipe: OwnedFd,
    done: Sender<(Request, Option<Vec<u8>>)>,
) {
    let spawned = thread::Builder::new()
        .name("xdnd-read".into())
        .spawn(move || {
            let data = match read_bounded(pipe) {
                Ok(data) => Some(data),
                Err(err) => {
                    tracing::debug!(mime_type = request.mime_type, "drag not read: {err}");
                    None
                }
            };
            let _ = done.send((request, data));
        });
    if let Err(err) = spawned {
        tracing::warn!("drag not read: {err}");
    }
}

fn read_bounded(pipe: OwnedFd) -> io::Result<Vec<u8>> {
    let timeout = Timespec::try_from(PATIENCE).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut file = File::from(pipe);
    let mut data = Vec::new();
    let mut piece = vec![0; 64 << 10];
    loop {
        let fd = file.as_fd();
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        if poll(&mut fds, Some(&timeout))? == 0 {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let n = file.read(&mut piece)?;
        if n == 0 {
            return Ok(data);
        }
        if data.len() + n > LIMIT {
            return Err(io::Error::other("larger than the limit"));
        }
        data.extend_from_slice(&piece[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pipe_is_read_up_to_its_end_and_never_past_the_limit() {
        let (reader, writer) = std::io::pipe().unwrap();
        let writing = thread::spawn(move || {
            use std::io::Write as _;
            let mut writer = writer;
            writer.write_all(&[7; 300 << 10]).unwrap();
        });
        let data = read_bounded(OwnedFd::from(reader)).unwrap();
        writing.join().unwrap();
        assert_eq!(data.len(), 300 << 10);

        let (reader, writer) = std::io::pipe().unwrap();
        let writing = thread::spawn(move || {
            use std::io::Write as _;
            let mut writer = writer;
            let _ = writer.write_all(&vec![1; LIMIT + 1]);
        });
        assert!(read_bounded(OwnedFd::from(reader)).is_err());
        writing.join().unwrap();
    }
}
