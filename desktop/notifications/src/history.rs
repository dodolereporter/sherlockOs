//! `org.sherlockos.Notifications.History`: a small private interface of the daemon, for the
//! notification centre of the top bar. It lists the history, empties it, and runs an action of
//! a notification that is still open. `Changed` tells a listener to read the list again. See
//! "History over D-Bus" in `docs/notifications.md`.

use std::sync::Arc;

use zbus::object_server::SignalEmitter;

use crate::core::Core;
use crate::model::{Notification, Urgency};

/// Object path and interface name. The daemon serves them on the same connection as
/// `org.freedesktop.Notifications`, under the same well-known name.
pub const PATH: &str = "/org/sherlockos/Notifications";
pub const INTERFACE: &str = "org.sherlockos.Notifications.History";

/// One notification of the history, as `List` sends it (`(usssa(ss)bxb)`): id, application,
/// title, body as plain text, actions as key and label, whether it is still open, the time it
/// came in seconds since 1970, whether it is critical.
pub type Entry = (
    u32,
    String,
    String,
    String,
    Vec<(String, String)>,
    bool,
    i64,
    bool,
);

/// The history entry of `note`.
pub fn entry(note: &Notification, open: bool) -> Entry {
    let body = note.body.iter().map(|span| span.text.as_str()).collect();
    let received = note
        .received
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    (
        note.id,
        note.app_name.clone(),
        note.summary.clone(),
        body,
        note.actions.clone(),
        open,
        received,
        note.urgency == Urgency::Critical,
    )
}

/// Ids `Remove` reads at most: the whole history.
pub const MAX_REMOVED: usize = sherlock_ui::notifications::HISTORY;

pub(crate) struct History {
    pub(crate) core: Arc<Core>,
}

#[zbus::interface(name = "org.sherlockos.Notifications.History")]
impl History {
    /// The history, newest first.
    async fn list(&self) -> Vec<Entry> {
        self.core.history_entries()
    }

    /// Empties the history (« Tout effacer »). Open notifications stay on screen.
    async fn clear(&self) {
        self.core.clear_history();
    }

    /// Takes the notifications `ids` out of the history (« × » in the centre, D-23d); those
    /// still on screen close, dismissed. At most `MAX_REMOVED` ids are read.
    async fn remove(&self, ids: Vec<u32>) {
        self.core.remove(&ids[..ids.len().min(MAX_REMOVED)]);
    }

    /// Runs the action `key` of the notification `id`, as its button on a bubble would:
    /// `ActionInvoked`, then it closes unless it is resident. An id that is no longer open, or
    /// a key it does not have, is an error.
    async fn invoke_action(&self, id: u32, key: String) -> zbus::fdo::Result<()> {
        if self.core.invoke(id, &key) {
            Ok(())
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "notification {id} is not open or has no action {key}"
            )))
        }
    }

    /// The history changed: a notification came, closed, or the history was emptied.
    #[zbus(signal)]
    pub(crate) async fn changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
