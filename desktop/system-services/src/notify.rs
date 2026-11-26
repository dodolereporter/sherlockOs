//! A desktop notification (`org.freedesktop.Notifications.Notify`) on the session bus: what
//! the screenshots of the compositor send to confirm one.

use std::collections::HashMap;

use zbus::Connection;
use zbus::zvariant::Value;

use crate::Result;

pub const SERVICE: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";

/// Sends a notification from `app`, shown for the server's default time. Returns its id.
pub async fn send(bus: &Connection, app: &str, summary: &str, body: &str) -> Result<u32> {
    let actions: Vec<&str> = Vec::new();
    let hints: HashMap<&str, Value<'_>> = HashMap::new();
    let reply = bus
        .call_method(
            Some(SERVICE),
            PATH,
            Some(SERVICE),
            "Notify",
            &(app, 0u32, "", summary, body, actions, hints, -1i32),
        )
        .await?;
    Ok(reply.body().deserialize()?)
}
