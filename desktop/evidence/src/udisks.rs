//! The real device source: UDisks2 on the system D-Bus, through zbus.
//!
//! Mounting, unmounting and ejecting are UDisks methods (`Filesystem.Mount`, `Unmount`,
//! `Drive.Eject`); UDisks asks polkit, which shows the password prompt when needed. Evidence
//! never runs `mount` or `sudo` itself. A watcher thread follows the bus signals of UDisks so
//! that a key plugged in or pulled out appears or disappears at once.

use std::collections::HashMap;
use std::ffi::CString;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use iced::futures::channel::mpsc;
use iced::futures::{SinkExt, Stream, StreamExt};
use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as Variant};

use crate::devices::{
    self, DeviceError, DeviceSource, Interfaces, Objects, Properties, Value, Volume,
};

const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";

/// A connection to UDisks2.
#[derive(Debug)]
pub struct UDisks {
    connection: Connection,
}

fn error(error: zbus::Error) -> DeviceError {
    match &error {
        zbus::Error::MethodError(name, message, _) => {
            DeviceError::from_dbus(name.as_str(), message.as_deref().unwrap_or_default())
        }
        _ => {
            tracing::warn!(%error, "UDisks2 unreachable");
            DeviceError::Unavailable
        }
    }
}

impl UDisks {
    /// Connects to the system bus.
    pub fn connect() -> Result<Self, DeviceError> {
        Connection::system()
            .map(|connection| Self { connection })
            .map_err(error)
    }

    fn objects(&self) -> Result<Objects, DeviceError> {
        let reply = self
            .connection
            .call_method(
                Some(SERVICE),
                ROOT,
                Some(OBJECT_MANAGER),
                "GetManagedObjects",
                &(),
            )
            .map_err(error)?;
        let raw: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
            reply.body().deserialize().map_err(error)?;
        Ok(raw
            .into_iter()
            .map(|(path, ifaces)| {
                let ifaces: Interfaces = ifaces
                    .into_iter()
                    .map(|(name, props)| {
                        let props: Properties = props
                            .iter()
                            .filter_map(|(key, value)| Some((key.clone(), convert(value)?)))
                            .collect();
                        (name, props)
                    })
                    .collect();
                (path.to_string(), ifaces)
            })
            .collect())
    }

    /// Calls a UDisks method that takes only an empty options dictionary.
    fn call(
        &self,
        path: &str,
        interface: &str,
        method: &str,
    ) -> Result<zbus::Message, DeviceError> {
        let options: HashMap<&str, Variant<'_>> = HashMap::new();
        self.connection
            .call_method(Some(SERVICE), path, Some(interface), method, &(options,))
            .map_err(error)
    }
}

