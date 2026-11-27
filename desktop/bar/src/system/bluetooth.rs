//! Bluetooth through BlueZ on the system bus: the adapter and whether it is powered, its
//! devices (paired, and found by a scan), connecting, disconnecting, pairing with an agent that
//! asks the user, and forgetting a device.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use iced::futures::channel::{mpsc, oneshot};
use iced::futures::stream::{self, BoxStream};
use iced::futures::{FutureExt, StreamExt};
use zbus::Connection;
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use super::{Problem, Result, set};

pub const SERVICE: &str = "org.bluez";
/// Where BlueZ's adapters and devices live.
pub const PATH: &str = "/org/bluez";
pub const ADAPTER: &str = "org.bluez.Adapter1";
pub const DEVICE: &str = "org.bluez.Device1";
const BATTERY: &str = "org.bluez.Battery1";
const AGENT_MANAGER: &str = "org.bluez.AgentManager1";
/// Where the bar serves its agent, only while it pairs a device.
pub const AGENT_PATH: &str = "/org/sherlockos/bar/agent";
/// What the agent can do: show a code, ask for a yes or a no.
const CAPABILITY: &str = "KeyboardDisplay";
/// Devices found by a scan listed at most, besides the paired ones.
pub const MAX_FOUND: usize = 12;

/// What a device is, from its BlueZ `Icon` (a freedesktop icon name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Audio,
    Mouse,
    Keyboard,
    Phone,
    Computer,
    Gamepad,
    Other,
}

impl Kind {
    pub fn of(icon: &str) -> Self {
        match icon {
            "audio-headset" | "audio-headphones" | "audio-card" | "multimedia-player" => {
                Self::Audio
            }
            "input-mouse" | "input-tablet" => Self::Mouse,
            "input-keyboard" => Self::Keyboard,
            "phone" => Self::Phone,
            "computer" => Self::Computer,
            "input-gaming" => Self::Gamepad,
            _ => Self::Other,
        }
    }
}

/// A device of the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub path: String,
    pub name: String,
    pub kind: Kind,
    pub paired: bool,
    pub connected: bool,
    /// Its battery, 0 to 100, when it says (`org.bluez.Battery1`).
    pub battery: Option<u8>,
}

/// The first Bluetooth adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bluetooth {
    pub adapter: String,
    pub powered: bool,
    /// Devices connected through it.
    pub connected: usize,
    /// The paired devices, connected ones first, then the named ones a scan found.
    pub devices: Vec<Device>,
}

type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;
type Objects = HashMap<OwnedObjectPath, Interfaces>;

fn property<T: TryFrom<OwnedValue>>(
    properties: &HashMap<String, OwnedValue>,
    name: &str,
) -> Option<T> {
    properties
        .get(name)
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| T::try_from(v).ok())
}

/// A boolean property, false when it is missing.
fn flag(properties: &HashMap<String, OwnedValue>, name: &str) -> bool {
    property(properties, name).unwrap_or(false)
}

/// A device of the objects, if it has a name to show: a device found without one would only
/// show its address.
fn device(path: &str, interfaces: &Interfaces) -> Option<Device> {
    let properties = interfaces.get(DEVICE)?;
    let paired = flag(properties, "Paired");
    let named: Option<String> = property(properties, "Name");
    let name = match named {
        Some(name) => name,
        None if paired => property(properties, "Alias")?,
        None => return None,
    };
    let icon: Option<String> = property(properties, "Icon");
    let battery = interfaces
        .get(BATTERY)
        .and_then(|b| property::<u8>(b, "Percentage"))
        .map(|p| p.min(100));
    Some(Device {
        path: path.to_owned(),
        // Chosen by whoever owns the device, near: cleaned before it is shown.
        name: system_services::names::shown(&name),
        kind: Kind::of(icon.as_deref().unwrap_or_default()),
        paired,
        connected: flag(properties, "Connected"),
        battery,
    })
}

