//! Wi-Fi through NetworkManager on the system bus: switch it on and off, the network it is on,
//! the networks around, joining one (with its password when it has none saved, or a hidden one
//! by its name), following the activation to its end, and forgetting a saved network.

use std::collections::HashMap;
use std::time::Duration;

use iced::futures::StreamExt;
use iced::futures::future::Either;
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::{Problem, Result, get, set};

pub const SERVICE: &str = "org.freedesktop.NetworkManager";
pub const PATH: &str = "/org/freedesktop/NetworkManager";
pub const MANAGER: &str = "org.freedesktop.NetworkManager";
pub const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
pub const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
pub const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
pub const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
/// `NM_ACTIVE_CONNECTION_STATE_ACTIVATED` and `_DEACTIVATED`.
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;
/// `NM_ACTIVE_CONNECTION_STATE_REASON_NO_SECRETS` and `_LOGIN_FAILED`: what a wrong password
/// ends with, no secret agent offering another one.
const NO_SECRETS: u32 = 9;
const LOGIN_FAILED: u32 = 10;
/// Longest wait for an activation, past the authentication and the address (DHCP): « Connexion… »
/// never stays for ever.
pub const ACTIVATION_LIMIT: Duration = Duration::from_secs(60);
/// Saved connections read at most, to find the saved networks.
pub const MAX_SAVED: usize = 64;
/// The access point given for a hidden network: NetworkManager finds it by its name.
pub const ANY_POINT: &str = "/";
/// Longest name of a network, in bytes (IEEE 802.11).
pub const MAX_SSID: usize = 32;
/// `NM_DEVICE_TYPE_WIFI`.
const TYPE_WIFI: u32 = 2;
/// Access point flags: `NM_802_11_AP_FLAGS_PRIVACY`, and the key managements of the WPA and
/// RSN flags: `KEY_MGMT_PSK`, `KEY_MGMT_802_1X`, `KEY_MGMT_SAE`.
const PRIVACY: u32 = 0x1;
const KEY_PSK: u32 = 0x100;
const KEY_8021X: u32 = 0x200;
const KEY_SAE: u32 = 0x400;
/// Networks listed at most, the strongest ones.
pub const MAX_NETWORKS: usize = 12;

/// How a network is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    /// WPA or WPA2 with a password.
    Psk,
    /// WPA3 with a password only.
    Sae,
    /// WEP, or enterprise (802.1X): not joined from the bar.
    Other,
}

/// A network around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    /// Signal, 0 to 100.
    pub strength: u8,
    pub security: Security,
    pub active: bool,
    /// NetworkManager has a connection saved for it: joined without a password, and forgotten.
    pub saved: bool,
    /// Its access point, which a connection is asked on; `ANY_POINT` for a hidden network.
    pub path: String,
}

impl Network {
    /// A hidden network, typed by its name in « Autre réseau… ».
    pub fn hidden(ssid: &str, security: Security) -> Self {
        Self {
            ssid: ssid.to_owned(),
            strength: 0,
            security,
            active: false,
            saved: false,
            path: String::from(ANY_POINT),
        }
    }
}

/// The Wi-Fi as NetworkManager sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wifi {
    pub enabled: bool,
    /// The radio switch or the firmware keeps the Wi-Fi off.
    pub hardware_off: bool,
    /// The first Wi-Fi device; `None` without one.
    pub device: Option<String>,
    /// The one connected at the top, then the strongest first.
    pub networks: Vec<Network>,
}

impl Wifi {
    /// The network the Wi-Fi is connected to.
    pub fn connected(&self) -> Option<&Network> {
        self.networks.iter().find(|n| n.active)
    }
}

/// Reads the Wi-Fi state.
pub async fn read(bus: &Connection) -> Result<Wifi> {
    let enabled: bool = get(bus, SERVICE, PATH, MANAGER, "WirelessEnabled").await?;
    let hardware: bool = get(bus, SERVICE, PATH, MANAGER, "WirelessHardwareEnabled").await?;
    let device = wifi_device(bus).await?;
    let networks = match &device {
        Some(device) if enabled => networks(bus, device).await?,
        _ => Vec::new(),
    };
    Ok(Wifi {
        enabled,
        hardware_off: !hardware,
        device,
        networks,
    })
}

