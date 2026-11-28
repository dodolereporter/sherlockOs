//! Keyboard layout: French AZERTY by default.
//!
//! In order: the layout of the keyboard setting (`keyboard.conf` of `desktop-config`, which
//! Control Room writes, lot D-26c), the `XKB_DEFAULT_*` variables (read by xkbcommon itself), the system keyboard
//! setting (`XKBLAYOUT`, `XKBVARIANT`, `XKBOPTIONS`, `XKBMODEL` in `/etc/vconsole.conf`, which
//! systemd's `localectl` writes, or in `/etc/default/keyboard` on Debian), then `fr`.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use desktop_config::keyboard::Config;
use smithay::{
    input::keyboard::XkbConfig,
    reexports::calloop::{
        LoopHandle,
        timer::{TimeoutAction, Timer},
    },
};

use crate::State;

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

    /// The layout of `config` if it names one, on the model and options of `system`;
    /// `system` otherwise.
    pub fn chosen(config: &Config, system: Self) -> Self {
        match &config.layout {
            Some(layout) => Self {
                layout: layout.clone(),
                variant: config.variant.clone(),
                ..system
            },
            None => system,
        }
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

/// How often the settings file is looked at.
const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// The keyboard setting, the file it comes from, and the layout given to xkb last.
#[derive(Default)]
pub struct KeyboardSetting {
    config: Config,
    file: Option<PathBuf>,
    /// Modification time and length of the file when last read.
    stamp: Option<(SystemTime, u64)>,
    /// The layout xkb took last; `None` until the setting is first applied.
    applied: Option<Layout>,
}

impl KeyboardSetting {
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
        self.config = self.file.as_deref().map(read).unwrap_or_default();
        tracing::info!(config = ?self.config, "keyboard");
        true
    }
}

/// Largest settings file read: a link to `/dev/zero` or a huge file is left aside.
const MAX_FILE: u64 = 64 * 1024;

