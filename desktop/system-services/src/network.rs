//! Wi-Fi through NetworkManager on the system bus: switch it on and off, the network it is on,
//! the networks around, and joining one, with its password when it has none saved.

use std::collections::HashMap;

use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use crate::bus::{get, set};
use crate::{Problem, Result};

pub const SERVICE: &str = "org.freedesktop.NetworkManager";
pub const PATH: &str = "/org/freedesktop/NetworkManager";
pub const MANAGER: &str = "org.freedesktop.NetworkManager";
pub const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
pub const WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
pub const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";
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
    /// Its access point, which a connection is asked on.
    pub path: String,
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

/// The saved connection of the network `ssid`, if there is one.
pub async fn saved(bus: &Connection, ssid: &str) -> Result<Option<OwnedObjectPath>> {
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
    for connection in connections {
        let reply = bus
            .call_method(
                Some(SERVICE),
                connection.as_str(),
                Some(CONNECTION),
                "GetSettings",
                &(),
            )
            .await?;
        let settings: HashMap<String, HashMap<String, OwnedValue>> = reply.body().deserialize()?;
        let name = settings
            .get("802-11-wireless")
            .and_then(|wireless| wireless.get("ssid"))
            .and_then(|ssid| ssid.try_clone().ok())
            .and_then(|ssid| Vec::<u8>::try_from(ssid).ok());
        if name.as_deref() == Some(ssid.as_bytes()) {
            return Ok(Some(connection));
        }
    }
    Ok(None)
}

fn object(path: &str) -> Result<ObjectPath<'_>> {
    ObjectPath::try_from(path).map_err(|e| Problem::Failed(e.to_string()))
}

/// Joins `network` on `device`: its saved connection when it has one, else a new connection,
/// with `password` for a protected network. NetworkManager keeps the new connection, its
/// password readable by root only, and asks polkit.
pub async fn connect(
    bus: &Connection,
    device: &str,
    network: &Network,
    password: Option<&str>,
) -> Result<()> {
    let device = object(device)?;
    let point = object(&network.path)?;
    if let Some(connection) = saved(bus, &network.ssid).await? {
        bus.call_method(
            Some(SERVICE),
            PATH,
            Some(MANAGER),
            "ActivateConnection",
            &(connection, &device, &point),
        )
        .await?;
        return Ok(());
    }
    let settings = new_connection(network, password)?;
    bus.call_method(
        Some(SERVICE),
        PATH,
        Some(MANAGER),
        "AddAndActivateConnection",
        &(settings, &device, &point),
    )
    .await?;
    Ok(())
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

/// What a network asks to be joined, beyond its name (lot D-26n, Control Room).
#[derive(Clone, PartialEq, Eq)]
pub enum Credentials {
    /// Open: nothing.
    None,
    /// WPA, WPA2 or WPA3 with a password.
    Password(String),
    /// WEP, with its key.
    Wep(String),
    /// WPA2 or WPA3 Enterprise (802.1X): an identity and its password, the inner method, and
    /// the domain the server's certificate must name (eduroam asks for one).
    Enterprise {
        method: Eap,
        identity: String,
        password: String,
        domain: Option<String>,
    },
}

// The secrets never go to the log.
impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::None => "None",
            Self::Password(_) => "Password(…)",
            Self::Wep(_) => "Wep(…)",
            Self::Enterprise { .. } => "Enterprise(…)",
        })
    }
}

/// The outer method of an enterprise network, both with MSCHAPv2 inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eap {
    Peap,
    Ttls,
}

/// The longest name, identity, password or domain taken: NetworkManager's SSID is 32 bytes,
/// the others are bounded so that nothing unbounded goes to the bus.
pub const MAX_SSID: usize = 32;
pub const MAX_FIELD: usize = 256;

