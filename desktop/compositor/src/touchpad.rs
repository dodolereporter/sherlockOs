//! The touchpad setting (`touchpad.conf` of `desktop-config`, lot D-31) applied to libinput:
//! tap to click and to drag, natural two finger scrolling, disable while typing, speed and
//! acceleration, clicks by fingers or by areas. Read again when the file changes, and applied
//! to every touchpad, those already there and those plugged in later.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use desktop_config::touchpad::{Acceleration, Click, Config};
use smithay::reexports::{
    calloop::{
        LoopHandle,
        timer::{TimeoutAction, Timer},
    },
    input::{self, AccelProfile, ClickMethod, DeviceConfigError, ScrollMethod},
};

use crate::State;

/// How often the settings file is looked at.
const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// The setting, the file it comes from, and the touchpads it applies to.
#[derive(Default)]
pub struct Touchpad {
    config: Config,
    file: Option<PathBuf>,
    /// Modification time and length of the file when last read.
    stamp: Option<(SystemTime, u64)>,
    devices: Vec<input::Device>,
}

impl Touchpad {
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Reads the file again if it changed, appeared or went away. `true` if it was read.
    fn check(&mut self) -> bool {
        let stamp = self.file.as_deref().and_then(stamp);
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        self.config = match &self.file {
            Some(file) => match Config::read_lenient(file) {
                Ok((config, problems)) => {
                    for problem in problems {
                        tracing::warn!("touchpad: {}: {problem}, line ignored", file.display());
                    }
                    config
                }
                Err(err) => {
                    tracing::warn!("touchpad: {}: {err}", file.display());
                    Config::default()
                }
            },
            None => Config::default(),
        };
        tracing::info!(config = ?self.config, "touchpad");
        true
    }
}

/// Modification time and length of a file, if it exists.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// One libinput setting of a touchpad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Setting {
    Tap(bool),
    TapDrag(bool),
    DragLock(bool),
    TwoFingerScroll,
    NaturalScroll(bool),
    DisableWhileTyping(bool),
    Speed(f64),
    Profile(AccelProfile),
    Click(ClickMethod),
}

/// What `config` asks of libinput, in order.
pub fn settings(config: &Config) -> [Setting; 9] {
    [
        Setting::Tap(config.tap_to_click),
        Setting::TapDrag(config.tap_and_drag),
        Setting::DragLock(config.drag_lock),
        Setting::TwoFingerScroll,
        Setting::NaturalScroll(config.natural_scroll),
        Setting::DisableWhileTyping(config.disable_while_typing),
        Setting::Speed(config.speed),
        Setting::Profile(match config.acceleration {
            Acceleration::Adaptive => AccelProfile::Adaptive,
            Acceleration::Flat => AccelProfile::Flat,
        }),
        Setting::Click(match config.click {
            Click::Fingers => ClickMethod::Clickfinger,
            Click::Areas => ClickMethod::ButtonAreas,
        }),
    ]
}

/// Whether libinput reads `device` as a touchpad: it taps.
fn is_touchpad(device: &input::Device) -> bool {
    device.config_tap_finger_count() > 0
}

/// Applies `config` to the touchpad `device`. What it does not support is skipped: a touchpad
/// without a physical button has no button areas, for instance.
fn apply(device: &mut input::Device, config: &Config) {
    for setting in settings(config) {
        let result = match setting {
            Setting::Tap(on) => device.config_tap_set_enabled(on),
            Setting::TapDrag(on) => device.config_tap_set_drag_enabled(on),
            Setting::DragLock(on) => device.config_tap_set_drag_lock_enabled(on),
            Setting::TwoFingerScroll => device.config_scroll_set_method(ScrollMethod::TwoFinger),
            Setting::NaturalScroll(on) => device.config_scroll_set_natural_scroll_enabled(on),
            Setting::DisableWhileTyping(on) => device.config_dwt_set_enabled(on),
            Setting::Speed(speed) => device.config_accel_set_speed(speed),
            Setting::Profile(profile) => device.config_accel_set_profile(profile),
            Setting::Click(method) => device.config_click_set_method(method),
        };
        match result {
            Ok(()) => {}
            Err(DeviceConfigError::Unsupported) => {
                tracing::debug!(name = device.name(), ?setting, "touchpad: not supported");
            }
            Err(err) => tracing::warn!(name = device.name(), ?setting, ?err, "touchpad"),
        }
    }
}