/// The setting of `file`, read leniently: a line it gets wrong goes to the log and the others
/// count. A missing, unreadable or oversized file is the default setting.
fn read(file: &Path) -> Config {
    let text = match fs::metadata(file) {
        Ok(meta) if meta.len() > MAX_FILE => {
            tracing::warn!("keyboard: {}: larger than {MAX_FILE} bytes", file.display());
            return Config::default();
        }
        Ok(_) => fs::read_to_string(file),
        Err(err) => Err(err),
    };
    match text {
        Ok(text) => {
            let (config, problems) = Config::parse_lenient(&text);
            for problem in problems {
                tracing::warn!("keyboard: {}: {problem}", file.display());
            }
            config
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(err) => {
            tracing::warn!("keyboard: {}: {err}", file.display());
            Config::default()
        }
    }
}

/// Modification time and length of a file, if it exists.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl State {
    /// Reads the keyboard setting of `file` and applies it, then looks at it again every
    /// `RELOAD_EVERY`.
    pub fn watch_keyboard(
        &mut self,
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
    ) -> Result<(), crate::Error> {
        self.keyboard_setting.file = file;
        self.keyboard_setting.check();
        self.apply_keyboard();
        handle
            .insert_source(Timer::from_duration(RELOAD_EVERY), |_, _, state| {
                if state.keyboard_setting.check() {
                    state.apply_keyboard();
                }
                TimeoutAction::ToDuration(RELOAD_EVERY)
            })
            .map_err(|err| err.error)?;
        Ok(())
    }

    /// Changes the keyboard setting, as the file would.
    pub fn set_keyboard(&mut self, config: Config) {
        self.keyboard_setting.config = config;
        self.apply_keyboard();
    }

    /// Delay before a held key repeats and time between two repeats: those of the clients,
    /// followed by the media keys.
    pub fn key_repeat(&self) -> (Duration, Duration) {
        let config = &self.keyboard_setting.config;
        (
            Duration::from_millis(u64::from(config.repeat_delay)),
            Duration::from_millis(1000 / u64::from(config.repeat_rate.max(1))),
        )
    }

    /// Gives xkb the layout of the setting when it changed, and the clients the repeat. A
    /// layout xkb refuses leaves the one in use: the keyboard never stops answering.
    fn apply_keyboard(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let config = self.keyboard_setting.config.clone();
        let wanted = Layout::chosen(&config, Layout::system());
        if self.keyboard_setting.applied.as_ref() != Some(&wanted) {
            match keyboard.set_xkb_config(self, wanted.xkb()) {
                Ok(()) => {
                    tracing::info!(layout = ?wanted, "keyboard layout");
                    self.keyboard_setting.applied = Some(wanted);
                }
                Err(err) => tracing::warn!(?wanted, "keyboard layout refused ({err}), kept"),
            }
        }
        let rate = i32::try_from(config.repeat_rate).unwrap_or(crate::KEY_REPEAT_RATE);
        let delay = i32::try_from(config.repeat_delay).unwrap_or(crate::KEY_REPEAT_DELAY);
        keyboard.change_repeat_info(rate, delay);
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
    fn the_layout_of_the_setting_wins_and_keeps_the_system_model() {
        let system = Layout::read(no_env, Some(DEBIAN));
        assert_eq!(Layout::chosen(&Config::default(), system.clone()), system);
        let config = Config {
            layout: Some("us".into()),
            variant: "intl".into(),
            ..Config::default()
        };
        let chosen = Layout::chosen(&config, system);
        assert_eq!(
            (chosen.layout.as_str(), chosen.variant.as_str()),
            ("us", "intl")
        );
        assert_eq!(chosen.model, "pc105");
    }

    #[test]
    fn the_file_is_read_again_and_applied() {
        let dir = std::env::temp_dir().join(format!("compositor-keyboard-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("keyboard.conf");
        let _ = fs::remove_file(&file);
        let mut setting = KeyboardSetting {
            file: Some(file.clone()),
            ..KeyboardSetting::default()
        };
        assert!(!setting.check(), "no file, nothing new");
        fs::write(&file, "repeat-delay = 600\nrepeat-rate = 10\n").unwrap();
        assert!(setting.check());
        assert_eq!(setting.config().repeat_delay, 600);
        // A wrong line is left aside, the others count.
        fs::write(&file, "repeat-rate = 900\nrepeat-delay = 400\n").unwrap();
        assert!(setting.check());
        let expected = Config {
            repeat_delay: 400,
            ..Config::default()
        };
        assert_eq!(*setting.config(), expected);
        fs::remove_dir_all(&dir).unwrap();

        // Applied: a layout xkb refuses keeps the one in use, the repeat still changes.
        let event_loop = smithay::reexports::calloop::EventLoop::<State>::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.set_keyboard(Config {
            layout: Some("us".into()),
            ..Config::default()
        });
        let us = state.keyboard_setting.applied.clone().unwrap();
        assert_eq!(us.layout, "us");
        state.set_keyboard(Config {
            layout: Some("nosuchlayout".into()),
            repeat_delay: 500,
            repeat_rate: 20,
            ..Config::default()
        });
        assert_eq!(state.keyboard_setting.applied, Some(us));
        let (delay, interval) = state.key_repeat();
        assert_eq!((delay.as_millis(), interval.as_millis()), (500, 50));
    }

    #[test]
    fn every_layout_control_room_offers_is_known_to_xkb() {
        let display = smithay::reexports::wayland_server::Display::<crate::State>::new().unwrap();
        let mut seats = smithay::input::SeatState::<crate::State>::new();
        let mut seat = seats.new_wl_seat(&display.handle(), "test");
        for (layout, variant, name) in sherlock_ui::control_room::KEYBOARD_LAYOUTS {
            let layout = Layout {
                layout: layout.into(),
                variant: variant.into(),
                ..Layout::default()
            };
            assert!(seat.add_keyboard(layout.xkb(), 200, 25).is_ok(), "{name}");
        }
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