async fn wifi_device(bus: &Connection) -> Result<Option<String>> {
    let reply = bus
        .call_method(Some(SERVICE), PATH, Some(MANAGER), "GetDevices", &())
        .await?;
    let devices: Vec<OwnedObjectPath> = reply.body().deserialize()?;
    for device in devices {
        let kind: u32 = get(bus, SERVICE, device.as_str(), DEVICE, "DeviceType").await?;
        if kind == TYPE_WIFI {
            return Ok(Some(device.to_string()));
        }
    }
    Ok(None)
}

async fn networks(bus: &Connection, device: &str) -> Result<Vec<Network>> {
    // Without the settings service, nothing shows as saved: a network is still joined.
    let saved: Vec<Vec<u8>> = match saved_connections(bus).await {
        Ok(saved) => saved.into_iter().map(|(_, ssid)| ssid).collect(),
        Err(problem) => {
            tracing::debug!(%problem, "the saved networks are not known");
            Vec::new()
        }
    };
    let active: OwnedObjectPath = get(bus, SERVICE, device, WIRELESS, "ActiveAccessPoint").await?;
    let points: Vec<OwnedObjectPath> = get(bus, SERVICE, device, WIRELESS, "AccessPoints").await?;
    let mut found: Vec<Network> = Vec::new();
    for point in points {
        let path = point.as_str();
        let ssid: Vec<u8> = get(bus, SERVICE, path, ACCESS_POINT, "Ssid").await?;
        // A hidden network has no name to show.
        if ssid.is_empty() {
            continue;
        }
        let strength: u8 = get(bus, SERVICE, path, ACCESS_POINT, "Strength").await?;
        let flags: u32 = get(bus, SERVICE, path, ACCESS_POINT, "Flags").await?;
        let wpa: u32 = get(bus, SERVICE, path, ACCESS_POINT, "WpaFlags").await?;
        let rsn: u32 = get(bus, SERVICE, path, ACCESS_POINT, "RsnFlags").await?;
        let network = Network {
            ssid: String::from_utf8_lossy(&ssid).into_owned(),
            strength: strength.min(100),
            security: security(flags, wpa, rsn),
            active: point == active,
            saved: saved.contains(&ssid),
            path: path.to_owned(),
        };
        // One line per name: the access point in use, else the strongest.
        match found.iter_mut().find(|n| n.ssid == network.ssid) {
            Some(known)
                if !known.active && (network.active || network.strength > known.strength) =>
            {
                *known = network;
            }
            Some(_) => {}
            None => found.push(network),
        }
    }
    found.sort_by(|a, b| b.active.cmp(&a.active).then(b.strength.cmp(&a.strength)));
    found.truncate(MAX_NETWORKS);
    Ok(found)
}

/// How an access point is protected, from its flags.
pub fn security(flags: u32, wpa: u32, rsn: u32) -> Security {
    let keys = wpa | rsn;
    if keys & KEY_8021X != 0 {
        Security::Other
    } else if keys & KEY_PSK != 0 {
        Security::Psk
    } else if keys & KEY_SAE != 0 {
        Security::Sae
    } else if flags & PRIVACY != 0 {
        // WEP: privacy without WPA.
        Security::Other
    } else {
        Security::Open
    }
}

/// Switches the Wi-Fi on or off (polkit: `org.freedesktop.NetworkManager.enable-disable-wifi`).
pub async fn enable(bus: &Connection, on: bool) -> Result<()> {
    set(
        bus,
        SERVICE,
        PATH,
        MANAGER,
        "WirelessEnabled",
        Value::from(on),
    )
    .await
}

/// Asks the device for a new scan; the networks change afterwards.
pub async fn scan(bus: &Connection, device: &str) -> Result<()> {
    let options: HashMap<&str, Value<'_>> = HashMap::new();
    bus.call_method(
        Some(SERVICE),
        device,
        Some(WIRELESS),
        "RequestScan",
        &(options,),
    )
    .await?;
    Ok(())
}

