//! The clipboard history for the Super + V panel (lot D-42): `sherlock_clipboard_history_v1`,
//! offered to the clients of the shell socket only (ADR 0021). The panel lists the items, puts
//! one back on the clipboard (ADR 0030), pins, removes and clears.

use std::{io::Write as _, os::fd::AsFd as _};

use shell_protocol::clipboard::server::sherlock_clipboard_history_v1::{
    self, SherlockClipboardHistoryV1,
};
use smithay::reexports::{
    rustix,
    wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource},
};

use crate::{
    State,
    clipboard_history::{Changes, Entry, History, Kind},
};

const VERSION: u32 = 1;

pub(super) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, SherlockClipboardHistoryV1, ()>(VERSION, ());
}

/// The protocol's kind of an item.
fn wire_kind(kind: Kind) -> sherlock_clipboard_history_v1::Kind {
    use sherlock_clipboard_history_v1::Kind as Wire;
    match kind {
        Kind::Text => Wire::Text,
        Kind::Link => Wire::Link,
        Kind::Files => Wire::Files,
        Kind::Image => Wire::Image,
    }
}

/// Sends `entry`, and its thumbnail in a file of its own.
fn send_item(client: &SherlockClipboardHistoryV1, entry: &Entry) {
    let content = &entry.content;
    client.item(
        entry.id,
        wire_kind(content.kind),
        u32::from(entry.pinned),
        u32::try_from(entry.time).unwrap_or(u32::MAX),
        entry.app.clone(),
        content.preview.clone(),
    );
    let Some(png) = &content.thumbnail else {
        return;
    };
    let file = rustix::fs::memfd_create("clipboard-thumbnail", rustix::fs::MemfdFlags::CLOEXEC)
        .map(std::fs::File::from)
        .map_err(std::io::Error::from)
        .and_then(|mut file| {
            file.write_all(png)?;
            Ok(file)
        });
    match (file, u32::try_from(png.len())) {
        (Ok(file), Ok(size)) => client.thumbnail(entry.id, file.as_fd(), size),
        (Err(err), _) => tracing::debug!("clipboard thumbnail not sent: {err}"),
        (_, Err(_)) => {}
    }
}

/// Everything, to a client that just bound.
fn send_all(client: &SherlockClipboardHistoryV1, enabled: bool, history: &History) {
    client.state(u32::from(enabled));
    for entry in history.entries() {
        send_item(client, entry);
    }
    client.done();
}

impl State {
    /// Tells the panels what changed in the history.
    pub(crate) fn tell_clipboard_clients(&mut self, changes: Changes) {
        let state = self.clipboard_state_changed();
        self.shell.clipboards.retain(Resource::is_alive);
        if changes.updated.is_empty() && changes.removed.is_empty() && state.is_none() {
            return;
        }
        for client in &self.shell.clipboards {
            if let Some(enabled) = state {
                client.state(u32::from(enabled));
            }
            for id in &changes.removed {
                client.removed(*id);
            }
            for id in &changes.updated {
                if let Some(entry) = self.clipboard.history.get(*id) {
                    send_item(client, entry);
                }
            }
            client.done();
        }
    }
}

impl GlobalDispatch<SherlockClipboardHistoryV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<SherlockClipboardHistoryV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let client = data_init.init(resource, ());
        // Earlier changes go to the clients already there first.
        state.clipboard_changed();
        send_all(
            &client,
            state.clipboard_settings().history,
            state.clipboard_history(),
        );
        state.shell.clipboards.push(client);
    }

    fn can_view(client: Client, _: &()) -> bool {
        super::is_shell_client(&client)
    }
}

impl Dispatch<SherlockClipboardHistoryV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &SherlockClipboardHistoryV1,
        request: sherlock_clipboard_history_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use sherlock_clipboard_history_v1::Request;
        match request {
            Request::Destroy => state.shell.clipboards.retain(|c| c != resource),
            Request::Paste { id } => state.put_back(id),
            Request::SetPinned { id, pinned } => state.pin_clipboard_item(id, pinned != 0),
            Request::Remove { id } => state.remove_clipboard_item(id),
            Request::Clear => state.clear_clipboard_history(),
            _ => {}
        }
        state.clipboard_changed();
    }
}
