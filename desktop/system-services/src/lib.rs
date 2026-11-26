//! The system services behind the quick settings of the bar and the media keys of the
//! compositor (lot D-35): the sound of the default output and of the microphone (`pactl`, or
//! `wpctl`), the screen brightness (logind), the keyboard backlight (UPower, or logind), the
//! media players (MPRIS). Every write goes to the
//! service that owns the setting, which asks polkit where its policy wants it. Nothing runs as
//! root, nothing uses `sudo`, nothing writes into sysfs.
//!
//! Each call takes the command or the connection it goes through, so that the tests run them
//! against fake commands and fake services on a private bus. See `docs/media-keys.md`.

pub mod audio;
pub mod backlight;
pub mod keyboard;
pub mod mpris;
pub mod notify;
pub mod watch;

use zbus::DBusError;

/// What went wrong with a service.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    /// The service is not there: not installed, not running, no such device.
    #[error("absent: {0}")]
    Absent(String),
    /// The service, or polkit for it, said no.
    #[error("refused: {0}")]
    Refused(String),
    #[error("failed: {0}")]
    Failed(String),
}

impl From<zbus::Error> for Problem {
    fn from(error: zbus::Error) -> Self {
        let text = error.to_string();
        let name = match &error {
            zbus::Error::MethodError(name, _, _) => name.as_str().to_owned(),
            zbus::Error::FDO(fdo) => fdo.name().to_string(),
            // No bus at all.
            zbus::Error::Address(_) | zbus::Error::InputOutput(_) => {
                return Self::Absent(text);
            }
            _ => String::new(),
        };
        classify(&name, text)
    }
}

impl From<zbus::fdo::Error> for Problem {
    fn from(error: zbus::fdo::Error) -> Self {
        Self::from(zbus::Error::FDO(Box::new(error)))
    }
}

impl From<zbus::zvariant::Error> for Problem {
    fn from(error: zbus::zvariant::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

/// The kind of a D-Bus error, from its name.
fn classify(name: &str, text: String) -> Problem {
    const ABSENT: [&str; 5] = [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.UnknownObject",
        "org.freedesktop.DBus.Error.UnknownInterface",
        "org.freedesktop.DBus.Error.UnknownMethod",
    ];
    const REFUSED: [&str; 4] = [
        "AccessDenied",
        "NotAuthorized",
        "PermissionDenied",
        "InteractiveAuthorizationRequired",
    ];
    if ABSENT.contains(&name) {
        Problem::Absent(text)
    } else if REFUSED.iter().any(|r| name.ends_with(r)) {
        Problem::Refused(text)
    } else {
        Problem::Failed(text)
    }
}

pub type Result<T> = std::result::Result<T, Problem>;

/// `count` presses of a key that moves `value` by `step`, up when positive, down when negative:
/// the value they reach, and whether one of them pushed against the end, leaving the value where
/// it was. The on-screen display stretches when a key insists that way.
pub fn press<T: Copy + PartialEq>(value: T, count: i32, step: impl Fn(T, bool) -> T) -> (T, bool) {
    let up = count > 0;
    (0..count.unsigned_abs()).fold((value, false), |(v, pushed), _| {
        let next = step(v, up);
        (next, pushed || next == v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_sorted_by_their_name() {
        let absent = classify("org.freedesktop.DBus.Error.ServiceUnknown", String::new());
        assert!(matches!(absent, Problem::Absent(_)));
        for name in [
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.NetworkManager.PermissionDenied",
            "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired",
            "org.bluez.Error.NotAuthorized",
        ] {
            let problem = classify(name, String::new());
            assert!(matches!(problem, Problem::Refused(_)), "{name}");
        }
        let failed = classify("org.bluez.Error.Busy", String::new());
        assert!(matches!(failed, Problem::Failed(_)));
    }

    #[test]
    fn a_press_at_the_end_pushes_against_it() {
        let step = |v: u8, up: bool| {
            if up {
                (v + 5).min(100)
            } else {
                v.saturating_sub(5)
            }
        };
        assert_eq!(press(90, 1, step), (95, false));
        assert_eq!(press(95, 1, step), (100, false));
        assert_eq!(press(100, 1, step), (100, true));
        // Merged presses: the last one of three insists.
        assert_eq!(press(90, 3, step), (100, true));
        assert_eq!(press(0, -1, step), (0, true));
        assert_eq!(press(50, 0, step), (50, false));
    }
}
