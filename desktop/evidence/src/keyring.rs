//! The passwords of the network's servers in the Secret Service keyring (lot E-07b): GNOME
//! Keyring, KWallet or KeePassXC on the session bus, under the schema gvfs and Nautilus use
//! (`org.gnome.keyring.NetworkPassword`), so that they share them. Without a keyring, or with
//! a locked one, nothing is kept and nothing is found: Evidence asks again. Every call has a
//! time limit and runs off the interface thread.

use std::collections::HashMap;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use crate::smb::Credentials;

const SERVICE: &str = "org.freedesktop.secrets";
const PATH: &str = "/org/freedesktop/secrets";
const INTERFACE: &str = "org.freedesktop.Secret.Service";
const DEFAULT: &str = "/org/freedesktop/secrets/aliases/default";
const SCHEMA: &str = "org.gnome.keyring.NetworkPassword";
/// Longest wait for the keyring.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Longest secret read back.
const MAX_SECRET: usize = 4096;

/// A secret as the Secret Service carries it: session, parameters, value, content type.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// The attributes that find the password of `host`.
fn attributes<'a>(
    host: &'a str,
    credentials: Option<&'a Credentials>,
) -> HashMap<&'a str, &'a str> {
    let mut map = HashMap::from([
        ("xdg:schema", SCHEMA),
        ("protocol", "smb"),
        ("server", host),
    ]);
    if let Some(c) = credentials {
        map.insert("user", c.user.as_str());
        map.insert("domain", c.domain.as_str());
    }
    map
}

/// The keyring service and a plain session (the bus is local to the session).
fn open() -> zbus::Result<(Connection, OwnedObjectPath)> {
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(TIMEOUT)
        .build()?;
    let service = Proxy::new(&connection, SERVICE, PATH, INTERFACE)?;
    let (_, session): (OwnedValue, OwnedObjectPath) =
        service.call("OpenSession", &("plain", Value::from("")))?;
    Ok((connection, session))
}

/// The name and the password kept for `host`, if an unlocked keyring has them.
pub fn find(host: &str) -> Option<Credentials> {
    let found = (|| -> zbus::Result<Option<Credentials>> {
        let (connection, session) = open()?;
        let service = Proxy::new(&connection, SERVICE, PATH, INTERFACE)?;
        let (unlocked, _locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            service.call("SearchItems", &(attributes(host, None),))?;
        let Some(item) = unlocked.into_iter().next() else {
            return Ok(None);
        };
        let item_proxy = Proxy::new(
            &connection,
            SERVICE,
            item.as_str(),
            "org.freedesktop.Secret.Item",
        )?;
        let kept: HashMap<String, String> = item_proxy.get_property("Attributes")?;
        let (_, _, value, _): Secret = item_proxy.call("GetSecret", &(session,))?;
        if value.len() > MAX_SECRET {
            return Ok(None);
        }
        Ok(Some(Credentials {
            user: kept.get("user").cloned().unwrap_or_default(),
            domain: kept.get("domain").cloned().unwrap_or_default(),
            password: String::from_utf8(value).unwrap_or_default(),
        }))
    })();
    found.unwrap_or_else(|error| {
        tracing::info!(%error, "no keyring to read");
        None
    })
}

/// Keeps the name and the password of `host` in the default keyring, replacing the old ones.
/// False when there is no keyring, or it is locked (it would ask through a prompt).
pub fn keep(host: &str, credentials: &Credentials) -> bool {
    let kept = (|| -> zbus::Result<bool> {
        let (connection, session) = open()?;
        let collection = Proxy::new(
            &connection,
            SERVICE,
            DEFAULT,
            "org.freedesktop.Secret.Collection",
        )?;
        let label = format!("smb://{host}/");
        let properties: HashMap<&str, Value<'_>> = HashMap::from([
            (
                "org.freedesktop.Secret.Item.Label",
                Value::from(label.as_str()),
            ),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(attributes(host, Some(credentials))),
            ),
        ]);
        let secret: Secret = (
            session,
            Vec::new(),
            credentials.password.as_bytes().to_vec(),
            "text/plain".to_owned(),
        );
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
            collection.call("CreateItem", &(properties, secret, true))?;
        // A prompt means a locked keyring: nothing was kept.
        Ok(prompt.as_str() == "/")
    })();
    kept.unwrap_or_else(|error| {
        tracing::info!(%error, "no keyring to keep the password");
        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attributes_follow_the_gvfs_schema() {
        let c = Credentials {
            user: "ada".into(),
            domain: "MAISON".into(),
            password: "x".into(),
        };
        let found = attributes("nas.local", None);
        assert_eq!(found.get("xdg:schema"), Some(&SCHEMA));
        assert_eq!(found.get("server"), Some(&"nas.local"));
        assert!(!found.contains_key("user"));
        let kept = attributes("nas.local", Some(&c));
        assert_eq!(
            (kept.get("user"), kept.get("domain")),
            (Some(&"ada"), Some(&"MAISON"))
        );
    }
}