/// The saved Wi-Fi connections and the name of their network, `MAX_SAVED` at most.
async fn saved_connections(bus: &Connection) -> Result<Vec<(OwnedObjectPath, Vec<u8>)>> {
    let reply = bus
        .call_method(
            Some(SERVICE),
            SETTINGS_PATH,
            Some(SETTINGS),
            "ListConnections",
            &(),
        )
        .await?;
    let connections: Vec<OwnedObjectPath> = reply.body().deserialize()?;
    let mut found = Vec::new();
    for connection in connections.into_iter().take(MAX_SAVED) {
        let settings = settings_of(bus, &connection).await?;
        let name = settings
            .get("802-11-wireless")
            .and_then(|wireless| wireless.get("ssid"))
            .and_then(|ssid| ssid.try_clone().ok())
            .and_then(|ssid| Vec::<u8>::try_from(ssid).ok());
        if let Some(name) = name {
            found.push((connection, name));
        }
    }
    Ok(found)
}

/// The settings of a saved connection, without its secrets.
type Stored = HashMap<String, HashMap<String, OwnedValue>>;

async fn settings_of(bus: &Connection, connection: &OwnedObjectPath) -> Result<Stored> {
    let reply = bus
        .call_method(
            Some(SERVICE),
            connection.as_str(),
            Some(CONNECTION),
            "GetSettings",
            &(),
        )
        .await?;
    Ok(reply.body().deserialize()?)
}

/// The saved connection of the network `ssid`, if there is one.
pub async fn saved(bus: &Connection, ssid: &str) -> Result<Option<OwnedObjectPath>> {
    let found = saved_connections(bus).await?;
    Ok(found
        .into_iter()
        .find(|(_, name)| name == ssid.as_bytes())
        .map(|(connection, _)| connection))
}

/// Forgets the network `ssid`: every saved connection of it is deleted, which disconnects it
/// when it is in use (polkit: `settings.modify.system`).
pub async fn forget(bus: &Connection, ssid: &str) -> Result<()> {
    for (connection, name) in saved_connections(bus).await? {
        if name == ssid.as_bytes() {
            delete(bus, &connection).await?;
        }
    }
    Ok(())
}

async fn delete(bus: &Connection, connection: &OwnedObjectPath) -> Result<()> {
    bus.call_method(
        Some(SERVICE),
        connection.as_str(),
        Some(CONNECTION),
        "Delete",
        &(),
    )
    .await?;
    Ok(())
}

fn object(path: &str) -> Result<ObjectPath<'_>> {
    ObjectPath::try_from(path).map_err(|e| Problem::Failed(e.to_string()))
}

/// A connection asked of NetworkManager: its activation, which `outcome` follows, and the
/// connection it created, deleted if that first activation fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    pub active: OwnedObjectPath,
    pub created: Option<OwnedObjectPath>,
}

/// Joins `network` on `device`: its saved connection when it has one (its password replaced by
/// `password` when one is given: it was wrong), else a new connection, with `password` for a
/// protected network. NetworkManager keeps the new connection, its password readable by root
/// only, and asks polkit. The answer comes before the network is joined: `outcome` follows it.
pub async fn connect(
    bus: &Connection,
    device: &str,
    network: &Network,
    password: Option<&str>,
) -> Result<Attempt> {
    let device = object(device)?;
    let point = object(&network.path)?;
    if let Some(connection) = saved(bus, &network.ssid).await? {
        if let Some(password) = password.filter(|p| !p.is_empty()) {
            let mut settings = settings_of(bus, &connection).await?;
            let security = settings
                .entry(String::from("802-11-wireless-security"))
                .or_default();
            let management = match network.security {
                Security::Sae => "sae",
                _ => "wpa-psk",
            };
            security
                .entry(String::from("key-mgmt"))
                .or_insert(owned(Value::from(management))?);
            security.insert(String::from("psk"), owned(Value::from(password))?);
            bus.call_method(
                Some(SERVICE),
                connection.as_str(),
                Some(CONNECTION),
                "Update",
                &(settings,),
            )
            .await?;
        }
        let reply = bus
            .call_method(
                Some(SERVICE),
                PATH,
                Some(MANAGER),
                "ActivateConnection",
                &(&connection, &device, &point),
            )
            .await?;
        let active: OwnedObjectPath = reply.body().deserialize()?;
        return Ok(Attempt {
            active,
            created: None,
        });
    }
    let settings = new_connection(network, password)?;
    let reply = bus
        .call_method(
            Some(SERVICE),
            PATH,
            Some(MANAGER),
            "AddAndActivateConnection",
            &(settings, &device, &point),
        )
        .await?;
    let (created, active): (OwnedObjectPath, OwnedObjectPath) = reply.body().deserialize()?;
    Ok(Attempt {
        active,
        created: Some(created),
    })
}

