//! Bluetooth through BlueZ on the system bus: whether the adapter is powered, how many devices
//! are connected, and switching it on or off.

use std::collections::HashMap;

use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::{Problem, Result, set};

pub const SERVICE: &str = "org.bluez";
pub const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";

/// The first Bluetooth adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bluetooth {
    pub adapter: String,
    pub powered: bool,
    /// Devices connected through it.
    pub connected: usize,
}

type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

/// A boolean property, false when it is missing.
fn flag(properties: &HashMap<String, OwnedValue>, name: &str) -> bool {
    properties
        .get(name)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}

/// Reads the first adapter; `Problem::Absent` without one.
pub async fn read(bus: &Connection) -> Result<Bluetooth> {
    let reply = bus
        .call_method(
            Some(SERVICE),
            "/",
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await?;
    let objects: Objects = reply.body().deserialize()?;
    let mut adapters: Vec<(&str, bool)> = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            interfaces
                .get(ADAPTER)
                .map(|adapter| (path.as_str(), flag(adapter, "Powered")))
        })
        .collect();
    // The same adapter every time: the first by path (hci0).
    adapters.sort();
    let Some((adapter, powered)) = adapters.first().copied() else {
        return Err(Problem::Absent(String::from("no Bluetooth adapter")));
    };
    let prefix = format!("{adapter}/");
    let connected = objects
        .iter()
        .filter(|(path, _)| path.as_str().starts_with(&prefix))
        .filter_map(|(_, interfaces)| interfaces.get(DEVICE))
        .filter(|device| flag(device, "Connected"))
        .count();
    Ok(Bluetooth {
        adapter: adapter.to_owned(),
        powered,
        connected,
    })
}

/// Powers the adapter on or off.
pub async fn power(bus: &Connection, adapter: &str, on: bool) -> Result<()> {
    set(bus, SERVICE, adapter, ADAPTER, "Powered", Value::from(on)).await
}
