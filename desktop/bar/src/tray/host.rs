//! The bar as a `StatusNotifierHost` (lot D-45): it serves the watcher, or registers with the
//! one already on the bus, reads each item registered there, and follows them: the watcher's
//! `StatusNotifierItemRegistered` and `StatusNotifierItemUnregistered`, each item's `NewIcon`,
//! `NewStatus` and the like, and the applications that leave the bus.
//!
//! As for the global menu, every call has the time limit of the menus' connection
//! (`dbusmenu::TIMEOUT`) and runs on the bus's own executor, never on the interface thread: an
//! application that hangs keeps its item unread, never the bar frozen. A burst of signals from
//! one item gives one read (`bus::coalesce`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use iced::futures::channel::mpsc::{self, UnboundedSender};
use iced::futures::stream::{self, BoxStream};
use iced::futures::{FutureExt, StreamExt};
use zbus::fdo::{DBusProxy, PropertiesProxy, RequestNameFlags};
use zbus::names::{BusName, InterfaceName};
use zbus::zvariant::OwnedValue;
use zbus::{Connection, MatchRule, MessageStream};

use super::icon::Finder;
use super::{ITEM, Item, MAX_ITEMS, Properties, address, watcher};
use crate::{bus, dbusmenu};

/// What the bar learns of the notification area.
#[derive(Debug, Clone)]
pub enum Event {
    /// An item appeared or changed: the whole item, read again.
    Changed(Box<Item>),
    /// The item of this key left.
    Gone(String),
}

/// The items known, by key: the unique bus name of the application that owns each.
type Known = Arc<Mutex<HashMap<String, String>>>;

/// The notification area, on the session bus of the menus.
pub fn watch() -> BoxStream<'static, Event> {
    let started = async {
        match dbusmenu::bus().await {
            Ok(bus) => follow(bus).await,
            Err(error) => {
                tracing::warn!(%error, "no session bus: no notification area");
                stream::empty().boxed()
            }
        }
    };
    started.flatten_stream().boxed()
}

/// Follows the notification area on `bus`: serves the watcher, or registers with the one there.
pub async fn follow(bus: Connection) -> BoxStream<'static, Event> {
    match start(bus).await {
        Ok(events) => events,
        Err(error) => {
            tracing::warn!(%error, "the notification area is not followed");
            stream::empty().boxed()
        }
    }
}

async fn start(bus: Connection) -> zbus::Result<BoxStream<'static, Event>> {
    let ours = watcher::serve(&bus).await?;
    let host = format!("org.kde.StatusNotifierHost-{}", std::process::id());
    if let Err(error) = bus
        .request_name_with_flags(host.as_str(), RequestNameFlags::DoNotQueue.into())
        .await
    {
        tracing::debug!(%error, "no host name");
    }
    // The signals first: an item registered meanwhile is not missed.
    let registered = signals(&bus, watcher::NAME, "StatusNotifierItemRegistered").await?;
    let unregistered = signals(&bus, watcher::NAME, "StatusNotifierItemUnregistered").await?;
    let changed = item_signals(&bus).await?;
    let owners = DBusProxy::new(&bus)
        .await?
        .receive_name_owner_changed()
        .await?;
    if !ours {
        bus.call_method(
            Some(watcher::NAME),
            watcher::PATH,
            Some(watcher::NAME),
            "RegisterStatusNotifierHost",
            &host,
        )
        .await?;
    }
    let listed = PropertiesProxy::builder(&bus)
        .destination(watcher::NAME)?
        .path(watcher::PATH)?
        .build()
        .await?
        .get(
            InterfaceName::from_static_str_unchecked(watcher::NAME),
            "RegisteredStatusNotifierItems",
        )
        .await?;
    let listed = Vec::<String>::try_from(listed).unwrap_or_default();
    tracing::info!(items = listed.len(), "notification area followed");

    let (sink, events) = mpsc::unbounded();
    let known: Known = Arc::default();
    let finder = Arc::new(Finder::from_env());
    let context = Context {
        bus: bus.clone(),
        sink,
        known,
        finder,
    };
    // Items to read: those listed, those registered later, those that say they changed.
    let changed = {
        let context = context.clone();
        changed.filter_map(move |(sender, path)| {
            let context = context.clone();
            async move { context.key_of(&sender, &path) }
        })
    };
    let to_read = stream::iter(listed)
        .chain(stream::select(registered, changed.boxed()))
        .boxed();
    let reads = {
        let context = context.clone();
        bus::coalesce(to_read, bus::SIGNAL_GAP).for_each_concurrent(None, move |key| {
            let context = context.clone();
            async move { context.read(key).await }
        })
    };
    let gone = {
        let context = context.clone();
        unregistered.for_each(move |key| {
            context.forget(|k, _| *k == key);
            async {}
        })
    };
    let left = {
        let context = context.clone();
        owners.for_each(move |signal| {
            if let Ok(args) = signal.args()
                && args.new_owner().is_none()
            {
                let name = args.name().to_string();
                context.forget(|key, owner| *owner == name || key.starts_with(&format!("{name}/")));
            }
            async {}
        })
    };
    let executor = bus.executor();
    executor.spawn(reads, "tray-reads").detach();
    executor.spawn(gone, "tray-unregistered").detach();
    executor.spawn(left, "tray-owners").detach();
    Ok(events.boxed())
}

