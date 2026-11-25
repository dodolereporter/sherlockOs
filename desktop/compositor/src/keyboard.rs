//! Keyboard layout: French AZERTY by default.
//!
//! In order: the `XKB_DEFAULT_*` variables (read by xkbcommon itself), the system keyboard
//! setting (`XKBLAYOUT`, `XKBVARIANT`, `XKBOPTIONS`, `XKBMODEL` in `/etc/vconsole.conf`, which
//! systemd's `localectl` writes, or in `/etc/default/keyboard` on Debian), then `fr`.

use std::path::Path;

use smithay::input::keyboard::XkbConfig;

/// System files that may hold the keyboard setting, the first one found wins.
pub const SYSTEM_FILES: [&str; 2] = ["/etc/vconsole.conf", "/etc/default/keyboard"];

/// Layout used when nothing sets one.
const DEFAULT_LAYOUT: &str = "fr";

/// What the keyboard is set to. Empty fields are left to xkbcommon, which reads them from the
/// `XKB_DEFAULT_*` variables, or takes its own default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layout {
    pub model: String,
    pub layout: String,
    pub variant: String,
    pub options: Option<String>,
}

impl Layout {
    /// The layout from the environment (`var`) and the content of the first system file
    /// found (`system`).
    pub fn read(var: impl Fn(&str) -> Option<String>, system: Option<&str>) -> Self {
        if var("XKB_DEFAULT_LAYOUT").is_some_and(|v| !v.is_empty()) {
            return Self::default();
        }
        let value = |key: &str| system.and_then(|text| value(text, key)).unwrap_or_default();
        let layout = value("XKBLAYOUT");
        if layout.is_empty() {
            return Self::azerty();
        }
        let options = value("XKBOPTIONS");
        Self {
            model: value("XKBMODEL"),
            layout,
            variant: value("XKBVARIANT"),
            options: (!options.is_empty()).then_some(options),
        }
    }

    /// French AZERTY, whatever the setting: the fallback when xkb refuses the layout set.
    pub fn azerty() -> Self {
        Self {
            layout: DEFAULT_LAYOUT.into(),
            ..Self::default()
        }
    }

    /// The layout of this machine.
    pub fn system() -> Self {
        let text = SYSTEM_FILES
            .iter()
            .filter_map(|file| std::fs::read_to_string(Path::new(file)).ok())
            .find(|text| value(text, "XKBLAYOUT").is_some_and(|v| !v.is_empty()));
        Self::read(|key| std::env::var(key).ok(), text.as_deref())
    }

    pub fn xkb(&self) -> XkbConfig<'_> {
        XkbConfig {
            model: &self.model,
            layout: &self.layout,
            variant: &self.variant,
            options: self.options.clone(),
            ..XkbConfig::default()
        }
    }
}

/// Value of `key` in a shell-like `KEY="value"` file, quotes removed.
fn value(text: &str, key: &str) -> Option<String> {
    text.lines().rev().find_map(|line| {
        let (k, v) = line.trim().split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches(['"', '\'']).to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBIAN: &str = "# KEYBOARD CONFIGURATION FILE\n\nXKBMODEL=\"pc105\"\nXKBLAYOUT=\"fr\"\nXKBVARIANT=\"latin9\"\nXKBOPTIONS=\"\"\n\nBACKSPACE=\"guess\"\n";

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn azerty_without_any_setting() {
        let layout = Layout::read(no_env, None);
        assert_eq!(layout.layout, "fr");
        assert_eq!((layout.variant.as_str(), layout.options), ("", None));
    }

    #[test]
    fn the_system_setting_is_read() {
        let layout = Layout::read(no_env, Some(DEBIAN));
        assert_eq!(
            layout,
            Layout {
                model: "pc105".into(),
                layout: "fr".into(),
                variant: "latin9".into(),
                options: None,
            }
        );
        let vconsole = "KEYMAP=us\nXKBLAYOUT=us,fr\nXKBOPTIONS='grp:alt_shift_toggle'\n";
        let layout = Layout::read(no_env, Some(vconsole));
        assert_eq!(layout.layout, "us,fr");
        assert_eq!(layout.options.as_deref(), Some("grp:alt_shift_toggle"));
        // A file without a layout (only a console keymap): AZERTY.
        assert_eq!(Layout::read(no_env, Some("KEYMAP=fr\n")).layout, "fr");
    }

    #[test]
    fn the_environment_wins() {
        let env = |key: &str| (key == "XKB_DEFAULT_LAYOUT").then(|| String::from("de"));
        // Everything is left to xkbcommon, which reads XKB_DEFAULT_LAYOUT and the others.
        assert_eq!(Layout::read(env, Some(DEBIAN)), Layout::default());
        // An empty variable is no setting.
        let empty = |key: &str| (key == "XKB_DEFAULT_LAYOUT").then(String::new);
        assert_eq!(Layout::read(empty, None).layout, "fr");
    }

    #[test]
    fn a_layout_xkb_does_not_know_is_refused_and_azerty_is_not() {
        // What `State::new` relies on to fall back to AZERTY.
        let display = smithay::reexports::wayland_server::Display::<crate::State>::new().unwrap();
        let mut seats = smithay::input::SeatState::<crate::State>::new();
        let mut seat = seats.new_wl_seat(&display.handle(), "test");
        let unknown = Layout::read(no_env, Some("XKBLAYOUT=\"no-such-layout\"\n"));
        assert!(seat.add_keyboard(unknown.xkb(), 200, 25).is_err());
        assert!(seat.add_keyboard(Layout::azerty().xkb(), 200, 25).is_ok());
    }
}
