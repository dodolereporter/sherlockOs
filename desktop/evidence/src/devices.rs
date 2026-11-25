//! Devices of the sidebar: internal disks and their mountable partitions, USB keys, external
//! disks, memory cards and optical drives.
//!
//! The data comes from UDisks2 (see `udisks`) as a plain map of objects, interfaces and
//! properties. Everything here is pure: reading that map, choosing what to show, naming each
//! volume and picking its icon. The actions go through the `DeviceSource` trait, so the
//! application and its tests never need the real system bus.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use sherlock_ui::icon::Icon;
use sherlock_ui::texts::{self, evidence as t};

/// A property value, as far as Evidence needs to read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Str(String),
    /// A byte string (`ay`), such as a device path, without its trailing NUL.
    Bytes(Vec<u8>),
    /// A list of byte strings (`aay`), such as mount points.
    BytesList(Vec<Vec<u8>>),
    StrList(Vec<String>),
}

/// Properties of one interface.
pub type Properties = BTreeMap<String, Value>;
/// Interfaces of one object, by name.
pub type Interfaces = BTreeMap<String, Properties>;
/// All objects of UDisks2, by object path.
pub type Objects = BTreeMap<String, Interfaces>;

const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";
const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
const PARTITION: &str = "org.freedesktop.UDisks2.Partition";

/// Partition types that never hold the user's data, GPT GUIDs and MBR codes: EFI system,
/// extended boot (XBOOTLDR), BIOS boot, Microsoft reserved, Windows recovery, Linux swap.
const SYSTEM_PARTITION_TYPES: [&str; 10] = [
    "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
    "bc13c2ff-59e6-4262-a352-b275fd6f7172",
    "21686148-6449-6e6f-744e-656564454649",
    "e3c9e316-0b5c-4db8-817d-f92df00215ae",
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac",
    "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f",
    "0xef",
    "0x27",
    "0x82",
    "0xea",
];

/// A filesystem that can hold the user's files, on a partition that is not one of the
/// system's own.
fn is_data_partition(block: &Properties, partition: Option<&Properties>) -> bool {
    let mountable = string(block, "IdUsage") == "filesystem" && string(block, "IdType") != "swap";
    let kind = partition
        .map(|p| string(p, "Type").to_lowercase())
        .unwrap_or_default();
    mountable && !SYSTEM_PARTITION_TYPES.contains(&kind.as_str())
}
/// The empty object path, which UDisks uses for "none".
const NO_OBJECT: &str = "/";

fn get<'a>(objects: &'a Objects, path: &str, interface: &str) -> Option<&'a Properties> {
    objects.get(path)?.get(interface)
}

fn string(props: &Properties, key: &str) -> String {
    match props.get(key) {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Bytes(b)) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    }
}

fn boolean(props: &Properties, key: &str) -> bool {
    matches!(props.get(key), Some(Value::Bool(true)))
}

fn number(props: &Properties, key: &str) -> u64 {
    match props.get(key) {
        Some(Value::Int(n)) => u64::try_from(*n).unwrap_or_default(),
        _ => 0,
    }
}