/// What the tasks of the host share.
#[derive(Clone)]
struct Context {
    bus: Connection,
    sink: UnboundedSender<Event>,
    known: Known,
    finder: Arc<Finder>,
}

impl Context {
    fn known(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.known.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The key of the item at `path` of the connection `sender`, when it is known.
    fn key_of(&self, sender: &str, path: &str) -> Option<String> {
        let known = self.known();
        let found = known
            .iter()
            .find(|(key, owner)| *owner == sender && address(key, "").1 == path);
        found.map(|(key, _)| key.clone())
    }

    /// Reads the item of `key` and sends it.
    async fn read(&self, key: String) {
        {
            let known = self.known();
            if known.len() >= MAX_ITEMS && !known.contains_key(&key) {
                tracing::debug!(key, "too many tray items: one more left out");
                return;
            }
        }
        let (service, path) = address(&key, "");
        match self.properties(&service, &path).await {
            Ok((owner, properties)) => {
                let finder = &self.finder;
                let theme_path = properties.theme_path.as_deref();
                let item = Item {
                    key: key.clone(),
                    icon: finder.find(&properties.icon, theme_path),
                    attention: finder.find(&properties.attention, theme_path),
                    overlay: finder.find(&properties.overlay, theme_path),
                    service,
                    path,
                    properties,
                };
                self.known().insert(key, owner);
                let _ = self.sink.unbounded_send(Event::Changed(Box::new(item)));
            }
            Err(error) => tracing::warn!(%error, key, "a tray item does not answer"),
        }
    }

    /// The owner of `service` and the properties of its item at `path`.
    async fn properties(&self, service: &str, path: &str) -> zbus::Result<(String, Properties)> {
        let name = BusName::try_from(service)?;
        let owner = match &name {
            BusName::Unique(unique) => unique.to_string(),
            BusName::WellKnown(_) => DBusProxy::new(&self.bus)
                .await?
                .get_name_owner(name.clone())
                .await?
                .to_string(),
        };
        let values: HashMap<String, OwnedValue> = PropertiesProxy::builder(&self.bus)
            .destination(name)?
            .path(path)?
            .build()
            .await?
            .get_all(InterfaceName::from_static_str_unchecked(ITEM))
            .await?;
        Ok((owner, Properties::parse(&values)))
    }

    /// Forgets the items `gone` picks, and says so.
    fn forget(&self, gone: impl Fn(&String, &String) -> bool) {
        let mut known = self.known();
        let keys: Vec<String> = known
            .iter()
            .filter(|(key, owner)| gone(key, owner))
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            known.remove(&key);
            tracing::debug!(key, "tray item gone");
            let _ = self.sink.unbounded_send(Event::Gone(key));
        }
    }
}

/// The `member` signals of the `interface` of `sender`, their first argument a string.
async fn signals(
    bus: &Connection,
    interface: &'static str,
    member: &'static str,
) -> zbus::Result<BoxStream<'static, String>> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(interface)?
        .member(member)?
        .build();
    let messages = MessageStream::for_match_rule(rule, bus, Some(64)).await?;
    let read =
        messages.filter_map(|message| async move { message.ok()?.body().deserialize().ok() });
    Ok(read.boxed())
}

/// Each item that says it changed (`NewIcon`, `NewStatus`…): its connection and its path.
async fn item_signals(bus: &Connection) -> zbus::Result<BoxStream<'static, (String, String)>> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(ITEM)?
        .build();
    let messages = MessageStream::for_match_rule(rule, bus, Some(256)).await?;
    let changed = messages.filter_map(|message| async move {
        let message = message.ok()?;
        let header = message.header();
        Some((header.sender()?.to_string(), header.path()?.to_string()))
    });
    Ok(changed.boxed())
}