/// Reads the first adapter and its devices; `Problem::Absent` without one.
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
    let mut devices: Vec<Device> = objects
        .iter()
        .filter(|(path, _)| path.as_str().starts_with(&prefix))
        .filter_map(|(path, interfaces)| device(path.as_str(), interfaces))
        .collect();
    devices.sort_by(|a, b| {
        (b.paired, b.connected)
            .cmp(&(a.paired, a.connected))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let paired = devices.iter().filter(|d| d.paired).count();
    devices.truncate(paired + MAX_FOUND);
    Ok(Bluetooth {
        adapter: adapter.to_owned(),
        powered,
        connected: devices.iter().filter(|d| d.connected).count(),
        devices,
    })
}

/// Powers the adapter on or off.
pub async fn power(bus: &Connection, adapter: &str, on: bool) -> Result<()> {
    set(bus, SERVICE, adapter, ADAPTER, "Powered", Value::from(on)).await
}

async fn call(bus: &Connection, path: &str, interface: &str, method: &str) -> Result<()> {
    bus.call_method(Some(SERVICE), path, Some(interface), method, &())
        .await?;
    Ok(())
}

/// Looks for devices around (`on`), or stops looking.
pub async fn discover(bus: &Connection, adapter: &str, on: bool) -> Result<()> {
    let method = if on {
        "StartDiscovery"
    } else {
        "StopDiscovery"
    };
    call(bus, adapter, ADAPTER, method).await
}

pub async fn connect(bus: &Connection, device: &str) -> Result<()> {
    call(bus, device, DEVICE, "Connect").await
}

pub async fn disconnect(bus: &Connection, device: &str) -> Result<()> {
    call(bus, device, DEVICE, "Disconnect").await
}

/// Stops a pairing under way.
pub async fn cancel_pairing(bus: &Connection, device: &str) -> Result<()> {
    call(bus, device, DEVICE, "CancelPairing").await
}

/// Forgets a paired device: BlueZ removes it and its keys.
pub async fn forget(bus: &Connection, adapter: &str, device: &str) -> Result<()> {
    let device = ObjectPath::try_from(device).map_err(|e| Problem::Failed(e.to_string()))?;
    bus.call_method(
        Some(SERVICE),
        adapter,
        Some(ADAPTER),
        "RemoveDevice",
        &(device,),
    )
    .await?;
    Ok(())
}

/// What the agent asks the user while a device pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// Both sides show this code: « Confirmer » if they match.
    Confirm(String),
    /// The code to type on the device, then « Confirmer ».
    Type(String),
    /// The code the device asks to be typed on it; nothing to answer.
    Show(String),
}

/// The answer to a question of the agent, once: `true` for « Confirmer ». Dropped unanswered,
/// it cancels.
#[derive(Clone, Default)]
pub struct Reply(Arc<Mutex<Option<oneshot::Sender<bool>>>>);

impl Reply {
    pub fn new(sender: oneshot::Sender<bool>) -> Self {
        Self(Arc::new(Mutex::new(Some(sender))))
    }

    pub fn send(&self, yes: bool) {
        let sender = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(sender) = sender {
            let _ = sender.send(yes);
        }
    }
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Reply")
    }
}

/// What happens while a device pairs.
#[derive(Debug, Clone)]
pub enum Pairing {
    Ask(Asked, Reply),
    /// BlueZ cancelled its question.
    Canceled,
    Done(Result<()>),
}

/// The errors BlueZ expects from an agent.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum Refusal {
    Rejected,
    Canceled,
}

/// The agent of the bar: each question goes to the panel, and waits for its answer there. It
/// answers BlueZ alone (`bluez`, the unique name that owned `org.bluez` when the agent was
/// registered), and about the device being paired alone; anything else is rejected.
struct Agent {
    events: mpsc::UnboundedSender<Pairing>,
    bluez: String,
    device: String,
}

impl Agent {
    fn sent_by_bluez(&self, header: &Header<'_>) -> std::result::Result<(), Refusal> {
        match header.sender() {
            Some(sender) if sender.as_str() == self.bluez => Ok(()),
            sender => {
                tracing::warn!(
                    ?sender,
                    "a call to the Bluetooth agent not from BlueZ, rejected"
                );
                Err(Refusal::Rejected)
            }
        }
    }

