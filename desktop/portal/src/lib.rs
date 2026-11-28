//! The portal backend of the SherlockOS session (lot D-28d, ADR 0028): screen sharing for the
//! applications that ask `xdg-desktop-portal` (Discord, Chrome, Firefox). See `docs/portal.md`.

pub mod capture;
pub mod chooser;
pub mod picker;
pub mod pipewire;
pub mod pod;
pub mod screencast;

/// The name of the backend on the session bus, given in `sherlockos.portal`.
pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.sherlockos";
/// Where the frontend looks for the backend's interfaces.
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