/// Converts the property types Evidence reads; the others are skipped.
fn convert(value: &Variant<'_>) -> Option<Value> {
    Some(match value {
        Variant::Bool(b) => Value::Bool(*b),
        Variant::U8(n) => Value::Int(i64::from(*n)),
        Variant::U16(n) => Value::Int(i64::from(*n)),
        Variant::I16(n) => Value::Int(i64::from(*n)),
        Variant::U32(n) => Value::Int(i64::from(*n)),
        Variant::I32(n) => Value::Int(i64::from(*n)),
        Variant::U64(n) => Value::Int(i64::try_from(*n).unwrap_or(i64::MAX)),
        Variant::I64(n) => Value::Int(*n),
        Variant::Str(s) => Value::Str(s.to_string()),
        Variant::ObjectPath(p) => Value::Str(p.to_string()),
        Variant::Value(inner) => return convert(inner),
        Variant::Array(array) => {
            let bytes = |items: &[Variant<'_>]| -> Vec<u8> {
                let mut out: Vec<u8> = items
                    .iter()
                    .filter_map(|v| match v {
                        Variant::U8(b) => Some(*b),
                        _ => None,
                    })
                    .collect();
                while out.last() == Some(&0) {
                    out.pop();
                }
                out
            };
            match array.element_signature().to_string().as_str() {
                "y" => Value::Bytes(bytes(array)),
                "ay" => Value::BytesList(
                    array
                        .iter()
                        .filter_map(|item| match item {
                            Variant::Array(inner) => Some(bytes(inner)),
                            _ => None,
                        })
                        .collect(),
                ),
                "s" => Value::StrList(
                    array
                        .iter()
                        .filter_map(|item| match item {
                            Variant::Str(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Free bytes of a mounted filesystem.
pub fn free_space(path: &Path) -> Option<u64> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `statvfs` reads the NUL-terminated path, which lives until the end of the
    // call, and only writes into the structure we pass. It returns 0 when it filled the
    // whole structure, which can then be read.
    let stats = unsafe {
        if libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) != 0 {
            return None;
        }
        stats.assume_init()
    };
    stats.f_bavail.checked_mul(stats.f_frsize)
}

fn read_string(props: Option<&Properties>, key: &str) -> String {
    match props.and_then(|p| p.get(key)) {
        Some(Value::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

impl DeviceSource for UDisks {
    fn volumes(&self) -> Result<Vec<Volume>, DeviceError> {
        Ok(devices::volumes(&self.objects()?, &free_space))
    }

    fn mount(&self, id: &str) -> Result<PathBuf, DeviceError> {
        let reply = self.call(id, FILESYSTEM, "Mount")?;
        let point: String = reply.body().deserialize().map_err(error)?;
        Ok(PathBuf::from(point))
    }

    fn unmount(&self, id: &str) -> Result<(), DeviceError> {
        self.call(id, FILESYSTEM, "Unmount").map(|_| ())
    }

    fn eject(&self, id: &str) -> Result<(), DeviceError> {
        let objects = self.objects()?;
        let block = objects.get(id).and_then(|i| i.get(BLOCK));
        let mut drive = read_string(block, "Drive");
        if drive.is_empty() || drive == "/" {
            let backing = read_string(block, "CryptoBackingDevice");
            drive = read_string(objects.get(&backing).and_then(|i| i.get(BLOCK)), "Drive");
        }
        // Everything mounted from the same drive is unmounted first.
        for (path, ifaces) in &objects {
            let on_drive = read_string(ifaces.get(BLOCK), "Drive") == drive;
            let mounted = matches!(
                ifaces.get(FILESYSTEM).and_then(|fs| fs.get("MountPoints")),
                Some(Value::BytesList(points)) if !points.is_empty()
            );
            if on_drive && mounted {
                self.unmount(path)?;
            }
        }
        match self.call(&drive, DRIVE, "Eject") {
            Ok(_) => Ok(()),
            // A disk without a tray or a removable medium cannot be ejected, but it can be
            // powered off before being unplugged.
            Err(DeviceError::Failed(_)) => self.call(&drive, DRIVE, "PowerOff").map(|_| ()),
            Err(error) => Err(error),
        }
    }
}

/// Follows UDisks2: sends the volumes once, then again after every signal that changes them.
/// Runs on its own thread until the receiver is gone.
fn watch(sender: mpsc::UnboundedSender<Vec<Volume>>) {
    let Ok(udisks) = UDisks::connect() else {
        return;
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(SERVICE)
        .map(zbus::match_rule::Builder::build);
    let messages =
        rule.and_then(|rule| MessageIterator::for_match_rule(rule, &udisks.connection, None));
    let mut last = None;
    let mut send = |volumes: Vec<Volume>| {
        if last.as_ref() == Some(&volumes) {
            return true;
        }
        last = Some(volumes.clone());
        sender.unbounded_send(volumes).is_ok()
    };
    if let Ok(volumes) = udisks.volumes()
        && !send(volumes)
    {
        return;
    }
    let Ok(messages) = messages else {
        tracing::warn!("UDisks2 signals unavailable: devices will not refresh by themselves");
        return;
    };
    for message in messages {
        if message.is_err() {
            continue;
        }
        if let Ok(volumes) = udisks.volumes()
            && !send(volumes)
        {
            return;
        }
    }
}

/// The volumes, as a stream for an iced subscription.
pub fn stream() -> impl Stream<Item = Vec<Volume>> {
    iced::stream::channel(4, |mut output: mpsc::Sender<Vec<Volume>>| async move {
        let (sender, mut receiver) = mpsc::unbounded();
        std::thread::spawn(move || watch(sender));
        while let Some(volumes) = receiver.next().await {
            if output.send(volumes).await.is_err() {
                break;
            }
        }
    })
}
