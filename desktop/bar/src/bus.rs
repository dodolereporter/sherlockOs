//! The bar's D-Bus connections, opened on first use and then shared: the session bus for the
//! notification daemon. They follow `DBUS_SESSION_BUS_ADDRESS`, which is how the tests point
//! the bar at a private bus.

use std::sync::OnceLock;

use zbus::Connection;

/// The session bus.
pub async fn session() -> zbus::Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    if let Some(bus) = BUS.get() {
        return Ok(bus.clone());
    }
    let bus = Connection::session().await?;
    Ok(BUS.get_or_init(|| bus).clone())
}