fn owned(value: Value<'_>) -> Result<OwnedValue> {
    value
        .try_into_owned()
        .map_err(|e| Problem::Failed(e.to_string()))
}

/// How an activation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Joined,
    /// The password was refused: it is asked again.
    WrongPassword,
    /// Deactivated for another reason (`NMActiveConnectionStateReason`).
    Failed(u32),
    /// Still not joined after `ACTIVATION_LIMIT`.
    TimedOut,
}

/// How a state of an activation and its reason end it, if they do.
pub fn ending(state: u32, reason: u32) -> Option<Outcome> {
    match (state, reason) {
        (ACTIVATED, _) => Some(Outcome::Joined),
        (DEACTIVATED, NO_SECRETS | LOGIN_FAILED) => Some(Outcome::WrongPassword),
        (DEACTIVATED, reason) => Some(Outcome::Failed(reason)),
        _ => None,
    }
}

/// Follows the activation `active` to its end (`StateChanged` of
/// `org.freedesktop.NetworkManager.Connection.Active`), `limit` at most. An activation gone
/// before its state was read failed.
pub async fn outcome(bus: &Connection, active: &str, limit: Duration) -> Result<Outcome> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SERVICE)?
        .path(active)?
        .interface(ACTIVE)?
        .member("StateChanged")?
        .build();
    // Listened to before the state is read: no change is lost in between.
    let mut changes = zbus::MessageStream::for_match_rule(rule, bus, None).await?;
    match get::<u32>(bus, SERVICE, active, ACTIVE, "State").await {
        Ok(state) => {
            if let Some(end) = ending(state, 0) {
                return Ok(end);
            }
        }
        Err(problem) => {
            tracing::debug!(%problem, "the activation is gone");
            return Ok(Outcome::Failed(0));
        }
    }
    let ended = async {
        while let Some(message) = changes.next().await {
            let Ok(message) = message else { continue };
            let Ok((state, reason)) = message.body().deserialize::<(u32, u32)>() else {
                continue;
            };
            if let Some(end) = ending(state, reason) {
                return end;
            }
        }
        Outcome::Failed(0)
    };
    let timer = crate::bus::pause(limit);
    Ok(
        match iced::futures::future::select(Box::pin(ended), Box::pin(timer)).await {
            Either::Left((end, _)) => end,
            Either::Right(((), _)) => Outcome::TimedOut,
        },
    )
}

/// Joins `network` and follows the activation to its end. A connection created for it that
/// fails is deleted: NetworkManager never keeps a wrong password.
pub async fn join(
    bus: &Connection,
    device: &str,
    network: &Network,
    password: Option<&str>,
) -> Result<Outcome> {
    let attempt = connect(bus, device, network, password).await?;
    let end = outcome(bus, attempt.active.as_str(), ACTIVATION_LIMIT).await?;
    if let (Outcome::WrongPassword | Outcome::Failed(_), Some(created)) = (end, &attempt.created)
        && let Err(problem) = delete(bus, created).await
    {
        tracing::warn!(%problem, "cannot delete the connection that failed");
    }
    Ok(end)
}

/// The settings of a new connection to `network`.
pub type Settings<'a> = HashMap<&'static str, HashMap<&'static str, Value<'a>>>;

