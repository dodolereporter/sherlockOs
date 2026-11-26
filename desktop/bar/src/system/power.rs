//! Energy profiles through power-profiles-daemon on the system bus, under its current name
//! (`org.freedesktop.UPower.PowerProfiles`) or its first one (`net.hadess.PowerProfiles`).

use std::collections::HashMap;

use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

use super::{Problem, Result, get, set};

/// The two names of the service, the current one first: name, object, interface.
pub const NAMES: [(&str, &str, &str); 2] = [
    (
        "org.freedesktop.UPower.PowerProfiles",
        "/org/freedesktop/UPower/PowerProfiles",
        "org.freedesktop.UPower.PowerProfiles",
    ),
    (
        "net.hadess.PowerProfiles",
        "/net/hadess/PowerProfiles",
        "net.hadess.PowerProfiles",
    ),
];

/// A profile of the board: « Économie », « Équilibré », « Performance ».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Saver,
    Balanced,
    Performance,
}

impl Profile {
    pub const ALL: [Self; 3] = [Self::Saver, Self::Balanced, Self::Performance];

    /// Its name for the daemon.
    pub fn key(self) -> &'static str {
        match self {
            Self::Saver => "power-saver",
            Self::Balanced => "balanced",
            Self::Performance => "performance",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }
}

/// The daemon's state, and which of its names answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Power {
    pub active: Option<Profile>,
    /// The profiles this machine has: performance is missing on some.
    pub available: Vec<Profile>,
    name: usize,
}

impl Power {
    /// A state as the daemon under its current name would give it.
    pub fn new(active: Option<Profile>, available: Vec<Profile>) -> Self {
        Self {
            active,
            available,
            name: 0,
        }
    }
}

/// Reads the profiles, from the first name of the daemon that answers.
pub async fn read(bus: &Connection) -> Result<Power> {
    let mut last = Problem::Absent(String::from("power-profiles-daemon"));
    for (index, (service, path, interface)) in NAMES.into_iter().enumerate() {
        let active: String = match get(bus, service, path, interface, "ActiveProfile").await {
            Ok(active) => active,
            Err(problem @ Problem::Absent(_)) => {
                last = problem;
                continue;
            }
            Err(problem) => return Err(problem),
        };
        let profiles: Vec<HashMap<String, OwnedValue>> =
            get(bus, service, path, interface, "Profiles").await?;
        let available = profiles
            .iter()
            .filter_map(|p| p.get("Profile"))
            .filter_map(|p| p.try_clone().ok())
            .filter_map(|p| String::try_from(p).ok())
            .filter_map(|key| Profile::from_key(&key))
            .collect();
        return Ok(Power {
            active: Profile::from_key(&active),
            available,
            name: index,
        });
    }
    Err(last)
}

/// Makes `profile` active (polkit: `switch-profile`).
pub async fn choose(bus: &Connection, state: &Power, profile: Profile) -> Result<()> {
    let (service, path, interface) = NAMES[state.name.min(NAMES.len() - 1)];
    let value = Value::from(profile.key());
    set(bus, service, path, interface, "ActiveProfile", value).await
}