fn paths(props: &Properties, key: &str) -> Vec<PathBuf> {
    match props.get(key) {
        Some(Value::BytesList(list)) => list
            .iter()
            .map(|b| PathBuf::from(String::from_utf8_lossy(b).into_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

/// What kind of device a volume lives on: it decides the icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Internal,
    UsbKey,
    External,
    Card,
    Optical,
}

impl DeviceKind {
    pub fn icon(self) -> Icon {
        match self {
            Self::Internal => Icon::Drive,
            Self::UsbKey => Icon::UsbKey,
            Self::External => Icon::ExternalDrive,
            Self::Card => Icon::Card,
            Self::Optical => Icon::Optical,
        }
    }
}

/// A volume shown in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// UDisks object path of the block device: the volume's identifier.
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    /// Where it is mounted, if it is.
    pub mount_point: Option<PathBuf>,
    /// Size in bytes.
    pub size: u64,
    /// Free bytes, when mounted and readable.
    pub free: Option<u64>,
    /// An encrypted (LUKS) volume that is not unlocked yet.
    pub locked: bool,
    /// The medium can be ejected: removable drive.
    pub ejectable: bool,
}

impl Volume {
    /// The line under or beside the name: free space when mounted.
    pub fn detail(&self) -> Option<String> {
        if self.locked {
            return Some(t::LOCKED.to_owned());
        }
        self.free.map(|free| t::free_space(&texts::size(free)))
    }
}

/// Mount points that belong to the running system: never shown as devices.
fn is_system_mount(path: &std::path::Path) -> bool {
    path == std::path::Path::new("/")
        || path.starts_with("/boot")
        || path.starts_with("/efi")
        || path.starts_with("/usr")
        || path.starts_with("/var")
        || path == std::path::Path::new("/home")
}

fn kind_of(drive: Option<&Properties>) -> DeviceKind {
    let Some(drive) = drive else {
        return DeviceKind::Internal;
    };
    let media = string(drive, "Media");
    let compatible = match drive.get("MediaCompatibility") {
        Some(Value::StrList(list)) => list.clone(),
        _ => Vec::new(),
    };
    let any = |prefix: &str| {
        media.starts_with(prefix) || compatible.iter().any(|m| m.starts_with(prefix))
    };
    if any("optical") {
        DeviceKind::Optical
    } else if any("flash_sd")
        || any("flash_mmc")
        || any("flash_ms")
        || any("flash_cf")
        || string(drive, "ConnectionBus") == "sdio"
    {
        DeviceKind::Card
    } else if any("thumb")
        || (string(drive, "ConnectionBus") == "usb" && boolean(drive, "MediaRemovable"))
    {
        DeviceKind::UsbKey
    } else if string(drive, "ConnectionBus") == "usb"
        || boolean(drive, "Removable")
        || boolean(drive, "Ejectable")
    {
        DeviceKind::External
    } else {
        DeviceKind::Internal
    }
}

/// Reads the volumes to show from UDisks2 objects. `free` gives the free bytes of a mounted
/// path (`statvfs` on the real system).
///
/// Shown: every block device with a filesystem, and every encrypted container whose
/// cleartext side is not unlocked (shown locked). Hidden: what UDisks marks `HintIgnore`,
/// what it marks `HintSystem` unless the user mounted it outside the system's own places,
/// and anything mounted at the root, under /boot or another system path.
pub fn volumes(objects: &Objects, free: &dyn Fn(&std::path::Path) -> Option<u64>) -> Vec<Volume> {
    // Encrypted containers that are unlocked: their cleartext block is shown instead.
    let unlocked: Vec<String> = objects
        .values()
        .filter_map(|ifaces| ifaces.get(BLOCK))
        .map(|block| string(block, "CryptoBackingDevice"))
        .filter(|backing| backing != NO_OBJECT && !backing.is_empty())
        .collect();
    let mut out = Vec::new();
    for (path, ifaces) in objects {
        let Some(block) = ifaces.get(BLOCK) else {
            continue;
        };
        let filesystem = ifaces.get(FILESYSTEM);
        let locked = ifaces.contains_key(ENCRYPTED) && !unlocked.contains(path);
        if filesystem.is_none() && !locked {
            continue;
        }
        if boolean(block, "HintIgnore") {
            continue;
        }
        let mount_point = filesystem
            .map(|fs| paths(fs, "MountPoints"))
            .and_then(|points| points.into_iter().next());
        match &mount_point {
            Some(point) if is_system_mount(point) => continue,
            // An unmounted volume of an internal disk shows only if it holds data: not the
            // EFI or boot partition, not a recovery or reserved one, not swap.
            None if boolean(block, "HintSystem")
                && !locked
                && !is_data_partition(block, ifaces.get(PARTITION)) =>
            {
                continue;
            }
            _ => {}
        }
        // An unlocked container's cleartext block lives on the container's drive.
        let backing = string(block, "CryptoBackingDevice");
        let drive_path = match string(block, "Drive") {
            d if d == NO_OBJECT || d.is_empty() => objects
                .get(&backing)
                .and_then(|i| i.get(BLOCK))
                .map(|b| string(b, "Drive"))
                .unwrap_or_default(),
            d => d,
        };
        // Without a drive, a block is a loop or memory device (snaps, zram): not a device.
        let Some(drive) = get(objects, &drive_path, DRIVE) else {
            continue;
        };
        let drive = Some(drive);
        let size = number(block, "Size");
        let name = name_of(block, drive, size);
        out.push(Volume {
            id: path.clone(),
            name,
            kind: kind_of(drive),
            free: mount_point.as_deref().and_then(free),
            mount_point,
            size,
            locked,
            ejectable: drive.is_some_and(|d| boolean(d, "Ejectable") || boolean(d, "Removable")),
        });
    }
    out.sort_by(|a, b| {
        (a.kind == DeviceKind::Internal)
            .cmp(&(b.kind == DeviceKind::Internal))
            .reverse()
            .then_with(|| crate::listing::natural(&a.name, &b.name))
    });
    out
}

/// The volume's name: its label, else the name UDisks suggests, else the drive's model and
/// the size, else "Volume of 32 GB".
fn name_of(block: &Properties, drive: Option<&Properties>, size: u64) -> String {
    let label = string(block, "IdLabel");
    if !label.trim().is_empty() {
        return label;
    }
    let hint = string(block, "HintName");
    if !hint.trim().is_empty() {
        return hint;
    }
    let size = texts::size(size);
    let model = drive
        .map(|d| {
            let vendor = string(d, "Vendor");
            let model = string(d, "Model");
            format!("{} {}", vendor.trim(), model.trim())
                .trim()
                .to_owned()
        })
        .unwrap_or_default();
    if model.is_empty() {
        t::volume_of(&size)
    } else {
        t::model_of(&model, &size)
    }
}

/// Why an action on a device failed. The message speaks to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceError {
    /// Files are open on the volume.
    Busy,
    /// Polkit refused, or the user dismissed the password prompt.
    NotAllowed,
    /// The volume is encrypted and locked.
    Locked,
    /// The system bus or UDisks2 is not there.
    Unavailable,
    /// Anything else, with UDisks' own message for the log.
    Failed(String),
}

impl DeviceError {
    /// Maps a D-Bus error name from UDisks2 to an error.
    pub fn from_dbus(name: &str, message: &str) -> Self {
        match name {
            "org.freedesktop.UDisks2.Error.DeviceBusy" => Self::Busy,
            n if n.starts_with("org.freedesktop.UDisks2.Error.NotAuthorized") => Self::NotAllowed,
            "org.freedesktop.DBus.Error.AccessDenied" => Self::NotAllowed,
            "org.freedesktop.DBus.Error.ServiceUnknown" => Self::Unavailable,
            _ => Self::Failed(format!("{name}: {message}")),
        }
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Busy => t::DEVICE_BUSY,
            Self::NotAllowed => t::DEVICE_NOT_ALLOWED,
            Self::Locked => t::DEVICE_LOCKED,
            Self::Unavailable => t::DEVICE_UNAVAILABLE,
            Self::Failed(_) => t::DEVICE_FAILED,
        })
    }
}