    /// From BlueZ, about the device being paired.
    fn about(
        &self,
        header: &Header<'_>,
        device: &OwnedObjectPath,
    ) -> std::result::Result<(), Refusal> {
        self.sent_by_bluez(header)?;
        if device.as_str() == self.device {
            Ok(())
        } else {
            tracing::warn!(%device, "the Bluetooth agent asked about another device, rejected");
            Err(Refusal::Rejected)
        }
    }

    async fn ask(&self, asked: Asked) -> std::result::Result<(), Refusal> {
        let (sender, answer) = oneshot::channel();
        self.events
            .unbounded_send(Pairing::Ask(asked, Reply::new(sender)))
            .map_err(|_| Refusal::Canceled)?;
        match answer.await {
            Ok(true) => Ok(()),
            Ok(false) => Err(Refusal::Rejected),
            Err(_) => Err(Refusal::Canceled),
        }
    }

    fn show(&self, code: String) {
        let _ = self
            .events
            .unbounded_send(Pairing::Ask(Asked::Show(code), Reply::default()));
    }
}

/// A code of six digits, from the kernel's random numbers.
fn fresh_code() -> u32 {
    use std::io::Read as _;
    let mut bytes = [0u8; 4];
    let read = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if read.is_err() {
        tracing::warn!("no random numbers: the pairing code is drawn from the clock");
        bytes = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or_default()
            .to_le_bytes();
    }
    u32::from_le_bytes(bytes) % 1_000_000
}

fn code(passkey: u32) -> String {
    format!("{passkey:06}")
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self, #[zbus(header)] header: Header<'_>) -> std::result::Result<(), Refusal> {
        self.sent_by_bluez(&header)
    }

    async fn request_pin_code(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<String, Refusal> {
        self.about(&header, &device)?;
        let pin = code(fresh_code());
        self.ask(Asked::Type(pin.clone())).await?;
        Ok(pin)
    }

    fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<(), Refusal> {
        self.about(&header, &device)?;
        self.show(pincode);
        Ok(())
    }

    async fn request_passkey(
        &self,
        device: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<u32, Refusal> {
        self.about(&header, &device)?;
        let passkey = fresh_code();
        self.ask(Asked::Type(code(passkey))).await?;
        Ok(passkey)
    }

    fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        _entered: u16,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<(), Refusal> {
        self.about(&header, &device)?;
        self.show(code(passkey));
        Ok(())
    }

    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<(), Refusal> {
        self.about(&header, &device)?;
        self.ask(Asked::Confirm(code(passkey))).await
    }

    /// A pairing without a code that BlueZ asks the agent to allow: refused. A « Just Works »
    /// pairing started by `Pair` does not ask.
    fn request_authorization(
        &self,
        _device: OwnedObjectPath,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<(), Refusal> {
        self.sent_by_bluez(&header)?;
        Err(Refusal::Rejected)
    }

    /// A service of a device that asks to connect: refused, always.
    fn authorize_service(
        &self,
        _device: OwnedObjectPath,
        _uuid: String,
        #[zbus(header)] header: Header<'_>,
    ) -> std::result::Result<(), Refusal> {
        self.sent_by_bluez(&header)?;
        Err(Refusal::Rejected)
    }

    fn cancel(&self, #[zbus(header)] header: Header<'_>) -> std::result::Result<(), Refusal> {
        self.sent_by_bluez(&header)?;
        let _ = self.events.unbounded_send(Pairing::Canceled);
        Ok(())
    }
}

/// Pairs `device`, then trusts it and connects it. The agent is served and registered for this
/// pairing only; its questions come in the stream, which ends with `Done`. `bus` has the time
/// limit of the actions: a device that does not answer ends in an error, never in a wait.
/// Dropped before its end (the panel closed), the stream cancels the pairing and unregisters
/// the agent at once.
pub fn pair(bus: Connection, device: String) -> BoxStream<'static, Pairing> {
    let (events, questions) = mpsc::unbounded();
    let done = events.clone();
    let work = async move {
        let result = pair_with(&bus, &device, events).await;
        let _ = done.unbounded_send(Pairing::Done(result));
    };
    // The questions end once the agent and `done` are dropped: after `Done`.
    let driven = work.into_stream().filter_map(|()| async { None });
    stream::select(questions, driven).boxed()
}

/// The unique name that owns `org.bluez` now: the only caller the agent answers.
async fn bluez_owner(bus: &Connection) -> Result<String> {
    let reply = bus
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &(SERVICE,),
        )
        .await?;
    Ok(reply.body().deserialize::<String>()?)
}

