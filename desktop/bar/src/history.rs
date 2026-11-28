//! The notification centre's side of `org.sherlockos.Notifications.History`, the private interface
//! of the notification daemon (`docs/notifications.md`): the history, grouped by application,
//! « Tout effacer », the actions of a notification still open, and `Changed`.

use iced::futures::stream::{self, BoxStream};
use iced::futures::{Stream, StreamExt};
use sherlock_ui::notifications as ui;
use zbus::Connection;

/// Where the daemon serves the interface.
pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/sherlockos/Notifications";
pub const INTERFACE: &str = "org.sherlockos.Notifications.History";

// What the centre keeps of `List` (Q-38): the daemon's own bounds, here again because any
// program that takes the name after it could send more, and the centre lays it all out.

/// Notifications kept, the newest ones: the daemon's history.
pub const MAX_ENTRIES: usize = ui::HISTORY;
/// Actions kept of one notification, and bytes of an action key (the daemon's bounds).
pub const MAX_ACTIONS: usize = 32;
pub const MAX_KEY_BYTES: usize = 256;

/// What `List` sends for one notification (see the daemon's `history::Entry`).
type Wire = (
    u32,
    String,
    String,
    String,
    Vec<(String, String)>,
    bool,
    i64,
    bool,
);

/// A notification of the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub app: String,
    pub summary: String,
    pub body: String,
    /// Key and label of each action; only an open notification can still run one.
    pub actions: Vec<(String, String)>,
    pub open: bool,
    /// When it came, in seconds since 1970.
    pub received: i64,
    pub critical: bool,
    /// `received` as the centre shows it (`panel::when`), set by the bar.
    pub when: String,
}

/// `text`, its first `max` characters and the « … » the daemon may add.
fn cut(text: String, max: usize) -> String {
    match text.char_indices().nth(max + 1) {
        Some((at, _)) => text[..at].to_owned(),
        None => text,
    }
}

impl From<Wire> for Entry {
    /// Within the daemon's bounds, whoever sent it.
    fn from(wire: Wire) -> Self {
        let (id, app, summary, body, actions, open, received, critical) = wire;
        let actions = actions
            .into_iter()
            .filter(|(key, _)| !key.is_empty() && key.len() <= MAX_KEY_BYTES)
            .take(MAX_ACTIONS)
            .map(|(key, label)| (key, cut(label, ui::MAX_LABEL_CHARS)))
            .collect();
        Self {
            id,
            app: cut(app, ui::MAX_APP_NAME_CHARS),
            summary: cut(summary, ui::MAX_SUMMARY_CHARS),
            body: cut(body, ui::MAX_BODY_CHARS),
            actions,
            open,
            received,
            critical,
            when: String::new(),
        }
    }
}

/// The notifications of one application, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub app: String,
    pub entries: Vec<Entry>,
}

/// Groups `entries` (newest first) by application: the group of the newest notification
/// first, each group in the order its notifications came.
pub fn group(entries: Vec<Entry>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for entry in entries {
        match groups.iter_mut().find(|g| g.app == entry.app) {
            Some(group) => group.entries.push(entry),
            None => groups.push(Group {
                app: entry.app.clone(),
                entries: vec![entry],
            }),
        }
    }
    groups
}

/// The history, newest first.
pub async fn list(bus: &Connection) -> zbus::Result<Vec<Entry>> {
    let reply = bus
        .call_method(Some(NAME), PATH, Some(INTERFACE), "List", &())
        .await?;
    let wire: Vec<Wire> = reply.body().deserialize()?;
    Ok(wire
        .into_iter()
        .take(MAX_ENTRIES)
        .map(Entry::from)
        .collect())
}

/// The key of the action a click on a notification runs.
pub const DEFAULT_ACTION: &str = "default";

/// « × » of a notification or of an application's card: the notifications `ids` leave the
/// history, and close (dismissed) if they are still on screen.
pub async fn remove(bus: &Connection, ids: &[u32]) -> zbus::Result<()> {
    bus.call_method(Some(NAME), PATH, Some(INTERFACE), "Remove", &(ids,))
        .await
        .map(|_| ())
}