impl std::error::Error for DeviceError {}

/// Where the volumes come from, and what can be done to them.
pub trait DeviceSource: Send + Sync + fmt::Debug {
    /// The volumes to show now.
    fn volumes(&self) -> Result<Vec<Volume>, DeviceError>;
    /// Mounts a volume; gives where.
    fn mount(&self, id: &str) -> Result<PathBuf, DeviceError>;
    fn unmount(&self, id: &str) -> Result<(), DeviceError>;
    /// Unmounts what is mounted on the volume's drive, then ejects the medium.
    fn eject(&self, id: &str) -> Result<(), DeviceError>;
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A fake source: volumes in memory, mounted under a folder of the test.
    #[derive(Debug)]
    pub struct FakeSource {
        pub volumes: Mutex<Vec<Volume>>,
        pub root: PathBuf,
    }

    impl DeviceSource for FakeSource {
        fn volumes(&self) -> Result<Vec<Volume>, DeviceError> {
            Ok(self.volumes.lock().expect("lock").clone())
        }

        fn mount(&self, id: &str) -> Result<PathBuf, DeviceError> {
            let mut volumes = self.volumes.lock().expect("lock");
            let volume = volumes
                .iter_mut()
                .find(|v| v.id == id)
                .ok_or(DeviceError::Unavailable)?;
            if volume.locked {
                return Err(DeviceError::Locked);
            }
            let point = self.root.join(&volume.name);
            std::fs::create_dir_all(&point).expect("mount point");
            volume.mount_point = Some(point.clone());
            Ok(point)
        }

        fn unmount(&self, id: &str) -> Result<(), DeviceError> {
            let mut volumes = self.volumes.lock().expect("lock");
            let volume = volumes
                .iter_mut()
                .find(|v| v.id == id)
                .ok_or(DeviceError::Unavailable)?;
            if volume.name.contains("occupé") {
                return Err(DeviceError::Busy);
            }
            volume.mount_point = None;
            Ok(())
        }

        fn eject(&self, id: &str) -> Result<(), DeviceError> {
            self.unmount(id)?;
            self.volumes.lock().expect("lock").retain(|v| v.id != id);
            Ok(())
        }
    }

    /// Objects recorded with `busctl --system call org.freedesktop.UDisks2
    /// /org/freedesktop/UDisks2 org.freedesktop.DBus.ObjectManager GetManagedObjects
    /// --json=short` on the reference laptop (internal NVMe disk: EFI, root, swap; zram),
    /// trimmed, plus a USB key, an SD card, a locked LUKS disk and a loop device written in
    /// the same format.
    pub const RECORDED: &str = include_str!("devices/udisks.json");

