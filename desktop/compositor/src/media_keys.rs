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
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
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

/// Delay before a held key repeats, and its pace, by default: a media key repeats as the
/// keyboard setting says (`State::key_repeat`).
pub const REPEAT_DELAY: Duration = Duration::from_millis(crate::KEY_REPEAT_DELAY as u64);
pub const REPEAT_INTERVAL: Duration = Duration::from_millis(1000 / crate::KEY_REPEAT_RATE as u64);
/// Repeats of a held key kept while the worker is at work, or while the event loop came late,
/// at most (lot Q-46): they go together once it is free, merged into one step (`merge`). So the
/// steps of a hold follow how long the key is held, not how often the loop could run, and a
/// sound server that hangs gives no great jump when it comes back.
pub const REPEATS_OWED: u32 = 8;
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
    /// Volume and brightness act on a locked session too; the players do not (lot D-25a).
    pub fn acts_while_locked(self) -> bool {
        !matches!(self, Self::Player(_))
    }

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

/// A repeat timer fired at `now`, the next repeat due at `next`: the repeats due since then, one
/// per `interval` passed, added to `owed` (`REPEATS_OWED` at most), and when the next one is due.
pub fn due(next: Instant, now: Instant, interval: Duration, owed: u32) -> (Instant, u32) {
    let late = now.saturating_duration_since(next).as_nanos();
    let passed = late / interval.as_nanos().max(1);
    let count = u32::try_from(passed).unwrap_or(u32::MAX).saturating_add(1);
    // Very late (the machine slept, the loop stalled): the pace starts again from now.
    let next = match count {
        c if c > REPEATS_OWED => now + interval,
        c => next + interval.saturating_mul(c),
    };
    (next, owed.saturating_add(count).min(REPEATS_OWED))
}

/// The keys at work: the worker thread, whether it is at work, and the key held.
pub struct MediaKeys {
    worker: mpsc::Sender<MediaKey>,
    /// Set from a key sent until the worker has done everything it was given: a held key only
    /// repeats when it is not set, so that a slow `pactl` never piles keys up.
    busy: Arc<AtomicBool>,
    /// The key held, its repeat timer, and the repeats it owes (`REPEATS_OWED`): sent when the
    /// key is released, so that a hold gives its steps even if the worker was at work all along.
    held: Option<(Keycode, RegistrationToken, MediaKey, Arc<AtomicU32>)>,
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
        // A connection closed (the bus restarted) is made again: the keys never stay dead.
        if let Some(connection) = self.connection.as_ref().filter(|c| !c.is_closed()) {
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

    /// The worker of the media keys has done everything it was given (the tests).
    pub fn media_keys_idle(&self) -> bool {
        self.media_keys
            .as_ref()
            .is_none_or(|keys| !keys.busy.load(Ordering::SeqCst))
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
        // The repeat of the keyboard setting, as the other keys.
        let (delay, interval) = self.key_repeat();
        let timer = Timer::from_duration(delay);
        // When the next repeat is due, and the repeats due not sent yet.
        let mut next = Instant::now() + delay;
        let owing = Arc::new(AtomicU32::new(0));
        let owed_here = owing.clone();
        let token = self.loop_handle.insert_source(timer, move |_, _, _| {
            let now = Instant::now();
            // Every interval that passed counts, however late the loop runs this.
            let owed;
            (next, owed) = due(next, now, interval, owed_here.load(Ordering::SeqCst));
            owed_here.store(owed, Ordering::SeqCst);
            let wait = TimeoutAction::ToDuration(next.saturating_duration_since(now));
            // A repeat only goes when the worker has done the keys before it; meanwhile the
            // repeats are kept, and they go together.
            if busy.swap(true, Ordering::SeqCst) {
                return wait;
            }
            for _ in 0..owed_here.swap(0, Ordering::SeqCst) {
                if worker.send(key).is_err() {
                    return TimeoutAction::Drop;
                }
            }
            wait
        });
        match token {
            Ok(token) => {
                if let Some(keys) = &mut self.media_keys {
                    keys.held = Some((code, token, key, owing));
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
        if let Some((held, token, key, owing)) = keys.held.take() {
            if code.is_some_and(|code| code != held) {
                keys.held = Some((held, token, key, owing));
                return;
            }
            self.loop_handle.remove(token);
            // The repeats due while the worker was at work: they go now, merged into one step.
            let owed = owing.swap(0, Ordering::SeqCst);
            if owed > 0 {
                keys.busy.store(true, Ordering::SeqCst);
                for _ in 0..owed {
                    let _ = keys.worker.send(key);
                }
            }
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
        // Q-46: a timer that fires late owes every interval that passed, a few at most.
        let start = Instant::now();
        let ms = Duration::from_millis;
        assert_eq!(due(start, start, ms(40), 0), (start + ms(40), 1));
        let (next, owed) = due(start, start + ms(130), ms(40), 0);
        assert_eq!((next, owed), (start + ms(160), 4));
        assert_eq!(due(start, start + ms(10_000), ms(40), 0).1, REPEATS_OWED);
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