async fn unregister(bus: &Connection) {
    let path = ObjectPath::from_static_str_unchecked(AGENT_PATH);
    let _ = bus
        .call_method(
            Some(SERVICE),
            "/org/bluez",
            Some(AGENT_MANAGER),
            "UnregisterAgent",
            &(&path,),
        )
        .await;
    let _ = bus.object_server().remove::<Agent, _>(AGENT_PATH).await;
}

/// Undoes a pairing left before its end: `CancelPairing` while `Pair` is under way, then the
/// agent unregistered, on the connection's own executor since the pairing's future is gone.
struct Cleanup {
    bus: Connection,
    device: String,
    /// `Pair` has not answered yet.
    pairing: bool,
    armed: bool,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (bus, device, pairing) = (self.bus.clone(), self.device.clone(), self.pairing);
        let undo = async move {
            if pairing && let Err(problem) = cancel_pairing(&bus, &device).await {
                tracing::warn!(%problem, "the pairing left could not be cancelled");
            }
            unregister(&bus).await;
        };
        self.bus
            .executor()
            .spawn(undo, "bluetooth-pairing-cleanup")
            .detach();
    }
}

async fn pair_with(
    bus: &Connection,
    device: &str,
    events: mpsc::UnboundedSender<Pairing>,
) -> Result<()> {
    let agent = Agent {
        events,
        bluez: bluez_owner(bus).await?,
        device: device.to_owned(),
    };
    if !bus.object_server().at(AGENT_PATH, agent).await? {
        return Err(Problem::Failed(String::from(
            "a pairing is already under way",
        )));
    }
    let mut cleanup = Cleanup {
        bus: bus.clone(),
        device: device.to_owned(),
        pairing: false,
        armed: true,
    };
    let path = ObjectPath::from_static_str_unchecked(AGENT_PATH);
    let registered = bus
        .call_method(
            Some(SERVICE),
            "/org/bluez",
            Some(AGENT_MANAGER),
            "RegisterAgent",
            &(&path, CAPABILITY),
        )
        .await;
    let paired = match registered {
        Ok(_) => {
            cleanup.pairing = true;
            let paired = call(bus, device, DEVICE, "Pair").await;
            cleanup.pairing = false;
            match paired {
                Ok(()) => trust_connect(bus, device).await,
                Err(problem) => Err(problem),
            }
        }
        Err(error) => Err(Problem::from(error)),
    };
    cleanup.armed = false;
    unregister(bus).await;
    paired
}

async fn trust_connect(bus: &Connection, device: &str) -> Result<()> {
    set(bus, SERVICE, device, DEVICE, "Trusted", Value::from(true)).await?;
    connect(bus, device).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_of_a_device_comes_from_its_icon() {
        assert_eq!(Kind::of("audio-headset"), Kind::Audio);
        assert_eq!(Kind::of("input-mouse"), Kind::Mouse);
        assert_eq!(Kind::of("input-keyboard"), Kind::Keyboard);
        assert_eq!(Kind::of("phone"), Kind::Phone);
        assert_eq!(Kind::of(""), Kind::Other);
        assert_eq!(code(42), "000042");
        assert!(fresh_code() < 1_000_000);
    }

    #[test]
    fn a_reply_answers_once_and_a_dropped_one_cancels() {
        let (sender, answer) = oneshot::channel();
        let reply = Reply::new(sender);
        reply.send(true);
        reply.send(false);
        assert_eq!(iced::futures::executor::block_on(answer), Ok(true));
        let (sender, answer) = oneshot::channel::<bool>();
        drop(Reply::new(sender));
        assert!(iced::futures::executor::block_on(answer).is_err());
    }
}
