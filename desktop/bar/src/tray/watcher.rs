//! `org.kde.StatusNotifierWatcher` on the session bus (lot D-45): where applications register
//! their item, and hosts (the bar) learn of them. Without it, libappindicator, Electron and Qt
//! fall back on the X11 tray, or show nothing.
//!
//! As the registrar of the global menu, it answers every call at once and asks for its name
//! without queueing and without replacing: on a bus where another watcher answers (Plasma's
//! `kded6`), the bar leaves it alone and registers there as a host instead (`host`).
//!
//! Only `org.kde.StatusNotifierWatcher` is served: libappindicator, libayatana-appindicator,
//! Electron, Qt and KDE all look for it, none for `org.freedesktop.StatusNotifierWatcher`.

use std::sync::{Arc, Mutex, PoisonError};

use iced::futures::StreamExt;
use zbus::Connection;
use zbus::fdo::{self, DBusProxy, RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;

use super::{address, key};

/// The well-known name, and the interface.
pub const NAME: &str = "org.kde.StatusNotifierWatcher";
/// Its object path.
pub const PATH: &str = "/StatusNotifierWatcher";

/// The items registered, by key, and the hosts.
#[derive(Debug, Default)]
struct Registered {
    items: Vec<String>,
    hosts: Vec<String>,
}

struct Watcher {
    registered: Arc<Mutex<Registered>>,
}

impl Watcher {
    fn lock(&self) -> std::sync::MutexGuard<'_, Registered> {
        self.registered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn sender(header: &Header<'_>) -> fdo::Result<String> {
    let sender = header.sender().map(ToString::to_string);
    sender.ok_or_else(|| fdo::Error::Failed(String::from("call without a sender")))
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    /// `service` is a bus name, or an object path on the caller's connection.
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let (service, path) = address(service, &sender(&header)?);
        let key = key(&service, &path);
        let new = {
            let mut registered = self.lock();
            let new = !registered.items.contains(&key);
            if new {
                registered.items.push(key.clone());
            }
            new
        };
        if new {
            tracing::debug!(key, "tray item registered");
            if let Err(error) = Self::status_notifier_item_registered(&emitter, &key).await {
                tracing::debug!(%error, "StatusNotifierItemRegistered not sent");
            }
        }
        Ok(())
    }

    async fn register_status_notifier_host(
        &self,
        service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        {
            let mut registered = self.lock();
            if !registered.hosts.iter().any(|h| h == service) {
                registered.hosts.push(service.to_owned());
            }
        }
        if let Err(error) = Self::status_notifier_host_registered(&emitter).await {
            tracing::debug!(%error, "StatusNotifierHostRegistered not sent");
        }
        Ok(())
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.lock().items.clone()
    }

    /// The bar is a host itself: applications that ask see one at once, and do not fall back on
    /// the X11 tray.
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_unregistered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

/// The bus name of an item's key: what goes away when the application leaves the bus.
fn service_of(key: &str) -> &str {
    key.split_once('/').map_or(key, |(service, _)| service)
}

/// Serves the watcher on `bus` and asks for its name. `Ok(false)` when another watcher owns
/// it: this one then stays out of the way.
pub async fn serve(bus: &Connection) -> zbus::Result<bool> {
    let registered = Arc::new(Mutex::new(Registered::default()));
    let watcher = Watcher {
        registered: Arc::clone(&registered),
    };
    // The object first: a caller that sees the name gets an answer.
    bus.object_server().at(PATH, watcher).await?;
    let flags = RequestNameFlags::DoNotQueue.into();
    match bus.request_name_with_flags(NAME, flags).await {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            bus.object_server().remove::<Watcher, _>(PATH).await?;
            tracing::info!("another watcher owns {NAME}: the bar registers there as a host");
            return Ok(false);
        }
        Err(error) => return Err(error),
    }
    // An application that leaves the bus takes its items with it, and a host its place.
    let changes = DBusProxy::new(bus)
        .await?
        .receive_name_owner_changed()
        .await?;
    let emitter = SignalEmitter::new(bus, PATH)?.into_owned();
    let gone = changes.for_each(move |signal| {
        let (registered, emitter) = (Arc::clone(&registered), emitter.clone());
        async move {
            let Ok(args) = signal.args() else { return };
            if args.new_owner().is_some() {
                return;
            }
            let name = args.name().to_string();
            let (items, host_left) = {
                let mut registered = registered.lock().unwrap_or_else(PoisonError::into_inner);
                let items: Vec<String> = registered
                    .items
                    .iter()
                    .filter(|k| service_of(k) == name)
                    .cloned()
                    .collect();
                registered.items.retain(|k| service_of(k) != name);
                let hosts = registered.hosts.len();
                registered.hosts.retain(|h| *h != name);
                (items, registered.hosts.len() != hosts)
            };
            for item in items {
                tracing::debug!(key = item, "tray item gone");
                let sent = Watcher::status_notifier_item_unregistered(&emitter, &item).await;
                if let Err(error) = sent {
                    tracing::debug!(%error, "StatusNotifierItemUnregistered not sent");
                }
            }
            if host_left {
                let _ = Watcher::status_notifier_host_unregistered(&emitter).await;
            }
        }
    });
    bus.executor().spawn(gone, "tray-watcher-owners").detach();
    tracing::info!("{NAME} served");
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_item_leaves_with_its_bus_name() {
        assert_eq!(service_of(":1.7/org/ayatana/NotificationItem/x"), ":1.7");
        assert_eq!(
            service_of("org.kde.StatusNotifierItem-3-1/StatusNotifierItem"),
            "org.kde.StatusNotifierItem-3-1"
        );
        assert_eq!(service_of(":1.8"), ":1.8");
    }
}
