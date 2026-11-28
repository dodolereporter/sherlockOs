//! The system services behind the quick settings. Every write goes through D-Bus to the
//! service that owns the setting, which asks polkit where its policy wants it: NetworkManager
//! (Wi-Fi), BlueZ (Bluetooth), logind (brightness, in the `system-services` crate),
//! power-profiles-daemon (energy). Sound goes through `pactl`, PipeWire having no D-Bus
//! interface. Nothing runs as root, nothing uses `sudo`.
//!
//! Each call takes the connection it goes through, so that the tests run them on a private
//! bus against fake services.

pub mod bluetooth;
pub mod power;

// Sound and brightnesses are shared with the media keys of the compositor (lots D-35, D-35d).
pub use system_services::{Problem, Result, audio, backlight, keyboard, watch};
// Wi-Fi and the helpers of the system bus are shared with Control Room (lot D-26n).
pub use system_services::bus::{
    ACTION_TIMEOUT, READ_TIMEOUT, get, set, system, system_for_actions,
};
pub use system_services::network;
