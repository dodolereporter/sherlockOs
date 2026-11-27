//! `com.canonical.AppMenu.Registrar` on the session bus (lot D-36): where Qt applications
//! register the D-Bus menu of each window. Qt's generic theme and qt6ct look for this name when
//! the application starts, and once it has an owner, give every window a `QDBusMenuBar`, hide
//! the menu bar in the window, and call `RegisterWindow` with the window id and the object path
//! of its menu, waiting for the answer.
//!
//! A registrar that owns the name but does not answer made Konsole wait 25 seconds
//! (`docs/compositor.md`, "D-Bus session bus"). This one answers every call at once: it reads
//! the process of the caller from the bus daemon, keeps the registration, and passes it on to
//! the compositor (`register_app_menu` of `sherlock_shell_v1`), which gives the menu to the
//! window. It never waits for the compositor.
//!
//! The name is only asked for without queueing and without replacing: on a bus where another
//! registrar answers (Plasma's `kded6`), the bar leaves it alone. `bar::run` only starts it on
//! our compositor, once the shell protocol at version 3 is bound.

use std::sync::{Arc, Mutex, PoisonError};

use iced::futures::StreamExt;
use zbus::fdo::{self, DBusProxy, RequestNameFlags, RequestNameReply};
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};
use zbus::{Connection, names::BusName};

/// The well-known name of the registrar.
pub const NAME: &str = "com.canonical.AppMenu.Registrar";
/// Its object path.
pub const PATH: &str = "/com/canonical/AppMenu/Registrar";

/// What the registrar passes on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// `RegisterWindow`: the connection `service`, of process `pid`, exports at `path` the menu
    /// of the window `window`.
    Register {
        window: u32,
        pid: u32,
        service: String,
        path: String,
    },
    /// The menu that `service` exported at `path` is gone: `UnregisterWindow`, or the
    /// connection left the bus.
    Unregister { service: String, path: String },
}

/// Where the changes go: the compositor, or a test.
pub type Sink = Arc<dyn Fn(Change) + Send + Sync>;

/// One registered window: its id, the connection and the path of its menu.
type Entry = (u32, String, OwnedObjectPath);

struct Registrar {
    windows: Arc<Mutex<Vec<Entry>>>,
    sink: Sink,
}

fn sender(header: &Header<'_>) -> fdo::Result<String> {
    let sender = header.sender().map(ToString::to_string);
    sender.ok_or_else(|| fdo::Error::Failed(String::from("call without a sender")))
}

#[zbus::interface(name = "com.canonical.AppMenu.Registrar")]
impl Registrar {
    async fn register_window(
        &self,
        window_id: u32,
        menu_object_path: ObjectPath<'_>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] bus: &Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let service = sender(&header)?;
        // From the bus daemon: the caller cannot choose it.
        let name = BusName::try_from(service.as_str()).map_err(zbus::Error::from)?;
        let pid = DBusProxy::new(bus)
            .await?
            .get_connection_unix_process_id(name)
            .await?;
        let path = OwnedObjectPath::from(menu_object_path.clone());
        {
            let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = (window_id, service.clone(), path.clone());
            windows.retain(|e| e != &entry);
            windows.push(entry);
        }
        tracing::debug!(window_id, pid, service, %path, "window menu registered");
        (self.sink)(Change::Register {
            window: window_id,
            pid,
            service: service.clone(),
            path: path.to_string(),
        });
        let emitted = Self::window_registered(&emitter, window_id, &service, path.as_ref()).await;
        if let Err(error) = emitted {
            tracing::debug!(%error, "WindowRegistered not sent");
        }
        Ok(())
    }

