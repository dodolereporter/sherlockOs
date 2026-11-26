//! The bar's D-Bus connections, opened on first use and then shared: the session bus for the
//! notification daemon. They follow `DBUS_SESSION_BUS_ADDRESS`, which is how the tests point
//! the bar at a private bus.

use std::sync::OnceLock;
use std::time::Duration;

use zbus::Connection;

/// Longest wait for an answer on the session bus: a daemon that hangs gives an error, never a
/// panel that waits for ever.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The session bus.
pub async fn session() -> zbus::Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    if let Some(bus) = BUS.get() {
        return Ok(bus.clone());
    }
    let bus = zbus::connection::Builder::session()?
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    Ok(BUS.get_or_init(|| bus).clone())
}
