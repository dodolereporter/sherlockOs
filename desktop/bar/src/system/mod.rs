//! The system services behind the quick settings. Every write goes through D-Bus to the
//! service that owns the setting, which asks polkit where its policy wants it: NetworkManager
//! (Wi-Fi), BlueZ (Bluetooth), logind (brightness, in the `system-services` crate),
//! power-profiles-daemon (energy). Sound goes through `pactl`, PipeWire having no D-Bus
//! interface. Nothing runs as root, nothing uses `sudo`.
//!
//! Each call takes the connection it goes through, so that the tests run them on a private
//! bus against fake services.

pub mod bluetooth;
pub mod network;
pub mod power;

// Sound and brightness are shared with the media keys of the compositor (lot D-35).
pub use system_services::{Problem, Result, audio, backlight};

use std::sync::OnceLock;
use std::time::Duration;

use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// A property of an object, read with `org.freedesktop.DBus.Properties.Get`.
pub async fn get<T>(
    bus: &Connection,
    service: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Result<T>
where
    T: TryFrom<OwnedValue>,
    T::Error: std::fmt::Display,
{
    let reply = bus
        .call_method(
            Some(service),
            path,
            Some(PROPERTIES),
            "Get",
            &(interface, name),
        )
        .await?;
    let value: OwnedValue = reply.body().deserialize()?;
    T::try_from(value).map_err(|e| Problem::Failed(format!("{interface}.{name}: {e}")))
}

/// Writes a property with `org.freedesktop.DBus.Properties.Set`.
pub async fn set(
    bus: &Connection,
    service: &str,
    path: &str,
    interface: &str,
    name: &str,
    value: Value<'_>,
) -> Result<()> {
    bus.call_method(
        Some(service),
        path,
        Some(PROPERTIES),
        "Set",
        &(interface, name, value),
    )
    .await?;
    Ok(())
}

/// Longest wait for a read: a service that does not answer leaves its control unavailable,
/// never « … » for ever.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest wait for an action: long enough for polkit to ask the user for a password.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(60);

async fn open(cell: &'static OnceLock<Connection>, timeout: Duration) -> Result<Connection> {
    if let Some(bus) = cell.get() {
        return Ok(bus.clone());
    }
    let bus = zbus::connection::Builder::system()?
        .method_timeout(timeout)
        .build()
        .await?;
    Ok(cell.get_or_init(|| bus).clone())
}

/// The system bus for reads (`READ_TIMEOUT`), opened on first use and shared. It follows
/// `DBUS_SYSTEM_BUS_ADDRESS`.
pub async fn system() -> Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    open(&BUS, READ_TIMEOUT).await
}

/// The system bus for actions (`ACTION_TIMEOUT`), which may wait for polkit.
pub async fn system_for_actions() -> Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    open(&BUS, ACTION_TIMEOUT).await
}