    /// Reads busctl's JSON into `Objects`.
    pub fn parse_busctl(json: &str) -> Objects {
        let root: serde_json::Value = serde_json::from_str(json).expect("JSON");
        let data = &root["data"][0];
        let mut objects = Objects::new();
        for (path, ifaces) in data.as_object().expect("objects") {
            let mut interfaces = Interfaces::new();
            for (iface, props) in ifaces.as_object().expect("interfaces") {
                let mut properties = Properties::new();
                for (key, typed) in props.as_object().expect("properties") {
                    let data = &typed["data"];
                    let value = match typed["type"].as_str().expect("type") {
                        "b" => Value::Bool(data.as_bool().expect("boolean")),
                        "s" | "o" => Value::Str(data.as_str().expect("string").to_owned()),
                        "t" | "u" | "i" | "x" | "q" | "n" => {
                            Value::Int(data.as_i64().expect("number"))
                        }
                        "ay" => Value::Bytes(bytes(data)),
                        "aay" => Value::BytesList(
                            data.as_array().expect("list").iter().map(bytes).collect(),
                        ),
                        "as" => Value::StrList(
                            data.as_array()
                                .expect("list")
                                .iter()
                                .map(|s| s.as_str().expect("string").to_owned())
                                .collect(),
                        ),
                        _ => continue,
                    };
                    properties.insert(key.clone(), value);
                }
                interfaces.insert(iface.clone(), properties);
            }
            objects.insert(path.clone(), interfaces);
        }
        objects
    }

    fn bytes(data: &serde_json::Value) -> Vec<u8> {
        let mut out: Vec<u8> = data
            .as_array()
            .expect("bytes")
            .iter()
            .map(|b| u8::try_from(b.as_u64().expect("byte")).expect("byte"))
            .collect();
        while out.last() == Some(&0) {
            out.pop();
        }
        out
    }

    fn recorded() -> Vec<Volume> {
        volumes(&parse_busctl(RECORDED), &|_| Some(12_000_000_000))
    }

    #[test]
    fn system_partitions_stay_hidden_and_removable_media_show() {
        let names: Vec<_> = recorded().into_iter().map(|v| v.name).collect();
        // EFI (HintIgnore), root, swap, zram and the snap loop device are hidden, and so are an
        // unmounted recovery partition and an unmounted EFI partition. The internal data
        // partitions show: one mounted under /media, one (Windows) not mounted at all.
        assert_eq!(
            names,
            [
                "Données",
                "Windows",
                "CLE_USB",
                "SL32G de 32 Go",
                "Volume de 64 Go"
            ]
        );
    }

    #[test]
    fn each_volume_gets_its_kind_state_and_free_space() {
        let volumes = recorded();
        let by_name = |name: &str| volumes.iter().find(|v| v.name == name).expect("volume");
        let data = by_name("Données");
        assert_eq!(data.kind, DeviceKind::Internal);
        assert_eq!(
            data.mount_point.as_deref(),
            Some(std::path::Path::new("/media/utilisateur/Données"))
        );
        assert_eq!(data.detail().as_deref(), Some("12 Go libres"));
        assert!(!data.ejectable);
        let key = by_name("CLE_USB");
        assert_eq!(
            (key.kind, key.mount_point.is_none(), key.ejectable),
            (DeviceKind::UsbKey, true, true)
        );
        assert_eq!(key.detail(), None);
        let card = by_name("SL32G de 32 Go");
        assert_eq!(card.kind, DeviceKind::Card);
        let luks = by_name("Volume de 64 Go");
        assert!(luks.locked);
        assert_eq!(luks.kind, DeviceKind::External);
        assert_eq!(luks.detail().as_deref(), Some("Verrouillé"));
        assert_eq!(DeviceKind::UsbKey.icon(), Icon::UsbKey);
    }

    #[test]
    fn udisks_errors_become_short_french_messages() {
        let busy =
            DeviceError::from_dbus("org.freedesktop.UDisks2.Error.DeviceBusy", "target is busy");
        assert_eq!(busy, DeviceError::Busy);
        assert_eq!(busy.to_string(), t::DEVICE_BUSY);
        let denied =
            DeviceError::from_dbus("org.freedesktop.UDisks2.Error.NotAuthorizedDismissed", "");
        assert_eq!(denied, DeviceError::NotAllowed);
        assert!(matches!(
            DeviceError::from_dbus("x.y", "z"),
            DeviceError::Failed(_)
        ));
    }
}