    /// Under Wayland, Qt gives every window the same id: when a connection registered
    /// several menus under the id it withdraws, the ones still exported stay. Qt removes the
    /// object of the menu right after its call, before it reads our question.
    async fn unregister_window(
        &self,
        window_id: u32,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] bus: &Connection,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let service = sender(&header)?;
        let candidates: Vec<Entry> = {
            let windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
            let matching = windows
                .iter()
                .filter(|e| e.0 == window_id && e.1 == service);
            matching.cloned().collect()
        };
        if candidates.is_empty() {
            return Ok(());
        }
        let (windows, sink, bus) = (
            Arc::clone(&self.windows),
            Arc::clone(&self.sink),
            bus.clone(),
        );
        let several = candidates.len() > 1;
        let withdraw = async move {
            for entry in candidates {
                if several && exported(&bus, &entry.1, &entry.2).await {
                    continue;
                }
                windows
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .retain(|e| e != &entry);
                tracing::debug!(window_id, service = entry.1, path = %entry.2, "window menu withdrawn");
                sink(Change::Unregister {
                    service: entry.1,
                    path: entry.2.to_string(),
                });
            }
        };
        // The answer goes first: the question about the menus waits for nothing.
        emitter
            .connection()
            .executor()
            .spawn(withdraw, "registrar-withdraw")
            .detach();
        if let Err(error) = Self::window_unregistered(&emitter, window_id).await {
            tracing::debug!(%error, "WindowUnregistered not sent");
        }
        Ok(())
    }

    fn get_menu_for_window(&self, window_id: u32) -> fdo::Result<(String, OwnedObjectPath)> {
        let windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = windows.iter().rev().find(|e| e.0 == window_id);
        let entry = entry.ok_or_else(|| fdo::Error::InvalidArgs(String::from("no menu")))?;
        Ok((entry.1.clone(), entry.2.clone()))
    }

    fn get_menus(&self) -> Vec<Entry> {
        self.windows
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    #[zbus(signal)]
    async fn window_registered(
        emitter: &SignalEmitter<'_>,
        window_id: u32,
        service: &str,
        menu_object_path: ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn window_unregistered(emitter: &SignalEmitter<'_>, window_id: u32) -> zbus::Result<()>;
}

/// Whether `service` still exports a menu at `path`: its introspection names the interface.
async fn exported(bus: &Connection, service: &str, path: &OwnedObjectPath) -> bool {
    let asked = bus
        .call_method(
            Some(service),
            path.as_str(),
            Some("org.freedesktop.DBus.Introspectable"),
            "Introspect",
            &(),
        )
        .await;
    let xml = asked
        .ok()
        .and_then(|reply| reply.body().deserialize::<String>().ok());
    xml.is_some_and(|xml| xml.contains("com.canonical.dbusmenu"))
}

/// Serves the registrar on `bus` and asks for its name, changes going to `sink`. `Ok(false)`
/// when another registrar owns the name: this one then stays out of the way.
pub async fn serve(bus: &Connection, sink: Sink) -> zbus::Result<bool> {
    let windows = Arc::new(Mutex::new(Vec::new()));
    let registrar = Registrar {
        windows: Arc::clone(&windows),
        sink: Arc::clone(&sink),
    };
    // The object first: a caller that sees the name gets an answer.
    bus.object_server().at(PATH, registrar).await?;
    let flags = RequestNameFlags::DoNotQueue.into();
    match bus.request_name_with_flags(NAME, flags).await {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => {}
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            bus.object_server().remove::<Registrar, _>(PATH).await?;
            tracing::info!("another registrar owns {NAME}: no global menu from the bar");
            return Ok(false);
        }
        Err(error) => return Err(error),
    }
    // A connection that leaves the bus takes its menus with it.
    let changes = DBusProxy::new(bus)
        .await?
        .receive_name_owner_changed()
        .await?;
    let gone = changes.for_each(move |signal| {
        let (windows, sink) = (Arc::clone(&windows), Arc::clone(&sink));
        async move {
            let Ok(args) = signal.args() else { return };
            if args.new_owner().is_some() {
                return;
            }
            let service = args.name().to_string();
            let mut windows = windows.lock().unwrap_or_else(PoisonError::into_inner);
            for entry in windows.iter().filter(|e| e.1 == service) {
                sink(Change::Unregister {
                    service: service.clone(),
                    path: entry.2.to_string(),
                });
            }
            windows.retain(|e| e.1 != service);
        }
    });
    bus.executor().spawn(gone, "registrar-owners").detach();
    tracing::info!("{NAME} served");
    Ok(true)
}