/// The settings of a new connection to `network`, with `password` when it is protected.
pub fn new_connection<'a>(network: &'a Network, password: Option<&'a str>) -> Result<Settings<'a>> {
    let mut settings = HashMap::from([
        (
            "connection",
            HashMap::from([
                ("type", Value::from("802-11-wireless")),
                ("id", Value::from(network.ssid.as_str())),
            ]),
        ),
        (
            "802-11-wireless",
            HashMap::from([
                ("ssid", Value::from(network.ssid.as_bytes())),
                ("mode", Value::from("infrastructure")),
            ]),
        ),
    ]);
    if network.ssid.is_empty() || network.ssid.len() > MAX_SSID {
        return Err(Problem::Failed(String::from(
            "a network name has 1 to 32 bytes",
        )));
    }
    if network.path == ANY_POINT
        && let Some(wireless) = settings.get_mut("802-11-wireless")
    {
        wireless.insert("hidden", Value::from(true));
    }
    let management = match network.security {
        Security::Open => return Ok(settings),
        Security::Psk => "wpa-psk",
        Security::Sae => "sae",
        Security::Other => {
            return Err(Problem::Failed(String::from(
                "this network is joined from the settings only",
            )));
        }
    };
    let password = password
        .filter(|p| !p.is_empty())
        .ok_or_else(|| Problem::Failed(String::from("a protected network needs a password")))?;
    settings.insert(
        "802-11-wireless-security",
        HashMap::from([
            ("key-mgmt", Value::from(management)),
            ("psk", Value::from(password)),
        ]),
    );
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_security_of_a_network_comes_from_its_flags() {
        assert_eq!(security(0, 0, 0), Security::Open);
        assert_eq!(security(PRIVACY, 0, KEY_PSK), Security::Psk);
        assert_eq!(security(PRIVACY, KEY_PSK, 0), Security::Psk);
        assert_eq!(security(PRIVACY, 0, KEY_SAE), Security::Sae);
        assert_eq!(security(PRIVACY, 0, KEY_SAE | KEY_PSK), Security::Psk);
        assert_eq!(security(PRIVACY, 0, KEY_8021X), Security::Other);
        assert_eq!(security(PRIVACY, 0, 0), Security::Other);
    }

    #[test]
    fn a_protected_network_needs_its_password() {
        let network = |security| Network {
            ssid: String::from("Maison"),
            strength: 70,
            security,
            active: false,
            saved: false,
            path: String::from("/ap/1"),
        };
        let open = network(Security::Open);
        let settings = new_connection(&open, None).unwrap();
        assert!(!settings.contains_key("802-11-wireless-security"));
        let psk = network(Security::Psk);
        assert!(new_connection(&psk, None).is_err());
        assert!(new_connection(&psk, Some("")).is_err());
        let settings = new_connection(&psk, Some("secret")).unwrap();
        let security = &settings["802-11-wireless-security"];
        assert_eq!(security["key-mgmt"], Value::from("wpa-psk"));
        assert!(new_connection(&network(Security::Other), Some("x")).is_err());
        // A hidden network is asked by its name; a name too long is refused.
        let hidden = Network::hidden("Cachette", Security::Open);
        let settings = new_connection(&hidden, None).unwrap();
        assert_eq!(settings["802-11-wireless"]["hidden"], Value::from(true));
        assert!(!new_connection(&open, None).unwrap()["802-11-wireless"].contains_key("hidden"));
        let long = Network::hidden(&"x".repeat(33), Security::Open);
        assert!(new_connection(&long, None).is_err());
    }

    #[test]
    fn an_activation_ends_joined_refused_or_failed() {
        assert_eq!(ending(1, 0), None);
        assert_eq!(ending(ACTIVATED, 0), Some(Outcome::Joined));
        assert_eq!(
            ending(DEACTIVATED, NO_SECRETS),
            Some(Outcome::WrongPassword)
        );
        assert_eq!(
            ending(DEACTIVATED, LOGIN_FAILED),
            Some(Outcome::WrongPassword)
        );
        assert_eq!(ending(DEACTIVATED, 6), Some(Outcome::Failed(6)));
    }
}