/// « Tout effacer ».
pub async fn clear(bus: &Connection) -> zbus::Result<()> {
    bus.call_method(Some(NAME), PATH, Some(INTERFACE), "Clear", &())
        .await
        .map(|_| ())
}

/// Runs the action `key` of the open notification `id`.
pub async fn invoke(bus: &Connection, id: u32, key: &str) -> zbus::Result<()> {
    bus.call_method(
        Some(NAME),
        PATH,
        Some(INTERFACE),
        "InvokeAction",
        &(id, key),
    )
    .await
    .map(|_| ())
}

/// One item per `Changed` of the daemon, on `bus`. Ends when the bus closes.
pub async fn changes(bus: &Connection) -> zbus::Result<impl Stream<Item = ()> + use<>> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        // Only the daemon that owns the name: no other client makes the centre read again.
        .sender(NAME)?
        .interface(INTERFACE)?
        .member("Changed")?
        .path(PATH)?
        .build();
    let messages = zbus::MessageStream::for_match_rule(rule, bus, None).await?;
    Ok(messages.filter_map(|message| async move { message.ok().map(|_| ()) }))
}

/// `changes` on the session bus, for an iced subscription: nothing when the bus cannot be
/// reached (logged once).
pub fn watch() -> BoxStream<'static, ()> {
    stream::once(async {
        let watched = match crate::bus::session().await {
            Ok(bus) => changes(&bus).await,
            Err(error) => Err(error),
        };
        match watched {
            Ok(changes) => crate::bus::coalesce(changes.boxed(), crate::bus::SIGNAL_GAP),
            Err(error) => {
                tracing::warn!(%error, "the notification centre will not follow the history");
                stream::empty().boxed()
            }
        }
    })
    .flatten()
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, app: &str) -> Entry {
        Entry {
            id,
            app: app.to_owned(),
            summary: format!("n{id}"),
            body: String::new(),
            actions: Vec::new(),
            open: false,
            received: 0,
            critical: false,
            when: String::new(),
        }
    }

    #[test]
    fn groups_follow_the_newest_notification_of_each_application() {
        let groups = group(vec![
            entry(5, "Sherlock"),
            entry(4, "Conteneurs"),
            entry(3, "Sherlock"),
            entry(2, "Mises à jour"),
            entry(1, "Conteneurs"),
        ]);
        let apps: Vec<_> = groups.iter().map(|g| g.app.as_str()).collect();
        assert_eq!(apps, ["Sherlock", "Conteneurs", "Mises à jour"]);
        let ids: Vec<_> = groups[1].entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, [4, 1]);
        assert!(group(Vec::new()).is_empty());
    }

    #[test]
    fn an_entry_is_kept_within_the_daemon_bounds() {
        let actions = (0..1000)
            .map(|i| (format!("k{i}"), "Libellé".repeat(100)))
            .chain([
                (String::new(), String::from("vide")),
                ("k".repeat(300), String::new()),
            ])
            .collect();
        let entry = Entry::from((
            1,
            "a".repeat(10_000),
            "é".repeat(10_000),
            "b".repeat(1 << 20),
            actions,
            true,
            0,
            false,
        ));
        assert_eq!(entry.app.chars().count(), ui::MAX_APP_NAME_CHARS + 1);
        assert_eq!(entry.summary.chars().count(), ui::MAX_SUMMARY_CHARS + 1);
        assert_eq!(entry.body.chars().count(), ui::MAX_BODY_CHARS + 1);
        assert_eq!(entry.actions.len(), MAX_ACTIONS);
        assert!(entry.actions.iter().all(|(k, l)| {
            !k.is_empty()
                && k.len() <= MAX_KEY_BYTES
                && l.chars().count() <= ui::MAX_LABEL_CHARS + 1
        }));
        // What is within the bounds stays whole.
        assert_eq!(
            cut(String::from("Sherlock"), ui::MAX_APP_NAME_CHARS),
            "Sherlock"
        );
    }
}
