//! The system services behind the quick settings. Every write goes through D-Bus to the
//! service that owns the setting, which asks polkit where its policy wants it: NetworkManager
//! (Wi-Fi), BlueZ (Bluetooth), logind (brightness), power-profiles-daemon (energy). Sound goes
//! through `pactl`, PipeWire having no D-Bus interface. Nothing runs as root, nothing uses
//! `sudo`.
//!
//! Each call takes the connection it goes through, so that the tests run them on a private
//! bus against fake services.

pub mod audio;
pub mod backlight;
pub mod bluetooth;
pub mod network;
pub mod power;

use std::sync::OnceLock;
use std::time::Duration;

use zbus::Connection;
use zbus::DBusError;
use zbus::zvariant::{OwnedValue, Value};

/// What went wrong with a service.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// The service is not there: not installed, not running, no such device.
    #[error("absent: {0}")]
    Absent(String),
    /// The service, or polkit for it, said no.
    #[error("refused: {0}")]
    Refused(String),
    #[error("failed: {0}")]
    Failed(String),
}

impl From<zbus::Error> for Problem {
    fn from(error: zbus::Error) -> Self {
        let text = error.to_string();
        let name = match &error {
            zbus::Error::MethodError(name, _, _) => name.as_str().to_owned(),
            zbus::Error::FDO(fdo) => fdo.name().to_string(),
            // No bus at all.
            zbus::Error::Address(_) | zbus::Error::InputOutput(_) => {
                return Self::Absent(text);
            }
            _ => String::new(),
        };
        classify(&name, text)
    }
}

impl From<zbus::zvariant::Error> for Problem {
    fn from(error: zbus::zvariant::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

/// The kind of a D-Bus error, from its name.
fn classify(name: &str, text: String) -> Problem {
    const ABSENT: [&str; 5] = [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.UnknownObject",
        "org.freedesktop.DBus.Error.UnknownInterface",
        "org.freedesktop.DBus.Error.UnknownMethod",
    ];
    const REFUSED: [&str; 4] = [
        "AccessDenied",
        "NotAuthorized",
        "PermissionDenied",
        "InteractiveAuthorizationRequired",
    ];
    if ABSENT.contains(&name) {
        Problem::Absent(text)
    } else if REFUSED.iter().any(|r| name.ends_with(r)) {
        Problem::Refused(text)
    } else {
        Problem::Failed(text)
    }
}

pub type Result<T> = std::result::Result<T, Problem>;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_sorted_by_their_name() {
        let absent = classify("org.freedesktop.DBus.Error.ServiceUnknown", String::new());
        assert!(matches!(absent, Problem::Absent(_)));
        for name in [
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.NetworkManager.PermissionDenied",
            "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired",
            "org.bluez.Error.NotAuthorized",
        ] {
            let problem = classify(name, String::new());
            assert!(matches!(problem, Problem::Refused(_)), "{name}");
        }
        let failed = classify("org.bluez.Error.Busy", String::new());
        assert!(matches!(failed, Problem::Failed(_)));
    }
}