impl State {
    /// Reads the touchpad setting of `file`, then looks at it again every `RELOAD_EVERY`.
    pub fn watch_touchpad(
        &mut self,
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
    ) -> Result<(), crate::Error> {
        self.touchpad.file = file;
        self.touchpad.check();
        handle
            .insert_source(Timer::from_duration(RELOAD_EVERY), |_, _, state| {
                if state.touchpad.check() {
                    state.apply_touchpad();
                }
                TimeoutAction::ToDuration(RELOAD_EVERY)
            })
            .map_err(|err| err.error)?;
        Ok(())
    }

    /// Changes the touchpad setting, as the file would: what Control Room's page will do.
    pub fn set_touchpad(&mut self, config: Config) {
        self.touchpad.config = config;
        self.apply_touchpad();
    }

    fn apply_touchpad(&mut self) {
        let config = self.touchpad.config;
        for device in &mut self.touchpad.devices {
            apply(device, &config);
        }
    }

    /// A device was plugged in: a touchpad takes the setting.
    pub(crate) fn input_device_added(&mut self, mut device: input::Device) {
        tracing::info!(name = device.name(), "input device");
        if is_touchpad(&device) {
            apply(&mut device, &self.touchpad.config);
            self.touchpad.devices.push(device);
        }
    }

    pub(crate) fn input_device_removed(&mut self, device: &input::Device) {
        self.touchpad.devices.retain(|d| d != device);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_becomes_libinput_settings() {
        let default = settings(&Config::default());
        for expected in [
            Setting::Tap(true),
            Setting::TapDrag(true),
            Setting::DragLock(false),
            Setting::TwoFingerScroll,
            Setting::NaturalScroll(true),
            Setting::DisableWhileTyping(true),
            Setting::Speed(0.0),
            Setting::Profile(AccelProfile::Adaptive),
            Setting::Click(ClickMethod::Clickfinger),
        ] {
            assert!(default.contains(&expected), "{expected:?}");
        }
        let config = Config {
            natural_scroll: false,
            speed: 0.4,
            acceleration: Acceleration::Flat,
            click: Click::Areas,
            ..Config::default()
        };
        let changed = settings(&config);
        assert!(changed.contains(&Setting::NaturalScroll(false)));
        assert!(changed.contains(&Setting::Speed(0.4)));
        assert!(changed.contains(&Setting::Profile(AccelProfile::Flat)));
        assert!(changed.contains(&Setting::Click(ClickMethod::ButtonAreas)));
    }

    #[test]
    fn the_file_is_read_again_when_it_changes() {
        let dir = std::env::temp_dir().join(format!("compositor-touchpad-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("touchpad.conf");
        let _ = fs::remove_file(&file);
        let mut touchpad = Touchpad {
            file: Some(file.clone()),
            ..Touchpad::default()
        };
        assert!(!touchpad.check(), "no file, nothing new");
        fs::write(&file, "natural-scroll = false\n").unwrap();
        assert!(touchpad.check());
        assert!(!touchpad.config().natural_scroll);
        assert!(!touchpad.check(), "read once");
        // A wrong file gives the defaults.
        fs::write(&file, "natural-scroll = maybe, maybe not\n").unwrap();
        assert!(touchpad.check());
        assert_eq!(*touchpad.config(), Config::default());
        fs::remove_dir_all(&dir).unwrap();
    }
}
