//! The notification centre's side of `org.nomos.Notifications.History`, the private interface
//! of the notification daemon (`docs/notifications.md`): the history, grouped by application,
//! « Tout effacer », the actions of a notification still open, and `Changed`.

use iced::futures::stream::{self, BoxStream};
use iced::futures::{Stream, StreamExt};
use zbus::Connection;

/// Where the daemon serves the interface.
pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/nomos/Notifications";
pub const INTERFACE: &str = "org.nomos.Notifications.History";

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

impl From<Wire> for Entry {
    fn from(wire: Wire) -> Self {
        let (id, app, summary, body, actions, open, received, critical) = wire;
        Self {
            id,
            app,
            summary,
            body,
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
    Ok(wire.into_iter().map(Entry::from).collect())
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
}
