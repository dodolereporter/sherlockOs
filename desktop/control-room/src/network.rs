//! Wi-Fi through NetworkManager, for the « Réseau » page (lot D-26n): the same calls as the
//! bar's quick settings (`system_services::network`), each on the shared system bus, off the
//! interface thread. Reads go through the bus for reads, actions through the one that waits
//! for polkit.

use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use system_services::bus;
use system_services::network::{self, Credentials, Network, Outcome, Wifi};
use system_services::{Problem, Result};

/// How often the page reads the Wi-Fi again while it is open: the networks around change.
pub const READ_EVERY: Duration = Duration::from_secs(5);

pub async fn read() -> Result<Wifi> {
    let bus = bus::system().await?;
    network::read(&bus).await
}

pub async fn enable(on: bool) -> Result<()> {
    let bus = bus::system_for_actions().await?;
    network::enable(&bus, on).await
}

pub async fn scan(device: String) -> Result<()> {
    let bus = bus::system_for_actions().await?;
    network::scan(&bus, &device).await
}

/// Joins `network` through its saved connection: `false` when it has none, and a form must
/// ask what it needs.
pub async fn rejoin(device: String, network: Network) -> Result<bool> {
    let bus = bus::system_for_actions().await?;
    if network::saved(&bus, &network.ssid).await?.is_none() {
        return Ok(false);
    }
    ended(network::join(&bus, &device, &network, None).await?)?;
    Ok(true)
}

/// What `Problem::Failed` says of a password refused by the network.
pub const WRONG_PASSWORD: &str = "wrong password";

/// The end of an activation (`ACTIVATION_LIMIT` at most) as the page shows it: joined, or a
/// problem; a refused password has its own.
fn ended(outcome: Outcome) -> Result<()> {
    match outcome {
        Outcome::Joined => Ok(()),
        Outcome::WrongPassword => Err(Problem::Failed(String::from(WRONG_PASSWORD))),
        Outcome::Failed(reason) => Err(Problem::Failed(format!("deactivated ({reason})"))),
        Outcome::TimedOut => Err(Problem::Failed(String::from("not joined in time"))),
    }
}

/// Joins the network `ssid` (through `point`, or hidden) with `credentials`.
pub async fn join(
    device: String,
    ssid: String,
    point: Option<String>,
    credentials: Credentials,
) -> Result<()> {
    let bus = bus::system_for_actions().await?;
    ended(network::join_with(&bus, &device, &ssid, point.as_deref(), &credentials).await?)
}

/// Joins a visible network protected by a password: WPA2 or WPA3 as it says.
pub async fn connect(device: String, network: Network, password: String) -> Result<()> {
    let bus = bus::system_for_actions().await?;
    ended(network::join(&bus, &device, &network, Some(&password)).await?)
}

pub async fn forget(ssid: String) -> Result<()> {
    let bus = bus::system_for_actions().await?;
    if network::saved(&bus, &ssid).await?.is_none() {
        return Err(Problem::Absent(ssid));
    }
    network::forget(&bus, &ssid).await
}

/// A unit every `READ_EVERY`, on a thread that ends once nobody listens.
pub fn ticks(_: &()) -> impl Stream<Item = ()> + use<> {
    let (sender, receiver) = mpsc::unbounded();
    let started = std::thread::Builder::new()
        .name("network-ticks".into())
        .spawn(move || {
            while !sender.is_closed() {
                std::thread::sleep(READ_EVERY);
                if sender.unbounded_send(()).is_err() {
                    return;
                }
            }
        });
    if let Err(error) = started {
        tracing::warn!(%error, "the Wi-Fi will not be read again");
    }
    receiver
}
