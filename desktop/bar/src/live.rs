//! What the indicators of the bar show besides the `nmcli` source, live on the system bus
//! (UI-01): the strength of the Wi-Fi signal, a connection under way, a network without the
//! Internet, and the Bluetooth radio. Each is read when the bar starts, then again on the
//! signals of its service, coalesced: nothing polls.

use iced::futures::future::{self, BoxFuture};
use iced::futures::stream::{self, BoxStream};
use iced::futures::{FutureExt, StreamExt};
use sherlock_ui::icon::Radio;
use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::system::{self, bluetooth, get, network};

/// `NM_STATE_CONNECTING`.
const CONNECTING: u32 = 40;
/// `NM_CONNECTIVITY_NONE`, `PORTAL`, `LIMITED`: connected, but not to the Internet.
const NO_INTERNET: [u32; 3] = [1, 2, 3];
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const WIRELESS_TYPE: &str = "802-11-wireless";

/// The network as the Wi-Fi icon draws it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Link {
    /// The strength of the access point in use (0 to 100), when the Wi-Fi carries the
    /// connection.
    pub strength: Option<u8>,
    /// NetworkManager is connecting.
    pub connecting: bool,
    /// Connected, but NetworkManager finds no way to the Internet (`Connectivity`).
    pub no_internet: bool,
}

/// The link from NetworkManager's main object, its primary connection and that connection's
/// access point.
pub async fn read_link(bus: &Connection) -> system::Result<Link> {
    let (service, path, manager) = (network::SERVICE, network::PATH, network::MANAGER);
    let state: u32 = get(bus, service, path, manager, "State").await?;
    let connectivity: u32 = get(bus, service, path, manager, "Connectivity").await?;
    let primary: OwnedObjectPath = get(bus, service, path, manager, "PrimaryConnection").await?;
    let mut strength = None;
    if primary.as_str() != "/" {
        let kind: String = get(bus, service, primary.as_str(), ACTIVE, "Type").await?;
        if kind == WIRELESS_TYPE {
            let devices: Vec<OwnedObjectPath> =
                get(bus, service, primary.as_str(), ACTIVE, "Devices").await?;
            if let Some(device) = devices.first() {
                let point: OwnedObjectPath = get(
                    bus,
                    service,
                    device.as_str(),
                    network::WIRELESS,
                    "ActiveAccessPoint",
                )
                .await?;
                if point.as_str() != "/" {
                    strength =
                        Some(get(bus, service, point.as_str(), ACCESS_POINT, "Strength").await?);
                }
            }
        }
    }
    Ok(Link {
        strength,
        connecting: state == CONNECTING,
        no_internet: primary.as_str() != "/" && NO_INTERNET.contains(&connectivity),
    })
}

/// The Bluetooth radio, from BlueZ's first adapter; an error without an adapter.
pub async fn read_radio(bus: &Connection) -> system::Result<Radio> {
    let read = bluetooth::read(bus).await?;
    Ok(radio(read.powered, read.connected))
}

/// The radio of an adapter `powered` with `connected` devices.
pub fn radio(powered: bool, connected: usize) -> Radio {
    match (powered, connected) {
        (false, _) => Radio::Off,
        (true, 0) => Radio::On,
        (true, _) => Radio::Connected,
    }
}

fn properties(sender: &'static str) -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(sender)?
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build())
}

fn objects(member: &'static str) -> zbus::Result<zbus::MatchRule<'static>> {
    Ok(zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(bluetooth::SERVICE)?
        .interface("org.freedesktop.DBus.ObjectManager")?
        .member(member)?
        .build())
}

/// The link, live: at start, then on every `PropertiesChanged` of NetworkManager (its state,
/// its connectivity, the strength of an access point). `None` while it cannot be read.
pub fn link() -> BoxStream<'static, Option<Link>> {
    follow(
        |bus| async move { read_link(&bus).await }.boxed(),
        || Ok(vec![properties(network::SERVICE)?]),
    )
}

/// The Bluetooth radio, live: at start, then on BlueZ's `PropertiesChanged`,
/// `InterfacesAdded` and `InterfacesRemoved`. `None` without BlueZ or an adapter.
pub fn bluetooth() -> BoxStream<'static, Option<Radio>> {
    follow(
        |bus| async move { read_radio(&bus).await }.boxed(),
        || {
            Ok(vec![
                properties(bluetooth::SERVICE)?,
                objects("InterfacesAdded")?,
                objects("InterfacesRemoved")?,
            ])
        },
    )
}

/// Reads with `read` at start and after each batch of the signals `rules` match, on the system
/// bus, and yields what changed only.
fn follow<T>(
    read: fn(Connection) -> BoxFuture<'static, system::Result<T>>,
    rules: fn() -> zbus::Result<Vec<zbus::MatchRule<'static>>>,
) -> BoxStream<'static, Option<T>>
where
    T: Clone + PartialEq + Send + 'static,
{
    stream::once(async move {
        let bus = match system::system().await {
            Ok(bus) => bus,
            Err(problem) => {
                tracing::warn!(%problem, "the indicators will not follow the system bus");
                return stream::empty().boxed();
            }
        };
        let mut signals = Vec::new();
        match rules() {
            Ok(rules) => {
                for rule in rules {
                    match zbus::MessageStream::for_match_rule(rule, &bus, None).await {
                        Ok(messages) => signals.push(messages.map(|_| ()).boxed()),
                        Err(error) => tracing::warn!(%error, "cannot follow a service"),
                    }
                }
            }
            Err(error) => tracing::warn!(%error, "cannot follow a service"),
        }
        let changes =
            crate::bus::coalesce(stream::select_all(signals).boxed(), crate::bus::SIGNAL_GAP);
        stream::once(future::ready(()))
            .chain(changes)
            .then(move |()| read(bus.clone()).map(Result::ok))
            .scan(None, |last: &mut Option<Option<T>>, now| {
                let new = last.as_ref() != Some(&now);
                *last = Some(now.clone());
                future::ready(Some(new.then_some(now)))
            })
            .filter_map(future::ready)
            .boxed()
    })
    .flatten()
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_radio_says_off_on_or_connected() {
        assert_eq!(radio(false, 2), Radio::Off);
        assert_eq!(radio(true, 0), Radio::On);
        assert_eq!(radio(true, 1), Radio::Connected);
    }
}
