//! The media keys of the real session (lot D-35): sound and microphone, screen brightness and
//! media players. The compositor takes them whatever has the keyboard, and a worker thread does
//! what they ask, so that a slow `pactl` or bus never holds the event loop: the sound through
//! `pactl` or `wpctl`, the brightness through logind's `SetBrightness`, the players through
//! MPRIS (`system-services`, shared with the quick settings of the bar). What changed comes back
//! to the loop and shows in the on-screen display (`osd`). The volume and brightness keys
//! repeat while held, at the pace of the keyboard. See `docs/media-keys.md`.

use std::{path::PathBuf, sync::mpsc, time::Duration};

use sherlock_ui::osd::Kind;
use smithay::{
    input::keyboard::{Keycode, Keysym},
    reexports::calloop::{
        RegistrationToken,
        channel::{self, Event},
        timer::{TimeoutAction, Timer},
    },
};
use system_services::{
    Problem,
    audio::{Device, Mixer},
    backlight, mpris,
};

use crate::{State, osd::Level};

/// Delay before a held key repeats, and its pace: those of the keyboard (`State::new`).
pub const REPEAT_DELAY: Duration = Duration::from_millis(crate::KEY_REPEAT_DELAY as u64);
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(1000 / crate::KEY_REPEAT_RATE as u64);
/// Longest wait for logind or a media player.
const BUS_TIMEOUT: Duration = Duration::from_secs(5);

/// A key the compositor takes for the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKey {
    VolumeUp,
    VolumeDown,
    Mute,
    MicMute,
    BrightnessUp,
    BrightnessDown,
    Player(mpris::Command),
}

impl MediaKey {
    pub fn of(sym: Keysym) -> Option<Self> {
        Some(match sym {
            Keysym::XF86_AudioRaiseVolume => Self::VolumeUp,
            Keysym::XF86_AudioLowerVolume => Self::VolumeDown,
            Keysym::XF86_AudioMute => Self::Mute,
            Keysym::XF86_AudioMicMute => Self::MicMute,
            Keysym::XF86_MonBrightnessUp => Self::BrightnessUp,
            Keysym::XF86_MonBrightnessDown => Self::BrightnessDown,
            Keysym::XF86_AudioPlay => Self::Player(mpris::Command::PlayPause),
            Keysym::XF86_AudioPause => Self::Player(mpris::Command::Pause),
            Keysym::XF86_AudioNext => Self::Player(mpris::Command::Next),
            Keysym::XF86_AudioPrev => Self::Player(mpris::Command::Previous),
            _ => return None,
        })
    }

    /// Held, the key does it again: the steps of the volume and of the brightness.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Self::VolumeUp | Self::VolumeDown | Self::BrightnessUp | Self::BrightnessDown
        )
    }
}

/// What the keys go through. The tests give fake commands and private buses.
#[derive(Debug, Clone)]
pub struct Controls {
    pub mixer: Mixer,
    /// Where the kernel lists the backlights; only read.
    pub backlights: PathBuf,
    /// Address of the system bus (logind), or `None` for the machine's.
    pub system_bus: Option<String>,
    /// Address of the session bus (media players), or `None` for the session's.
    pub session_bus: Option<String>,
}

impl Controls {
    /// The machine's: `pactl` or `wpctl`, `/sys/class/backlight`, its buses.
    pub fn system() -> Self {
        Self {
            mixer: Mixer::find(),
            backlights: PathBuf::from(backlight::SYSFS),
            system_bus: None,
            session_bus: None,
        }
    }
}

/// The keys at work: the worker thread and the key held.
pub struct MediaKeys {
    worker: mpsc::Sender<MediaKey>,
    held: Option<(Keycode, RegistrationToken)>,
}

/// A bus, opened on first use and kept.
struct Bus {
    address: Option<String>,
    system: bool,
    connection: Option<zbus::Connection>,
}

impl Bus {
    fn get(&mut self) -> Result<zbus::Connection, Problem> {
        if let Some(connection) = &self.connection {
            return Ok(connection.clone());
        }
        let builder = match (&self.address, self.system) {
            (Some(address), _) => zbus::connection::Builder::address(address.as_str())?,
            (None, true) => zbus::connection::Builder::system()?,
            (None, false) => zbus::connection::Builder::session()?,
        };
        let connection = zbus::block_on(builder.method_timeout(BUS_TIMEOUT).build())?;
        self.connection = Some(connection.clone());
        Ok(connection)
    }
}

