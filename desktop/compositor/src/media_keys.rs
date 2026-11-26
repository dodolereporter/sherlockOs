//! The media keys of the real session (lots D-35, D-35d): sound and microphone, screen and
//! keyboard brightness, media players. The compositor takes them whatever has the keyboard, and
//! a worker thread does what they ask, so that a slow `pactl` or bus never holds the event loop:
//! the sound through `pactl` or `wpctl`, the screen brightness through logind's `SetBrightness`,
//! the keyboard backlight through UPower (or logind), the players through MPRIS
//! (`system-services`, shared with the quick settings of the bar). What changed comes back to
//! the loop and shows in the on-screen display (`osd`). The volume and brightness keys repeat
//! while held, at the pace of the keyboard. See `docs/media-keys.md`.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

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
    backlight, keyboard, mpris,
};

use crate::{
    State,
    osd::{Edge, Level},
};

/// Delay before a held key repeats, and its pace: those of the keyboard (`State::new`).
pub const REPEAT_DELAY: Duration = Duration::from_millis(crate::KEY_REPEAT_DELAY as u64);
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(1000 / crate::KEY_REPEAT_RATE as u64);
/// Longest wait for logind, UPower or a media player.
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
    KeyboardUp,
    KeyboardDown,
    /// Turns the keyboard backlight off, or back on.
    KeyboardToggle,
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
            Keysym::XF86_KbdBrightnessUp => Self::KeyboardUp,
            Keysym::XF86_KbdBrightnessDown => Self::KeyboardDown,
            Keysym::XF86_KbdLightOnOff => Self::KeyboardToggle,
            Keysym::XF86_AudioPlay => Self::Player(mpris::Command::PlayPause),
            Keysym::XF86_AudioPause => Self::Player(mpris::Command::Pause),
            Keysym::XF86_AudioNext => Self::Player(mpris::Command::Next),
            Keysym::XF86_AudioPrev => Self::Player(mpris::Command::Previous),
            _ => return None,
        })
    }

    /// Held, the key does it again: the steps of the volume and of the brightnesses.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Self::VolumeUp
                | Self::VolumeDown
                | Self::BrightnessUp
                | Self::BrightnessDown
                | Self::KeyboardUp
                | Self::KeyboardDown
        )
    }
}

/// What the keys go through. The tests give fake commands and private buses.
#[derive(Debug, Clone)]
pub struct Controls {
    pub mixer: Mixer,
    /// Where the kernel lists the backlights and the LEDs (the keyboard's, when UPower is not
    /// there); only read.
    pub backlights: PathBuf,
    pub leds: PathBuf,
    /// Address of the system bus (logind), or `None` for the machine's.
    pub system_bus: Option<String>,
    /// Address of the session bus (media players), or `None` for the session's.
    pub session_bus: Option<String>,
}

impl Controls {
    /// The machine's: `pactl` or `wpctl`, `/sys/class/backlight` and `/sys/class/leds`, its
    /// buses.
    pub fn system() -> Self {
        Self {
            mixer: Mixer::find(),
            backlights: PathBuf::from(backlight::SYSFS),
            leds: PathBuf::from(keyboard::LEDS),
            system_bus: None,
            session_bus: None,
        }
    }
}

/// The keys at work: the worker thread, whether it is at work, and the key held.
pub struct MediaKeys {
    worker: mpsc::Sender<MediaKey>,
    /// Set from a key sent until the worker has done everything it was given: a held key only
    /// repeats when it is not set, so that a slow `pactl` never piles keys up.
    busy: Arc<AtomicBool>,
    held: Option<(Keycode, RegistrationToken)>,
}