/// The settings of a new connection to the network `ssid` with `credentials`; `hidden`: the
/// network does not say its name, the device asks for it.
pub fn connection<'a>(
    ssid: &'a str,
    hidden: bool,
    credentials: &'a Credentials,
) -> Result<Settings<'a>> {
    let bad = |what: &str| Err(Problem::Failed(format!("{what} is empty or too long")));
    if ssid.is_empty() || ssid.len() > MAX_SSID {
        return bad("the network's name");
    }
    let too_long = |text: &str| text.is_empty() || text.len() > MAX_FIELD;
    let mut settings = HashMap::from([
        (
            "connection",
            HashMap::from([
                ("type", Value::from("802-11-wireless")),
                ("id", Value::from(ssid)),
            ]),
        ),
        (
            "802-11-wireless",
            HashMap::from([
                ("ssid", Value::from(ssid.as_bytes())),
                ("mode", Value::from("infrastructure")),
                ("hidden", Value::from(hidden)),
            ]),
        ),
    ]);
    let security = match credentials {
        Credentials::None => return Ok(settings),
        Credentials::Password(password) if too_long(password) => return bad("the password"),
        Credentials::Password(password) => HashMap::from([
            ("key-mgmt", Value::from("wpa-psk")),
            ("psk", Value::from(password.as_str())),
        ]),
        Credentials::Wep(key) if too_long(key) => return bad("the key"),
        Credentials::Wep(key) => HashMap::from([
            ("key-mgmt", Value::from("none")),
            ("auth-alg", Value::from("open")),
            ("wep-key0", Value::from(key.as_str())),
            // A passphrase, hashed to the key by NetworkManager; 1 is a key in hexadecimal.
            (
                "wep-key-type",
                Value::from(if is_hex_key(key) { 1u32 } else { 2u32 }),
            ),
        ]),
        Credentials::Enterprise {
            method,
            identity,
            password,
            domain,
        } => {
            if too_long(identity) || too_long(password) {
                return bad("the identity or the password");
            }
            let eap = match method {
                Eap::Peap => "peap",
                Eap::Ttls => "ttls",
            };
            let mut dot1x = HashMap::from([
                ("eap", Value::from(vec![eap])),
                ("identity", Value::from(identity.as_str())),
                ("password", Value::from(password.as_str())),
                ("phase2-auth", Value::from("mschapv2")),
            ]);
            if let Some(domain) = domain.as_deref().filter(|d| !d.is_empty()) {
                if domain.len() > MAX_FIELD {
                    return bad("the domain");
                }
                dot1x.insert("domain-suffix-match", Value::from(domain));
            }
            settings.insert("802-1x", dot1x);
            HashMap::from([("key-mgmt", Value::from("wpa-eap"))])
        }
    };
    settings.insert("802-11-wireless-security", security);
    Ok(settings)
}

/// A WEP key given in hexadecimal: 10 or 26 digits.
fn is_hex_key(key: &str) -> bool {
    matches!(key.len(), 10 | 26) && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Joins the network `ssid` with `credentials`, on `device`: through `point` (an access point
/// of the scan) or, for a hidden network, by its name. NetworkManager keeps the connection,
/// its secrets readable by root only, and asks polkit.
pub async fn join(
    bus: &Connection,
    device: &str,
    ssid: &str,
    point: Option<&str>,
    credentials: &Credentials,
) -> Result<()> {
    let settings = connection(ssid, point.is_none(), credentials)?;
    let device = object(device)?;
    let point = object(point.unwrap_or("/"))?;
    bus.call_method(
        Some(SERVICE),
        PATH,
        Some(MANAGER),
        "AddAndActivateConnection",
        &(settings, &device, &point),
    )
    .await?;
    Ok(())
}

/// Forgets the network `ssid`: its saved connection is deleted, with its secrets. `false` when
/// none was saved.
pub async fn forget(bus: &Connection, ssid: &str) -> Result<bool> {
    let Some(connection) = saved(bus, ssid).await? else {
        return Ok(false);
    };
    bus.call_method(
        Some(SERVICE),
        connection.as_str(),
        Some(CONNECTION),
        "Delete",
        &(),
    )
    .await?;
    Ok(true)
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
    }

    #[test]
    fn every_kind_of_network_gets_its_settings() {
        let none = Credentials::None;
        let hidden = connection("Cachée", true, &none).unwrap();
        assert_eq!(hidden["802-11-wireless"]["hidden"], Value::from(true));
        assert!(!hidden.contains_key("802-11-wireless-security"));
        let wep = Credentials::Wep(String::from("0123456789"));
        let settings = connection("Vieux", false, &wep).unwrap();
        let security = &settings["802-11-wireless-security"];
        assert_eq!(security["key-mgmt"], Value::from("none"));
        assert_eq!(security["wep-key-type"], Value::from(1u32));
        let eduroam = Credentials::Enterprise {
            method: Eap::Peap,
            identity: String::from("moi@universite.fr"),
            password: String::from("secret"),
            domain: Some(String::from("universite.fr")),
        };
        let settings = connection("eduroam", false, &eduroam).unwrap();
        assert_eq!(
            settings["802-11-wireless-security"]["key-mgmt"],
            Value::from("wpa-eap")
        );
        let dot1x = &settings["802-1x"];
        assert_eq!(dot1x["phase2-auth"], Value::from("mschapv2"));
        assert_eq!(dot1x["domain-suffix-match"], Value::from("universite.fr"));
        // Bounded, and never in the log.
        assert!(connection(&"x".repeat(33), false, &none).is_err());
        assert!(connection("", false, &none).is_err());
        let long = Credentials::Password("p".repeat(MAX_FIELD + 1));
        assert!(connection("Maison", false, &long).is_err());
        assert!(!format!("{eduroam:?}").contains("secret"));
    }
}