/// Does what `key` asks, on the worker thread. Returns what to show.
fn act(controls: &Controls, buses: &mut (Bus, Bus), key: MediaKey) -> Result<Option<Level>, Problem> {
    let sound = |kind, sound: system_services::audio::Sound| Level {
        kind,
        percent: sound.volume,
        muted: sound.muted,
    };
    Ok(Some(match key {
        MediaKey::VolumeUp | MediaKey::VolumeDown => {
            let up = key == MediaKey::VolumeUp;
            sound(Kind::Volume, controls.mixer.step_volume(up)?)
        }
        MediaKey::Mute => sound(Kind::Volume, controls.mixer.toggle_mute(Device::Output)?),
        MediaKey::MicMute => sound(
            Kind::Microphone,
            controls.mixer.toggle_mute(Device::Microphone)?,
        ),
        MediaKey::BrightnessUp | MediaKey::BrightnessDown => {
            let up = key == MediaKey::BrightnessUp;
            let bus = buses.0.get()?;
            let percent = zbus::block_on(backlight::step_from(&bus, &controls.backlights, up))?;
            Level {
                kind: Kind::Brightness,
                percent: u16::from(percent),
                muted: false,
            }
        }
        MediaKey::Player(command) => {
            let bus = buses.1.get()?;
            let player = zbus::block_on(mpris::send(&bus, command))?;
            tracing::debug!(player, ?command, "media key sent");
            return Ok(None);
        }
    }))
}

/// The worker: one key after the other, in the order pressed.
fn work(controls: Controls, keys: mpsc::Receiver<MediaKey>, shown: channel::Sender<Level>) {
    let bus = |address: &Option<String>, system| Bus {
        address: address.clone(),
        system,
        connection: None,
    };
    let mut buses = (
        bus(&controls.system_bus, true),
        bus(&controls.session_bus, false),
    );
    while let Ok(key) = keys.recv() {
        match act(&controls, &mut buses, key) {
            Ok(Some(level)) => {
                if shown.send(level).is_err() {
                    return;
                }
            }
            Ok(None) => {}
            Err(problem) => tracing::warn!(?key, "media key: {problem}"),
        }
    }
}

impl State {
    /// Takes the media keys from now on, through `controls`. The real session calls it; nested
    /// mode leaves them to the host session.
    pub fn enable_media_keys(&mut self, controls: Controls) {
        let (keys, worker_keys) = mpsc::channel();
        let (shown, levels) = channel::channel();
        let inserted = self.loop_handle.insert_source(levels, |event, _, state| {
            if let Event::Msg(level) = event {
                state.show_osd(level);
            }
        });
        if let Err(err) = inserted {
            tracing::warn!("media keys left to the clients: {err}");
            return;
        }
        let spawned = std::thread::Builder::new()
            .name(String::from("media-keys"))
            .spawn(move || work(controls, worker_keys, shown));
        if let Err(err) = spawned {
            tracing::warn!("media keys left to the clients: {err}");
            return;
        }
        self.media_keys = Some(MediaKeys {
            worker: keys,
            held: None,
        });
    }

    /// The media keys are taken.
    pub(crate) fn takes_media_keys(&self) -> bool {
        self.media_keys.is_some()
    }

    /// `key` (`code`) is pressed: its action goes to the worker, and again while it is held.
    pub(crate) fn press_media_key(&mut self, code: Keycode, key: MediaKey) {
        self.stop_media_repeat(None);
        let Some(keys) = &self.media_keys else {
            return;
        };
        let _ = keys.worker.send(key);
        if !key.repeats() {
            return;
        }
        let worker = keys.worker.clone();
        let timer = Timer::from_duration(REPEAT_DELAY);
        let token = self.loop_handle.insert_source(timer, move |_, _, _| {
            match worker.send(key) {
                Ok(()) => TimeoutAction::ToDuration(REPEAT_INTERVAL),
                Err(_) => TimeoutAction::Drop,
            }
        });
        match token {
            Ok(token) => {
                if let Some(keys) = &mut self.media_keys {
                    keys.held = Some((code, token));
                }
            }
            Err(err) => tracing::warn!("media key repeat: {err}"),
        }
    }

    /// Stops the repeat of the held media key: when `code` is released, or any with `None`
    /// (another key, the session left).
    pub(crate) fn stop_media_repeat(&mut self, code: Option<Keycode>) {
        let Some(keys) = &mut self.media_keys else {
            return;
        };
        if let Some((held, token)) = keys.held
            && code.is_none_or(|code| code == held)
        {
            keys.held = None;
            self.loop_handle.remove(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_known_by_their_symbol() {
        assert_eq!(
            MediaKey::of(Keysym::XF86_AudioRaiseVolume),
            Some(MediaKey::VolumeUp)
        );
        assert_eq!(
            MediaKey::of(Keysym::XF86_AudioPlay),
            Some(MediaKey::Player(mpris::Command::PlayPause))
        );
        assert_eq!(MediaKey::of(Keysym::a), None);
        assert!(MediaKey::BrightnessDown.repeats());
        assert!(!MediaKey::Mute.repeats());
        assert!(REPEAT_INTERVAL < REPEAT_DELAY);
    }
}
