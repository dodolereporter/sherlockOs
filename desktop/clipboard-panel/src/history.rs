//! The compositor's clipboard history, read through `sherlock_clipboard_history_v1` on the shell
//! socket (ADR 0021): a thread waits for its events and hands the panel each consistent state;
//! the requests (put back, pin, remove, clear) go out from the interface's thread without
//! waiting for anything.

use std::collections::BTreeMap;
use std::fs::File;
use std::os::unix::fs::FileExt as _;
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, MutexGuard, PoisonError};

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use shell_protocol::clipboard::client::sherlock_clipboard_history_v1::{
    self as protocol, SherlockClipboardHistoryV1,
};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};

use crate::items::{self, Item, Kind};

/// Largest thumbnail read: the compositor sends 160 pixels a side, a few tens of KiB.
pub const MAX_THUMBNAIL: u32 = 1 << 20;

/// What the panel learns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The history, in the panel's order, and whether it is on.
    State { enabled: bool, items: Vec<Item> },
    /// No history in this session: no shell socket, or a compositor without the protocol.
    Unavailable,
}

/// The requests of the panel.
#[derive(Clone)]
pub struct Control {
    connection: Connection,
    history: SherlockClipboardHistoryV1,
}

impl Control {
    fn flush(&self) {
        if let Err(error) = self.connection.flush() {
            tracing::warn!(%error, "cannot send to the compositor");
        }
    }

    /// Puts item `id` back on the clipboard (ADR 0029).
    pub fn paste(&self, id: u32) {
        self.history.paste(id);
        self.flush();
    }

    pub fn set_pinned(&self, id: u32, pinned: bool) {
        self.history.set_pinned(id, u32::from(pinned));
        self.flush();
    }

    pub fn remove(&self, id: u32) {
        self.history.remove(id);
        self.flush();
    }

    pub fn clear(&self) {
        self.history.clear();
        self.flush();
    }
}

static CONTROL: Mutex<Option<Control>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The requests, once connected.
pub fn control() -> Option<Control> {
    lock(&CONTROL).clone()
}

/// What the thread gathers between two `done`.
#[derive(Default)]
struct Gathered {
    enabled: bool,
    items: BTreeMap<u32, Item>,
    done: bool,
}

/// The events of the history, from a thread of their own, for as long as the connection lasts.
pub fn events() -> impl Stream<Item = Event> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("clipboard-history".into())
        .spawn(move || {
            if let Err(error) = watch(&sender) {
                tracing::warn!(%error, "no clipboard history");
                let _ = sender.unbounded_send(Event::Unavailable);
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "no thread for the clipboard history");
    }
    receiver
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("no shell socket in this session")]
    NoSocket,
    #[error("shell socket: {0}")]
    Socket(#[from] std::io::Error),
    #[error("Wayland: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("Wayland: {0}")]
    Globals(#[from] wayland_client::globals::GlobalError),
    #[error("the compositor has no clipboard history: {0}")]
    Bind(#[from] wayland_client::globals::BindError),
    #[error("Wayland: {0}")]
    Dispatch(#[from] wayland_client::DispatchError),
}

fn watch(sender: &mpsc::UnboundedSender<Event>) -> Result<(), Error> {
    let path = shell_protocol::socket_from_env().ok_or(Error::NoSocket)?;
    let connection = Connection::from_socket(UnixStream::connect(path)?)?;
    let (globals, mut queue) = registry_queue_init::<Gathered>(&connection)?;
    let history: SherlockClipboardHistoryV1 = globals.bind(&queue.handle(), 1..=1, ())?;
    *lock(&CONTROL) = Some(Control {
        connection,
        history,
    });
    let mut gathered = Gathered::default();
    loop {
        queue.blocking_dispatch(&mut gathered)?;
        if !std::mem::take(&mut gathered.done) {
            continue;
        }
        let mut items: Vec<Item> = gathered.items.values().cloned().collect();
        items::sort(&mut items);
        let event = Event::State {
            enabled: gathered.enabled,
            items,
        };
        if sender.unbounded_send(event).is_err() {
            // The panel is gone.
            return Ok(());
        }
    }
}

/// The PNG of `size` bytes at the start of `file`, if it is not too large.
fn read_thumbnail(file: &File, size: u32) -> Option<Vec<u8>> {
    if size == 0 || size > MAX_THUMBNAIL {
        return None;
    }
    let mut png = vec![0; size as usize];
    file.read_exact_at(&mut png, 0).ok()?;
    Some(png)
}

impl Dispatch<SherlockClipboardHistoryV1, ()> for Gathered {
    fn event(
        gathered: &mut Self,
        _: &SherlockClipboardHistoryV1,
        event: protocol::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            protocol::Event::State { enabled } => gathered.enabled = enabled != 0,
            protocol::Event::Item {
                id,
                kind,
                pinned,
                time,
                app,
                preview,
            } => {
                let thumbnail = gathered.items.remove(&id).and_then(|i| i.thumbnail);
                let mut item = Item::new(
                    id,
                    Kind::from_wire(match kind {
                        WEnum::Value(kind) => kind as u32,
                        WEnum::Unknown(value) => value,
                    }),
                    pinned != 0,
                    u64::from(time),
                    app,
                    preview,
                );
                item.thumbnail = thumbnail;
                gathered.items.insert(id, item);
            }
            protocol::Event::Thumbnail { id, fd, size } => {
                let png = read_thumbnail(&File::from(fd), size);
                if let Some(item) = gathered.items.get_mut(&id) {
                    item.thumbnail = png;
                }
            }
            protocol::Event::Removed { id } => {
                gathered.items.remove(&id);
            }
            protocol::Event::Done => gathered.done = true,
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Gathered {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn a_thumbnail_too_large_or_short_is_not_read() {
        let mut file = tempfile();
        file.write_all(b"png").unwrap();
        assert_eq!(read_thumbnail(&file, 3).as_deref(), Some(&b"png"[..]));
        assert_eq!(read_thumbnail(&file, 4), None);
        assert_eq!(read_thumbnail(&file, 0), None);
        assert_eq!(read_thumbnail(&file, MAX_THUMBNAIL + 1), None);
    }

    fn tempfile() -> File {
        let path = std::env::temp_dir().join(format!("clipboard-panel-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(&path).unwrap();
        file
    }
}