/// The keys waiting for the worker, in order, with the steps of the volume or of the
/// brightness that follow each other merged into one: their sum, set at once.
pub fn merge(keys: &[MediaKey]) -> Vec<(MediaKey, i32)> {
    let axis = |key: MediaKey| match key {
        MediaKey::VolumeUp => Some((MediaKey::VolumeUp, 1)),
        MediaKey::VolumeDown => Some((MediaKey::VolumeUp, -1)),
        MediaKey::BrightnessUp => Some((MediaKey::BrightnessUp, 1)),
        MediaKey::BrightnessDown => Some((MediaKey::BrightnessUp, -1)),
        MediaKey::KeyboardUp => Some((MediaKey::KeyboardUp, 1)),
        MediaKey::KeyboardDown => Some((MediaKey::KeyboardUp, -1)),
        _ => None,
    };
    let mut merged: Vec<(MediaKey, i32)> = Vec::new();
    for &key in keys {
        match (axis(key), merged.last_mut()) {
            (Some((first, step)), Some((last, steps))) if *last == first => *steps += step,
            (Some((first, step)), _) => merged.push((first, step)),
            (None, _) => merged.push((key, 0)),
        }
    }
    merged.retain(|(key, steps)| axis(*key).is_none() || *steps != 0);
    merged
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

/// What the worker keeps from one key to the next: its buses, opened on first use, and the
/// keyboard brightness its on and off key turned off.
struct Worker {
    system: Bus,
    session: Bus,
    keyboard_off: Option<u32>,
}

/// What a key changed: the level to show, and the end it pushed against.
pub type Shown = (Level, Option<Edge>);

/// The end of `steps` that pushed against it, if one did.
fn edge(steps: i32, pushed: bool) -> Option<Edge> {
    match (pushed, steps > 0) {
        (false, _) => None,
        (true, true) => Some(Edge::High),
        (true, false) => Some(Edge::Low),
    }
}

/// The level of the keyboard backlight.
fn keyboard_level(light: &keyboard::KeyboardLight) -> Level {
    Level {
        kind: Kind::Keyboard,
        percent: u16::from(light.percent()),
        muted: false,
    }
}

/// Does what `key` asks, on the worker thread, `steps` times for the volume and the
/// brightnesses (`VolumeUp`, `BrightnessUp` and `KeyboardUp`, negative steps going down).
/// Returns what to show.
fn act(
    controls: &Controls,
    worker: &mut Worker,
    (key, steps): (MediaKey, i32),
) -> Result<Option<Shown>, Problem> {
    let sound = |kind, sound: system_services::audio::Sound| Level {
        kind,
        percent: sound.volume,
        muted: sound.muted,
    };
    Ok(Some(match key {
        MediaKey::VolumeUp | MediaKey::VolumeDown => {
            let (level, pushed) = controls.mixer.step_volume(steps)?;
            (sound(Kind::Volume, level), edge(steps, pushed))
        }
        MediaKey::Mute => (
            sound(Kind::Volume, controls.mixer.toggle_mute(Device::Output)?),
            None,
        ),
        MediaKey::MicMute => (
            sound(
                Kind::Microphone,
                controls.mixer.toggle_mute(Device::Microphone)?,
            ),
            None,
        ),
        MediaKey::BrightnessUp | MediaKey::BrightnessDown => {
            let bus = worker.system.get()?;
            let root = &controls.backlights;
            let (percent, pushed) = zbus::block_on(backlight::step_from(&bus, root, steps))?;
            let level = Level {
                kind: Kind::Brightness,
                percent: u16::from(percent),
                muted: false,
            };
            (level, edge(steps, pushed))
        }
        MediaKey::KeyboardUp | MediaKey::KeyboardDown => {
            let bus = worker.system.get()?;
            let leds = &controls.leds;
            let (light, pushed) = zbus::block_on(keyboard::step_from(&bus, leds, steps))?;
            (keyboard_level(&light), edge(steps, pushed))
        }
        MediaKey::KeyboardToggle => {
            let bus = worker.system.get()?;
            let restore = worker.keyboard_off;
            let leds = &controls.leds;
            let (light, off) = zbus::block_on(keyboard::toggle(&bus, leds, restore))?;
            worker.keyboard_off = off.or(worker.keyboard_off);
            (keyboard_level(&light), None)
        }
        MediaKey::Player(command) => {
            let bus = worker.session.get()?;
            let player = zbus::block_on(mpris::send(&bus, command))?;
            tracing::debug!(player, ?command, "media key sent");
            return Ok(None);
        }
    }))
}

/// The worker: the keys in the order pressed, those waiting taken together (`merge`).
fn work(
    controls: Controls,
    keys: mpsc::Receiver<MediaKey>,
    busy: Arc<AtomicBool>,
    shown: channel::Sender<Shown>,
) {
    let bus = |address: &Option<String>, system| Bus {
        address: address.clone(),
        system,
        connection: None,
    };
    let mut worker = Worker {
        system: bus(&controls.system_bus, true),
        session: bus(&controls.session_bus, false),
        keyboard_off: None,
    };
    while let Ok(first) = keys.recv() {
        busy.store(true, Ordering::SeqCst);
        let waiting: Vec<MediaKey> = std::iter::once(first).chain(keys.try_iter()).collect();
        for key in merge(&waiting) {
            match act(&controls, &mut worker, key) {
                Ok(Some(change)) => {
                    if shown.send(change).is_err() {
                        return;
                    }
                }
                Ok(None) => {}
                Err(problem) => tracing::warn!(?key, "media key: {problem}"),
            }
        }
        busy.store(false, Ordering::SeqCst);
    }
}

impl State {
    /// Takes the media keys from now on, through `controls`. The real session calls it; nested
    /// mode leaves them to the host session.
    pub fn enable_media_keys(&mut self, controls: Controls) {
        let (keys, worker_keys) = mpsc::channel();
        let busy = Arc::new(AtomicBool::new(false));
        let worker_busy = busy.clone();
        let (shown, levels) = channel::channel();
        let inserted = self.loop_handle.insert_source(levels, |event, _, state| {
            if let Event::Msg((level, against)) = event {
                state.show_osd(level, against);
            }
        });
        if let Err(err) = inserted {
            tracing::warn!("media keys left to the clients: {err}");
            return;
        }
        let spawned = std::thread::Builder::new()
            .name(String::from("media-keys"))
            .spawn(move || work(controls, worker_keys, worker_busy, shown));
        if let Err(err) = spawned {
            tracing::warn!("media keys left to the clients: {err}");
            return;
        }
        self.media_keys = Some(MediaKeys {
            worker: keys,
            busy,
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
        // A press always goes: the keys of a hand never pile up.
        keys.busy.store(true, Ordering::SeqCst);
        let _ = keys.worker.send(key);
        if !key.repeats() {
            return;
        }
        let (worker, busy) = (keys.worker.clone(), keys.busy.clone());
        let timer = Timer::from_duration(REPEAT_DELAY);
        let token = self.loop_handle.insert_source(timer, move |_, _, _| {
            // A repeat only goes when the worker has done the keys before it.
            if busy.swap(true, Ordering::SeqCst) {
                return TimeoutAction::ToDuration(REPEAT_INTERVAL);
            }
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
        assert_eq!(
            MediaKey::of(Keysym::XF86_KbdLightOnOff),
            Some(MediaKey::KeyboardToggle)
        );
        assert!(MediaKey::KeyboardUp.repeats());
        assert!(!MediaKey::KeyboardToggle.repeats());
        assert!(REPEAT_INTERVAL < REPEAT_DELAY);
    }

    #[test]
    fn the_steps_waiting_are_set_at_once() {
        use MediaKey::*;
        let play = Player(mpris::Command::PlayPause);
        let keys = [
            VolumeUp,
            VolumeUp,
            VolumeDown,
            VolumeUp,
            Mute,
            BrightnessDown,
            BrightnessDown,
            KeyboardUp,
            KeyboardDown,
            KeyboardDown,
        ];
        let merged = merge(&keys);
        let want = [
            (VolumeUp, 2),
            (Mute, 0),
            (BrightnessUp, -2),
            (KeyboardUp, -1),
        ];
        assert_eq!(merged, want);
        // Steps that cancel out do nothing; other keys stay in their place.
        assert_eq!(merge(&[VolumeUp, VolumeDown, play]), [(play, 0)]);
        assert!(merge(&[]).is_empty());
    }
}
