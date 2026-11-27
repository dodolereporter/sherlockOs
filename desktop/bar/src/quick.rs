//! The quick settings of the control centre (Panels board): Wi-Fi, Bluetooth, Do not disturb,
//! game mode, volume, brightness, keyboard backlight, graphics card, energy, and the session.
//! Their state, their messages, the calls to the services (`system`), and their view. While
//! the panel is open, the volume and the brightnesses follow what changes elsewhere (`watch`,
//! lot D-35d), but for a slider held under the pointer.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::futures::stream::{self, BoxStream};
use iced::futures::{Future, StreamExt};
use iced::widget::{Column, column, container, row, space, text};
use iced::{Alignment, Element, Length, Padding, Task};
use launcher::source::Power as Command;
use launcher::system::{NoSession, Session};
use notifications_config::NotificationsConfig;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::motion::Fade;
use sherlock_ui::panels::{self, Glyph, text as words};
use sherlock_ui::scroll::scroll;
use sherlock_ui::{color, metrics, typography};

use crate::source::battery::{Battery, Charge};
use crate::system::audio::{Output, Pactl};
use crate::system::backlight::{self, Backlight};
use crate::system::bluetooth::{self, Bluetooth};
use crate::system::keyboard::{self, KeyboardLight};
use crate::system::network::{self, Network, Outcome, Security, Wifi};
use crate::system::power::{self, Power, Profile};
use crate::system::watch;
use crate::system::{self, Problem};
use crate::unfold::Unfold;

mod devices;

/// A value read from a service.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Load<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(Problem),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }

    pub fn from_result(result: system::Result<T>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(problem) => Self::Failed(problem),
        }
    }
}

/// The service an action went to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Wifi,
    Bluetooth,
    Sound,
    Brightness,
    Energy,
    DoNotDisturb,
    Session,
    /// The keyboard backlight, when the machine has one.
    Keyboard,
}

/// One read of a service at a time: one asked meanwhile runs once the first ends, however
/// many were asked. A burst of signals never stacks reads on the bus.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reads {
    reading: u8,
    again: u8,
}

impl Reads {
    fn bit(service: Service) -> u8 {
        1 << service as u8
    }

    /// Whether to read `service` now; else it is read once the read under way ends.
    pub fn start(&mut self, service: Service) -> bool {
        let bit = Self::bit(service);
        if self.reading & bit != 0 {
            self.again |= bit;
            return false;
        }
        self.reading |= bit;
        true
    }

    /// The read of `service` ended: whether to read it again.
    pub fn end(&mut self, service: Service) -> bool {
        let bit = Self::bit(service);
        self.reading &= !bit;
        let again = self.again & bit != 0;
        self.again &= !bit;
        again
    }
}

/// How much a tile is on, from 0 to 1: a switch mixes its colors over `panels::TILE_FADE`, the
/// first state read shows at once.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Lit(Option<Fade>);

impl Lit {
    pub fn set(&mut self, on: bool, now: Duration) {
        let to = f32::from(u8::from(on));
        match &mut self.0 {
            Some(fade) if fade.target() != to => fade.retarget(now, to, panels::TILE_FADE),
            Some(_) => {}
            None => self.0 = Some(Fade::new(to, to, now, Duration::ZERO)),
        }
    }

    pub fn value(&self, now: Duration) -> f32 {
        self.0.map_or(0.0, |fade| fade.value(now))
    }

    pub fn running(&self, now: Duration) -> bool {
        self.0.is_some_and(|fade| !fade.is_done(now))
    }
}

/// The page of the panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Main,
    /// The Wi-Fi networks around.
    Networks,
    /// The Bluetooth devices, paired and found (`devices.rs`).
    Devices,
}

/// What runs the session commands: the launcher's.
#[derive(Clone)]
pub struct Commands(pub Arc<dyn Session>);

impl fmt::Debug for Commands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Commands")
    }
}

impl Default for Commands {
    fn default() -> Self {
        Self(Arc::new(NoSession))
    }
}

/// A slider value sent to its service one call at a time: the latest one wins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Throttle {
    busy: bool,
    wanted: Option<u8>,
}

impl Throttle {
    /// A call is under way.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The value to send now, if no call is under way.
    pub fn want(&mut self, value: u8) -> Option<u8> {
        if self.busy {
            self.wanted = Some(value);
            None
        } else {
            self.busy = true;
            Some(value)
        }
    }

    /// The call ended: the value to send next, if one came meanwhile.
    pub fn done(&mut self) -> Option<u8> {
        match self.wanted.take() {
            Some(value) => Some(value),
            None => {
                self.busy = false;
                None
            }
        }
    }
}

/// A password being typed (shared with the lock screen).
pub use system_services::secret::Secret;

/// What the sound server is asked, one `pactl` at a time: the latest volume and the latest mute
/// state wait while a call is under way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SoundQueue {
    busy: bool,
    volume: Option<u8>,
    mute: Option<bool>,
}

/// A call to the sound server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundCall {
    Volume(u8),
    Mute(bool),
}

impl SoundQueue {
    /// A call is under way.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// `call` now, if no call is under way; else it waits, replacing one of its kind.
    pub fn push(&mut self, call: SoundCall) -> Option<SoundCall> {
        if !self.busy {
            self.busy = true;
            return Some(call);
        }
        match call {
            SoundCall::Volume(value) => self.volume = Some(value),
            SoundCall::Mute(muted) => self.mute = Some(muted),
        }
        None
    }

    /// The call ended: the next one waiting, the mute state first.
    pub fn done(&mut self) -> Option<SoundCall> {
        let next = match (self.mute.take(), self.volume.take()) {
            (Some(muted), volume) => {
                self.volume = volume;
                Some(SoundCall::Mute(muted))
            }
            (None, Some(value)) => Some(SoundCall::Volume(value)),
            (None, None) => None,
        };
        self.busy = next.is_some();
        next
    }
}

#[derive(Debug, Default)]
pub struct Quick {
    pub wifi: Load<Wifi>,
    pub bluetooth: Load<Bluetooth>,
    pub sound: Load<Output>,
    pub brightness: Load<Backlight>,
    pub keyboard: Load<KeyboardLight>,
    pub energy: Load<Power>,
    pub do_not_disturb: Option<bool>,
    pub page: Page,
    /// The network chosen that needs its password, and the password typed.
    pub joining: Option<Network>,
    pub password: Secret,
    /// The network being joined, until its activation ends (`network::ACTIVATION_LIMIT` at
    /// most).
    pub connecting: Option<String>,
    /// The network whose last join ended badly, and how: said on its line and on the tile.
    pub refused: Option<(String, Outcome)>,
    /// When the password field started shaking: the password was wrong.
    pub shaken: Option<Duration>,
    /// The password typed shows in clear (the eye of the field).
    pub reveal: bool,
    /// « Autre réseau… » open: the name and the protection of a hidden network.
    pub other: Option<(String, Security)>,
    /// A saved network whose « Oublier ce réseau » waits for its confirmation.
    pub forgetting_network: Option<String>,
    /// A session command waiting for its second click, and when it was armed.
    pub armed: Option<(Command, Instant)>,
    /// The last action that failed.
    pub failure: Option<(Service, Problem)>,
    pub commands: Commands,
    /// `pactl`, the Do not disturb file and the backlights: the real ones but in tests.
    pub pactl: Pactl,
    pub settings: Option<PathBuf>,
    pub sysfs: PathBuf,
    pub leds: PathBuf,
    sound_calls: SoundQueue,
    light: Throttle,
    keyboard_calls: Throttle,
    /// The slider held under the pointer: a change made elsewhere does not move it.
    pub held: Option<Service>,
    pub reads: Reads,
    /// The instant of the animations (`animated::clock`), given by the bar.
    pub now: Duration,
    /// The Wi-Fi, Bluetooth and Do not disturb tiles.
    pub lit: [Lit; 3],
    /// The page coming in, and whether it comes from the right.
    pub page_motion: Unfold,
    page_forward: bool,
    /// The page the one shown replaced: it slides out while the new one comes in.
    page_from: Page,
    /// The Bluetooth device a connection, a disconnection or a pairing is under way for.
    pub device_busy: Option<String>,
    /// The question of the BlueZ agent while a device pairs.
    pub asked: Option<(bluetooth::Asked, bluetooth::Reply)>,
    /// A paired device whose « Oublier » waits for its confirmation.
    pub forgetting: Option<String>,
    /// The adapter a scan runs on while the device list is open.
    pub scanning: Option<String>,
    /// The pairing under way, stopped if the panel closes.
    pub pairing: Option<iced::task::Handle>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Wifi(Load<Wifi>),
    Bluetooth(Load<Bluetooth>),
    Sound(Load<Output>),
    Brightness(Load<Backlight>),
    Keyboard(Load<KeyboardLight>),
    Energy(Load<Power>),
    ToggleWifi,
    ShowNetworks(bool),
    Join(Network),
    Password(Secret),
    SubmitPassword,
    /// The join of a network ended: joined, its password refused, failed or too long.
    Joined(Network, Result<Outcome, Problem>),
    /// The eye of the password field.
    Reveal,
    /// « Autre réseau… » opened or closed, its name typed, its protection chosen.
    OtherNetwork(bool),
    OtherName(String),
    OtherSecurity(Security),
    /// « Oublier ce réseau », then its confirmation.
    ForgetNetwork(String),
    ConfirmForgetNetwork(bool),
    ToggleBluetooth,
    ShowDevices(bool),
    /// A device of the list: connected or disconnected when paired, else paired.
    Device(String),
    Pairing(bluetooth::Pairing),
    /// The connection or disconnection of a device ended.
    DeviceDone(Result<(), Problem>),
    /// « Confirmer » or « Annuler » under a question of the agent.
    Answer(bool),
    /// « Oublier » a paired device, then its confirmation.
    Forget(String),
    ConfirmForget(bool),
    ToggleDoNotDisturb,
    /// « Ne pas déranger » as the file says, read or written off the interface thread.
    DoNotDisturb(Result<bool, Problem>),
    Volume(u8),
    Mute,
    Light(u8),
    KeyboardLight(u8),
    /// The slider of a service let go.
    Release(Service),
    Profile(Profile),
    Session(Command),
    /// An action ended.
    Done(Service, Result<(), Problem>),
    /// A service says something changed: read it again.
    Changed(Service),
}

/// Runs `call` on the system bus in an iced task; `done` turns its result into a message.
fn on_system<T, F>(
    call: impl FnOnce(zbus::Connection) -> F + Send + 'static,
    done: impl Fn(system::Result<T>) -> Message + Send + 'static,
) -> Task<Message>
where
    T: Send + 'static,
    F: Future<Output = system::Result<T>> + Send,
{
    Task::perform(
        async move {
            let bus = system::system().await?;
            call(bus).await
        },
        done,
    )
}

/// An action on the system bus; `Done` when it ends.
fn act<F>(
    service: Service,
    call: impl FnOnce(zbus::Connection) -> F + Send + 'static,
) -> Task<Message>
where
    F: Future<Output = system::Result<()>> + Send,
{
    Task::perform(
        async move {
            let bus = system::system_for_actions().await?;
            call(bus).await
        },
        move |done| Message::Done(service, done),
    )
}

/// Whether the notifications file at `path` says « Ne pas déranger »; a file that cannot be
/// read says no.
fn read_do_not_disturb(path: &Path) -> bool {
    NotificationsConfig::load(path).is_ok_and(|(config, _)| config.do_not_disturb)
}

/// Turns « Ne pas déranger » over in the file at `path` and returns the new state. Two
/// toggles at once are made one after the other: neither is lost.
fn toggle_do_not_disturb(path: &Path) -> system::Result<bool> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _turn = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut config, _) = NotificationsConfig::load(path).unwrap_or_default();
    config.do_not_disturb = !config.do_not_disturb;
    config
        .save(path)
        .map_err(|error| Problem::Failed(error.to_string()))?;
    Ok(config.do_not_disturb)
}

/// Runs blocking `work` (a command, a file) on a thread of its own, never on iced's pool: a
/// sound server that hangs holds that thread only, for `pactl`'s time limit at most.
fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> system::Result<T> + Send + 'static,
    done: impl Fn(system::Result<T>) -> Message + Send + 'static,
) -> Task<Message> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(String::from("quick-settings"))
        .spawn(move || {
            let _ = sender.send(work());
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start a thread for the quick settings");
    }
    Task::perform(
        async move {
            receiver
                .await
                .unwrap_or_else(|_| Err(Problem::Failed(String::from("the call was lost"))))
        },
        done,
    )
}

impl Quick {
    /// The quick settings, the session commands run by `commands`.
    pub fn new(commands: Arc<dyn Session>) -> Self {
        Self {
            commands: Commands(commands),
            settings: notifications_config::path(),
            sysfs: PathBuf::from(backlight::SYSFS),
            leds: PathBuf::from(keyboard::LEDS),
            ..Self::default()
        }
    }

    /// Reads every service: when the panel opens.
    pub fn open(&mut self) -> Task<Message> {
        let stopped = self.close();
        self.page = Page::Main;
        self.armed = None;
        self.failure = None;
        self.refused = None;
        self.held = None;
        // The file is read off the interface thread: the tile shows its last state meanwhile.
        let do_not_disturb = match self.settings.clone() {
            Some(path) => blocking(
                move || Ok(read_do_not_disturb(&path)),
                Message::DoNotDisturb,
            ),
            None => Task::none(),
        };
        Task::batch([
            stopped,
            do_not_disturb,
            self.read(Service::Wifi),
            self.read(Service::Bluetooth),
            self.read(Service::Sound),
            self.read(Service::Brightness),
            self.read(Service::Keyboard),
            self.read(Service::Energy),
        ])
    }

    fn read(&mut self, service: Service) -> Task<Message> {
        if matches!(service, Service::DoNotDisturb | Service::Session) || !self.reads.start(service)
        {
            return Task::none();
        }
        match service {
            Service::Wifi => on_system(
                |bus| async move { network::read(&bus).await },
                |r| Message::Wifi(Load::from_result(r)),
            ),
            Service::Bluetooth => on_system(
                |bus| async move { bluetooth::read(&bus).await },
                |r| Message::Bluetooth(Load::from_result(r)),
            ),
            Service::Energy => on_system(
                |bus| async move { power::read(&bus).await },
                |r| Message::Energy(Load::from_result(r)),
            ),
            Service::Sound => {
                let pactl = self.pactl.clone();
                blocking(
                    move || pactl.read(),
                    |r| Message::Sound(Load::from_result(r)),
                )
            }
            Service::Brightness => {
                let root = self.sysfs.clone();
                blocking(
                    move || backlight::find(&root),
                    |r| Message::Brightness(Load::from_result(r)),
                )
            }
            Service::Keyboard => {
                let leds = self.leds.clone();
                on_system(
                    move |bus| async move { keyboard::read(&bus, &leds).await },
                    |r| Message::Keyboard(Load::from_result(r)),
                )
            }
            Service::DoNotDisturb | Service::Session => Task::none(),
        }
    }

    /// Frames are needed: a tile or a page moves.
    pub fn running(&self) -> bool {
        self.page_motion.running()
            || self.lit.iter().any(|lit| lit.running(self.now))
            || self
                .shaken
                .is_some_and(|at| self.now.saturating_sub(at) < sherlock_ui::lock::screen::SHAKE)
    }

    pub fn tick(&mut self, now: Duration) {
        self.now = now;
        self.page_motion.tick(now);
    }

    /// A tile switched by an action or by a read: its colors mix towards its new state. A
    /// service not read, or not there, starts from nothing: its first state shows at once.
    fn relight(&mut self) {
        let on = [
            self.wifi
                .ready()
                .map(|w| w.enabled && w.device.is_some() && !w.hardware_off),
            self.bluetooth.ready().map(|b| b.powered),
            self.do_not_disturb,
        ];
        for (lit, on) in self.lit.iter_mut().zip(on) {
            match on {
                Some(on) => lit.set(on, self.now),
                None => *lit = Lit::default(),
            }
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        self.relight();
        task
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        // Any other click cancels a session command waiting for its confirmation.
        let passive = matches!(
            message,
            Message::Session(_)
                | Message::Pairing(_)
                | Message::DeviceDone(_)
                | Message::Done(..)
                | Message::Joined(..)
                | Message::Changed(_)
                | Message::Wifi(_)
                | Message::Bluetooth(_)
                | Message::Sound(_)
                | Message::Brightness(_)
                | Message::Keyboard(_)
                | Message::Energy(_)
        );
        if !passive {
            self.armed = None;
        }
        match message {
            Message::Wifi(wifi) => {
                let joined = match (&self.connecting, &wifi) {
                    (Some(name), Load::Ready(read)) => {
                        read.connected().is_some_and(|n| &n.ssid == name)
                    }
                    _ => false,
                };
                if joined {
                    self.connecting = None;
                }
                self.wifi = wifi;
                return self.read_ended(Service::Wifi);
            }
            Message::Bluetooth(read) => {
                self.bluetooth = read;
                // Switched on while the list is open: the scan starts.
                let scan = if self.page == Page::Devices {
                    self.start_scan()
                } else {
                    Task::none()
                };
                return Task::batch([scan, self.read_ended(Service::Bluetooth)]);
            }
            Message::Sound(mut read) => {
                // A slider held, or a value on its way to the server, stays where it is.
                if let (Load::Ready(new), Some(old)) = (&mut read, self.sound.ready())
                    && self.settling(Service::Sound)
                {
                    new.sound = old.sound;
                }
                self.sound = read;
                return self.read_ended(Service::Sound);
            }
            Message::Brightness(mut read) => {
                if let (Load::Ready(new), Some(old)) = (&mut read, self.brightness.ready())
                    && self.settling(Service::Brightness)
                {
                    new.brightness = old.brightness;
                }
                self.brightness = read;
                return self.read_ended(Service::Brightness);
            }
            Message::Keyboard(mut read) => {
                if let (Load::Ready(new), Some(old)) = (&mut read, self.keyboard.ready())
                    && self.settling(Service::Keyboard)
                {
                    new.brightness = old.brightness;
                }
                self.keyboard = read;
                return self.read_ended(Service::Keyboard);
            }
            Message::Energy(read) => {
                self.energy = read;
                return self.read_ended(Service::Energy);
            }
            Message::ToggleWifi => {
                if let Some(wifi) = self.wifi.ready() {
                    let on = !wifi.enabled;
                    return act(Service::Wifi, move |bus| async move {
                        network::enable(&bus, on).await
                    });
                }
            }
            Message::ShowNetworks(show) => {
                let page = if show { Page::Networks } else { Page::Main };
                self.turn_page(page, show);
                self.forget_typed();
                let device = self.wifi.ready().and_then(|w| w.device.clone());
                if let (true, Some(device)) = (show, device) {
                    // The networks found come later, with the signals of the device
                    // (`watch`): the list is read again then.
                    return on_system(
                        move |bus| async move { network::scan(&bus, &device).await },
                        |r| match r {
                            Ok(()) => Message::Changed(Service::Wifi),
                            Err(problem) => Message::Done(Service::Wifi, Err(problem)),
                        },
                    );
                }
            }
            Message::Join(chosen) => return self.join(chosen),
            Message::Password(typed) => self.password = typed,
            Message::SubmitPassword => return self.submit(),
            Message::Joined(network, result) => return self.joined(network, result),
            Message::Reveal => self.reveal = !self.reveal,
            Message::OtherNetwork(open) => {
                self.forget_typed();
                self.other = open.then(|| (String::new(), Security::Psk));
            }
            Message::OtherName(name) => {
                if let Some((typed, _)) = &mut self.other {
                    // A name has 32 bytes at most: the field takes no more.
                    *typed = name;
                    while typed.len() > network::MAX_SSID {
                        typed.pop();
                    }
                }
            }
            Message::OtherSecurity(security) => {
                if let Some((_, chosen)) = &mut self.other {
                    *chosen = security;
                }
            }
            Message::ForgetNetwork(ssid) => self.forgetting_network = Some(ssid),
            Message::ConfirmForgetNetwork(yes) => {
                if let (true, Some(ssid)) = (yes, self.forgetting_network.take()) {
                    if self.refused.as_ref().is_some_and(|(name, _)| *name == ssid) {
                        self.refused = None;
                    }
                    return act(Service::Wifi, move |bus| async move {
                        network::forget(&bus, &ssid).await
                    });
                }
                self.forgetting_network = None;
            }
            Message::ToggleBluetooth => {
                if let Some(read) = self.bluetooth.ready() {
                    let (adapter, on) = (read.adapter.clone(), !read.powered);
                    return act(Service::Bluetooth, move |bus| async move {
                        bluetooth::power(&bus, &adapter, on).await
                    });
                }
            }
            Message::ToggleDoNotDisturb => {
                if let Some(path) = self.settings.clone() {
                    return blocking(move || toggle_do_not_disturb(&path), Message::DoNotDisturb);
                }
            }
            Message::DoNotDisturb(done) => self.do_not_disturb_done(done),
            Message::Volume(value) => {
                self.held = Some(Service::Sound);
                if let Load::Ready(output) = &mut self.sound {
                    output.sound.volume = u16::from(value);
                }
                if let Some(call) = self.sound_calls.push(SoundCall::Volume(value)) {
                    return self.send_sound(call);
                }
            }
            Message::Mute => {
                if let Load::Ready(output) = &mut self.sound {
                    output.sound.muted = !output.sound.muted;
                    let call = SoundCall::Mute(output.sound.muted);
                    if let Some(call) = self.sound_calls.push(call) {
                        return self.send_sound(call);
                    }
                }
            }
            Message::Light(value) => {
                self.held = Some(Service::Brightness);
                if let Load::Ready(light) = &mut self.brightness {
                    light.brightness = light.raw(value);
                }
                if let Some(value) = self.light.want(value) {
                    return self.send_light(value);
                }
            }
            Message::KeyboardLight(value) => {
                self.held = Some(Service::Keyboard);
                if let Load::Ready(light) = &mut self.keyboard {
                    light.brightness = light.raw(value);
                }
                if let Some(value) = self.keyboard_calls.want(value) {
                    return self.send_keyboard(value);
                }
            }
            Message::Release(service) => {
                if self.held == Some(service) {
                    self.held = None;
                }
                // What changed elsewhere while it was held shows now.
                return self.read(service);
            }
            Message::Profile(profile) => {
                if let Load::Ready(state) = &self.energy {
                    let state = state.clone();
                    return act(Service::Energy, move |bus| async move {
                        power::choose(&bus, &state, profile).await
                    });
                }
            }
            Message::ShowDevices(show) => return self.show_devices(show),
            Message::Device(path) => return self.device(path),
            Message::Pairing(event) => return self.pairing(event),
            Message::DeviceDone(result) => return self.device_done(result),
            Message::Answer(yes) => return self.answer(yes),
            Message::Forget(path) => self.forgetting = Some(path),
            Message::ConfirmForget(yes) => return self.forget(yes),
            Message::Session(command) => self.session(command),
            Message::Done(service, done) => return self.done(service, done),
            Message::Changed(service) => return self.read(service),
        }
        Task::none()
    }

    /// A read ended: the service read once more if a change came meanwhile.
    fn read_ended(&mut self, service: Service) -> Task<Message> {
        if self.reads.end(service) {
            self.read(service)
        } else {
            Task::none()
        }
    }

    /// An action ended: its failure kept for the view, the next slider value sent, else the
    /// service read again.
    fn done(&mut self, service: Service, done: Result<(), Problem>) -> Task<Message> {
        match done {
            Err(problem) => {
                tracing::warn!(?service, %problem, "quick setting refused");
                self.failure = Some((service, problem));
                if service == Service::Wifi {
                    self.connecting = None;
                }
            }
            Ok(()) => {
                if self.failure.as_ref().is_some_and(|(s, _)| *s == service) {
                    self.failure = None;
                }
            }
        }
        let next = match service {
            Service::Sound => self.sound_calls.done().map(|c| self.send_sound(c)),
            Service::Brightness => self.light.done().map(|v| self.send_light(v)),
            Service::Keyboard => self.keyboard_calls.done().map(|v| self.send_keyboard(v)),
            _ => None,
        };
        match next {
            Some(task) => task,
            // A slider read again in the middle of a drag would jump back.
            None => self.read(service),
        }
    }

    fn send_sound(&self, call: SoundCall) -> Task<Message> {
        let pactl = self.pactl.clone();
        blocking(
            move || match call {
                SoundCall::Volume(value) => pactl.set_volume(value),
                SoundCall::Mute(muted) => pactl.set_mute(muted),
            },
            |r| Message::Done(Service::Sound, r),
        )
    }

    /// The panel closed, whichever way: the password typed is wiped, the network chosen
    /// forgotten, a scan for Bluetooth devices stopped, a question of the agent cancelled.
    pub fn close(&mut self) -> Task<Message> {
        self.forget_typed();
        self.forgetting = None;
        self.stop_pairing();
        self.stop_scan()
    }

    /// A read of `service` must not move its slider: it is held, or a value is on its way.
    fn settling(&self, service: Service) -> bool {
        self.held == Some(service)
            || match service {
                Service::Sound => self.sound_calls.busy(),
                Service::Brightness => self.light.busy(),
                Service::Keyboard => self.keyboard_calls.busy(),
                _ => false,
            }
    }

    fn send_keyboard(&self, value: u8) -> Task<Message> {
        let Some(light) = self.keyboard.ready().cloned() else {
            return Task::none();
        };
        act(Service::Keyboard, move |bus| async move {
            keyboard::set(&bus, &light, light.raw(value)).await
        })
    }

    fn send_light(&self, value: u8) -> Task<Message> {
        let Some(light) = self.brightness.ready().cloned() else {
            return Task::none();
        };
        act(Service::Brightness, move |bus| async move {
            backlight::set(&bus, &light, value).await
        })
    }

    /// What was typed for a network goes: its password (wiped), a hidden network's form, a
    /// pending « Oublier ce réseau ».
    fn forget_typed(&mut self) {
        self.password.wipe();
        self.joining = None;
        self.other = None;
        self.reveal = false;
        self.shaken = None;
        self.forgetting_network = None;
    }

    /// A network of the list: joined at once when it is open or saved; else its password is
    /// asked for below it.
    fn join(&mut self, chosen: Network) -> Task<Message> {
        if chosen.active || self.connecting.is_some() {
            return Task::none();
        }
        if chosen.security == Security::Open || chosen.saved {
            return self.connect(chosen, None);
        }
        self.forget_typed();
        self.joining = Some(chosen);
        Task::none()
    }

    /// « Rejoindre », or Enter in a password field: the network asked for, or the hidden one.
    fn submit(&mut self) -> Task<Message> {
        if let Some(chosen) = self.joining.clone()
            && !self.password.is_empty()
        {
            // Moved out: the field empties, the secret is wiped once sent.
            let password = std::mem::take(&mut self.password);
            return self.connect(chosen, Some(password));
        }
        if let Some((name, security)) = self.other.clone()
            && !name.is_empty()
            && (security == Security::Open || !self.password.is_empty())
        {
            let password = (security != Security::Open).then(|| std::mem::take(&mut self.password));
            return self.connect(Network::hidden(&name, security), password);
        }
        Task::none()
    }

    /// Joins `chosen` through the bus for actions (polkit may ask for a password, for
    /// `ACTION_TIMEOUT`), and follows the activation to its end.
    fn connect(&mut self, chosen: Network, password: Option<Secret>) -> Task<Message> {
        let Some(device) = self.wifi.ready().and_then(|w| w.device.clone()) else {
            return Task::none();
        };
        self.forget_typed();
        self.refused = None;
        if self
            .failure
            .as_ref()
            .is_some_and(|(s, _)| *s == Service::Wifi)
        {
            self.failure = None;
        }
        self.connecting = Some(chosen.ssid.clone());
        let network = chosen.clone();
        Task::perform(
            async move {
                let bus = system::system_for_actions().await?;
                let password = password.as_ref().map(Secret::as_str);
                network::join(&bus, &device, &chosen, password).await
            },
            move |result| Message::Joined(network.clone(), result),
        )
    }

    /// A join ended. A wrong password is asked again under its network's line (or in the
    /// hidden network's form), the field shaking.
    fn joined(&mut self, network: Network, result: Result<Outcome, Problem>) -> Task<Message> {
        if self.connecting.as_deref() == Some(network.ssid.as_str()) {
            self.connecting = None;
        }
        match result {
            Ok(Outcome::Joined) => {}
            Ok(Outcome::WrongPassword) if network.security != Security::Open => {
                tracing::info!("the Wi-Fi password was refused");
                self.forget_typed();
                if network.path == network::ANY_POINT {
                    self.other = Some((network.ssid.clone(), network.security));
                } else {
                    self.joining = Some(network.clone());
                }
                self.shaken = Some(self.now);
                self.refused = Some((network.ssid, Outcome::WrongPassword));
            }
            Ok(outcome) => {
                tracing::warn!(?outcome, "the Wi-Fi network was not joined");
                self.refused = Some((network.ssid, outcome));
            }
            Err(problem) => return self.done(Service::Wifi, Err(problem)),
        }
        self.read(Service::Wifi)
    }

    /// « Ne pas déranger » read or written: the tile follows, or « Échec » under it.
    fn do_not_disturb_done(&mut self, done: Result<bool, Problem>) {
        match done {
            Ok(on) => {
                self.do_not_disturb = Some(on);
                if self
                    .failure
                    .as_ref()
                    .is_some_and(|(s, _)| *s == Service::DoNotDisturb)
                {
                    self.failure = None;
                }
            }
            Err(problem) => {
                tracing::warn!(%problem, "cannot write Do not disturb");
                self.failure = Some((Service::DoNotDisturb, problem));
            }
        }
    }

    /// The launcher's rules: locking runs at once; logging out, restarting and powering off
    /// want a second click, at least `CONFIRM_DELAY` later, so that a double click only arms.
    fn session(&mut self, command: Command) {
        if command.needs_confirmation() {
            let delay = sherlock_ui::launcher::CONFIRM_DELAY;
            match self.armed {
                Some((armed, at)) if armed == command && at.elapsed() >= delay => {}
                Some((armed, _)) if armed == command => return,
                _ => {
                    self.armed = Some((command, Instant::now()));
                    return;
                }
            }
        }
        self.run_now(command);
    }

    /// Runs `command` at once: what was confirmed elsewhere, as in the Sherlock menu (D-36d).
    pub fn run_now(&mut self, command: Command) {
        self.armed = None;
        match self.commands.0.run(command) {
            Ok(()) => {
                tracing::info!(?command, "session command");
                self.failure = None;
            }
            Err(error) => {
                tracing::error!(%error, "cannot run the session command");
                // logind or polkit said no: « Refusé » under the buttons.
                let problem = match error {
                    launcher::launch::LaunchError::Refused { .. } => {
                        Problem::Refused(error.to_string())
                    }
                    _ => Problem::Failed(error.to_string()),
                };
                self.failure = Some((Service::Session, problem));
            }
        }
    }
}

/// The changes the services announce while the control centre is open: on the system bus,
/// NetworkManager's main object and the states of its devices, BlueZ's adapters,
/// power-profiles-daemon and UPower's keyboard backlight (nothing when the bus cannot be
/// reached, logged once); the sound and the screen brightness changed elsewhere (`local`).
pub fn watch() -> BoxStream<'static, Service> {
    let bus = stream::once(async {
        match changes().await {
            Ok(changes) => changes,
            Err(problem) => {
                tracing::warn!(%problem, "the quick settings will not follow the services");
                stream::empty().boxed()
            }
        }
    })
    .flatten();
    let local = local(Pactl::default().program, PathBuf::from(backlight::SYSFS));
    stream::select(bus, local).boxed()
}

/// The sound (`pactl subscribe`, or a stand-in) and the brightness of the backlight under
/// `sysfs` changed elsewhere, a key or another application, for as long as the stream lives
/// (lot D-35d, `system_services::watch`).
pub fn local(pactl: std::ffi::OsString, sysfs: PathBuf) -> BoxStream<'static, Service> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let tell = |service| {
        let sender = sender.clone();
        move || sender.unbounded_send(service).is_ok()
    };
    let alive = || {
        let sender = sender.clone();
        move || !sender.is_closed()
    };
    let sound = watch::sound(pactl, tell(Service::Sound), alive());
    if let Err(error) = sound {
        tracing::warn!(%error, "the volume will not follow the sound");
    }
    let light = watch::brightness(sysfs, tell(Service::Brightness), alive());
    if let Err(error) = light {
        tracing::warn!(%error, "the brightness will not follow the screen");
    }
    receiver.boxed()
}

/// The streams of `watch`, on the system bus.
pub async fn changes() -> system::Result<BoxStream<'static, Service>> {
    let bus = system::system().await?;
    changes_on(&bus).await
}

/// The streams of `watch`, on `bus`.
pub async fn changes_on(bus: &zbus::Connection) -> system::Result<BoxStream<'static, Service>> {
    // The path keeps the services apart: zbus does not filter a well-known sender name on its
    // side, and the system connection is shared (see `live::properties`).
    let properties =
        |sender: &'static str, namespace: &'static str| -> zbus::Result<zbus::MatchRule<'static>> {
            Ok(zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(sender)?
                .path_namespace(namespace)?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .build())
        };
    let mut rules = vec![
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(network::SERVICE)?
                .path(network::PATH)?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .build(),
            Service::Wifi,
        ),
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(network::SERVICE)?
                .interface(network::DEVICE)?
                .member("StateChanged")?
                .build(),
            Service::Wifi,
        ),
        // The end of a scan: `LastScan` and `AccessPoints` of the Wi-Fi device, and the access
        // points that come and go.
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(network::SERVICE)?
                .path_namespace(network::PATH)?
                .interface("org.freedesktop.DBus.Properties")?
                .member("PropertiesChanged")?
                .arg(0, network::WIRELESS)?
                .build(),
            Service::Wifi,
        ),
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(network::SERVICE)?
                .interface(network::WIRELESS)?
                .member("AccessPointAdded")?
                .build(),
            Service::Wifi,
        ),
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(network::SERVICE)?
                .interface(network::WIRELESS)?
                .member("AccessPointRemoved")?
                .build(),
            Service::Wifi,
        ),
        (
            properties(bluetooth::SERVICE, bluetooth::PATH)?,
            Service::Bluetooth,
        ),
        // Devices found by a scan, and forgotten.
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(bluetooth::SERVICE)?
                .path("/")?
                .interface("org.freedesktop.DBus.ObjectManager")?
                .member("InterfacesAdded")?
                .build(),
            Service::Bluetooth,
        ),
        (
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(bluetooth::SERVICE)?
                .path("/")?
                .interface("org.freedesktop.DBus.ObjectManager")?
                .member("InterfacesRemoved")?
                .build(),
            Service::Bluetooth,
        ),
    ];
    for (name, path, _) in power::NAMES {
        rules.push((properties(name, path)?, Service::Energy));
    }
    // UPower tells of a change of the keyboard backlight, from a key or elsewhere.
    rules.push((
        zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(keyboard::UPOWER)?
            .interface(keyboard::INTERFACE)?
            .member("BrightnessChanged")?
            .build(),
        Service::Keyboard,
    ));
    let mut streams = Vec::new();
    for (rule, service) in rules {
        let messages = zbus::MessageStream::for_match_rule(rule, bus, None).await?;
        streams.push(messages.map(move |_| service).boxed());
    }
    let all = stream::select_all(streams).boxed();
    Ok(crate::bus::coalesce(all, crate::bus::SIGNAL_GAP))
}

// The view.

impl Quick {
    /// The panel: the main page or the Wi-Fi networks, sliding in when it changes.
    /// `battery` goes next to « Énergie ».
    pub fn view<'a>(&'a self, user: &'a str, battery: Option<Battery>) -> Element<'a, Message> {
        let page = move |page: Page| match page {
            Page::Main => self.view_main(user, battery),
            Page::Networks => self.view_networks(),
            Page::Devices => self.view_devices(),
        };
        let forward = self.page_forward;
        let coming = self.page_motion.page(forward, || page(self.page));
        // The page it replaces slides out under it; at rest, an empty layer in its place keeps
        // the tree of the page shown (its scroll, its field) from one frame to the next.
        let leaving = if self.page_motion.running() && self.page_from != self.page {
            self.page_motion
                .leaving_page(forward, || page(self.page_from))
        } else {
            // A stack takes the size of its first layer.
            space().width(Length::Fill).height(Length::Fill).into()
        };
        iced::widget::stack![leaving, coming].into()
    }

    /// Shows `page`, sliding in from the right (`forward`) or from the left, the page shown
    /// sliding out the other way.
    pub(crate) fn turn_page(&mut self, page: Page, forward: bool) {
        if page != self.page {
            self.page_motion = Unfold::opening(self.now);
            self.page_forward = forward;
            self.page_from = self.page;
        }
        self.page = page;
    }

    /// The short word that replaces the line of `service` after its last action failed.
    fn failed(&self, service: Service) -> Option<&'static str> {
        match &self.failure {
            Some((s, Problem::Refused(_))) if *s == service => Some(words::REFUSED),
            Some((s, _)) if *s == service => Some(words::FAILED),
            _ => None,
        }
    }

    fn view_main<'a>(&'a self, user: &'a str, battery: Option<Battery>) -> Element<'a, Message> {
        let tiles = column![
            row![self.wifi_tile(), self.bluetooth_tile()].spacing(panels::TILE_GAP),
            row![self.do_not_disturb_tile(), game_tile()].spacing(panels::TILE_GAP),
        ]
        .spacing(panels::TILE_GAP);
        let gpu = words::GPU_CHOICES
            .iter()
            .map(|label| (*label, false, None))
            .collect();
        column![
            tiles,
            self.volume_row(),
            self.lights(),
            group(
                words::GPU,
                Some(words::SOON.to_owned()),
                panels::segmented(gpu)
            ),
            self.energy_group(battery),
            space::vertical(),
            self.footer(user),
        ]
        .spacing(metrics::GAP_SECTION)
        .height(Length::Fill)
        .into()
    }

    fn wifi_tile(&self) -> Element<'_, Message> {
        let (on, line, enabled) = match &self.wifi {
            Load::Loading => (false, words::LOADING.to_owned(), false),
            Load::Failed(_) => (false, words::UNAVAILABLE.to_owned(), false),
            Load::Ready(wifi) if wifi.device.is_none() => {
                (false, words::NO_WIFI_DEVICE.to_owned(), false)
            }
            Load::Ready(wifi) if wifi.hardware_off => (false, words::BLOCKED.to_owned(), false),
            Load::Ready(wifi) => {
                let refused = self.refused.as_ref().map(|(_, outcome)| refusal(*outcome));
                let line = match (wifi.enabled, &self.connecting, wifi.connected(), refused) {
                    (false, _, _, _) => words::OFF.to_owned(),
                    (true, Some(_), _, _) => words::CONNECTING.to_owned(),
                    (true, None, Some(network), _) => network.ssid.clone(),
                    (true, None, None, Some(refused)) => refused.to_owned(),
                    (true, None, None, None) => words::NOT_CONNECTED.to_owned(),
                };
                (wifi.enabled, line, true)
            }
        };
        let line = self.failed(Service::Wifi).map_or(line, str::to_owned);
        let more = (enabled && on).then_some((words::NETWORKS, Message::ShowNetworks(true)));
        panels::quick_tile(
            Glyph::Wifi,
            words::WIFI,
            line,
            self.lit[0].value(self.now),
            enabled.then_some(Message::ToggleWifi),
            more,
        )
    }

    fn bluetooth_tile(&self) -> Element<'_, Message> {
        let (line, enabled) = match &self.bluetooth {
            Load::Loading => (words::LOADING.to_owned(), false),
            Load::Failed(_) => (words::UNAVAILABLE.to_owned(), false),
            Load::Ready(read) if !read.powered => (words::OFF.to_owned(), true),
            Load::Ready(read) if read.connected > 0 => (words::devices(read.connected), true),
            Load::Ready(_) => (words::ON.to_owned(), true),
        };
        let line = self.failed(Service::Bluetooth).map_or(line, str::to_owned);
        let more = enabled.then_some((words::BLUETOOTH_DEVICES, Message::ShowDevices(true)));
        panels::quick_tile(
            Glyph::Bluetooth,
            words::BLUETOOTH,
            line,
            self.lit[1].value(self.now),
            enabled.then_some(Message::ToggleBluetooth),
            more,
        )
    }

    fn do_not_disturb_tile(&self) -> Element<'_, Message> {
        let on = self.do_not_disturb == Some(true);
        let line = match self.failed(Service::DoNotDisturb) {
            Some(failed) => failed,
            None if on => words::ON,
            None => words::OFF,
        };
        let enabled = self.settings.is_some();
        panels::quick_tile(
            Glyph::Moon,
            words::DO_NOT_DISTURB,
            line.to_owned(),
            self.lit[2].value(self.now),
            enabled.then_some(Message::ToggleDoNotDisturb),
            None,
        )
    }

    fn volume_row(&self) -> Element<'_, Message> {
        let (muted, volume, line) = match &self.sound {
            Load::Ready(output) => (
                output.sound.muted,
                Some(u8::try_from(output.sound.volume.min(100)).unwrap_or(100)),
                output.name.clone(),
            ),
            Load::Loading => (false, None, words::LOADING.to_owned()),
            Load::Failed(_) => (false, None, words::NO_OUTPUT.to_owned()),
        };
        let line = self.failed(Service::Sound).map_or(line, str::to_owned);
        let (glyph, label) = if muted {
            (Glyph::Muted, words::UNMUTE)
        } else {
            (Glyph::Volume, words::MUTE)
        };
        let mute = volume.map(|_| Message::Mute);
        let on_change: Option<fn(u8) -> Message> = volume.map(|_| Message::Volume as fn(u8) -> _);
        let control = panels::held_slider_row(
            panels::glyph_button(glyph, label, mute),
            volume.unwrap_or(0),
            on_change,
            Some(Message::Release(Service::Sound)),
        );
        let caption = ui::ellipsis(line)
            .size(typography::SMALL_SIZE)
            .style(panels::secondary_text);
        let caption = container(caption).padding(Padding {
            left: panels::SLIDER_INSET,
            ..Padding::ZERO
        });
        column![control, caption]
            .spacing(panels::CAPTION_GAP)
            .into()
    }

    fn brightness_row(&self) -> Element<'_, Message> {
        let value = self.brightness.ready().map(Backlight::percent);
        let icon = container(panels::glyph(Glyph::Sun, color::palette().text_secondary))
            .center(panels::ROW_BUTTON);
        let on_change: Option<fn(u8) -> Message> = value.map(|_| Message::Light as fn(u8) -> _);
        let release = Some(Message::Release(Service::Brightness));
        panels::held_slider_row(icon, value.unwrap_or(0), on_change, release)
    }

    /// Whether the machine has a keyboard backlight, as last read: the panel opens with room
    /// for its slider or without.
    pub fn has_keyboard(&self) -> bool {
        self.keyboard.ready().is_some()
    }

    /// The brightness, and under it the keyboard backlight when the machine has one and the
    /// panel opened with room for it (a backlight found while it is open shows the next time).
    fn lights(&self) -> Element<'_, Message> {
        let row = crate::panel::keyboard_room()
            .then(|| self.keyboard_row())
            .flatten();
        match row {
            Some(keyboard) => column![self.brightness_row(), keyboard]
                .spacing(metrics::GAP_SECTION)
                .into(),
            None => self.brightness_row(),
        }
    }

    fn keyboard_row(&self) -> Option<Element<'_, Message>> {
        let light = self.keyboard.ready()?;
        let glyph = panels::glyph(Glyph::Keyboard, color::palette().text_secondary);
        let icon = container(glyph).center(panels::ROW_BUTTON);
        let on_change = Some(Message::KeyboardLight as fn(u8) -> _);
        let release = Some(Message::Release(Service::Keyboard));
        Some(panels::held_slider_row(
            icon,
            light.percent(),
            on_change,
            release,
        ))
    }

    /// « Énergie »: the profiles, and the battery with its time left on the right of the
    /// label, but when the service failed.
    fn energy_group(&self, battery: Option<Battery>) -> Element<'_, Message> {
        let (state, note) = match &self.energy {
            Load::Ready(state) => (Some(state), self.failed(Service::Energy)),
            Load::Loading => (None, None),
            Load::Failed(_) => (None, Some(words::ENERGY_ABSENT)),
        };
        let note = note
            .map(str::to_owned)
            .or_else(|| battery.map(battery_line));
        let choices = Profile::ALL
            .into_iter()
            .zip(words::PROFILES)
            .map(|(profile, label)| {
                let active = state.is_some_and(|s| s.active == Some(profile));
                let offered = state.is_some_and(|s| s.available.contains(&profile));
                (label, active, offered.then_some(Message::Profile(profile)))
            })
            .collect();
        group(words::ENERGY, note, panels::segmented(choices))
    }

    fn footer<'a>(&'a self, user: &'a str) -> Element<'a, Message> {
        let p = color::palette();
        let (line, tint) = match (&self.armed, self.failed(Service::Session)) {
            (Some(_), _) => (words::CONFIRM, p.amber_text),
            (None, Some(failed)) => (failed, p.salmon_text),
            (None, None) => (user, p.text_primary),
        };
        let name = row![
            panels::glyph(Glyph::User, p.text_secondary),
            ui::ellipsis(line)
                .font(typography::UI_SEMIBOLD)
                .color(tint)
                .width(Length::Fill),
        ]
        .spacing(panels::HEADER_GAP)
        .align_y(Alignment::Center);
        let buttons = [
            (Command::Lock, Glyph::Lock, words::LOCK),
            (Command::LogOut, Glyph::LogOut, words::LOG_OUT),
            (Command::Reboot, Glyph::Reboot, words::REBOOT),
            (Command::PowerOff, Glyph::PowerOff, words::POWER_OFF),
        ]
        .map(|(command, glyph, label)| {
            let armed = self.armed.is_some_and(|(a, _)| a == command);
            panels::round_button(glyph, label, armed, Some(Message::Session(command))).into()
        });
        let line = row![
            container(name).width(Length::Fill),
            iced::widget::Row::with_children(buttons).spacing(panels::ROUND_GAP),
        ]
        .align_y(Alignment::Center);
        let line = container(line).padding(Padding {
            top: panels::FOOTER_GAP,
            ..Padding::ZERO
        });
        column![ui::divider(), line].into()
    }

    fn view_networks(&self) -> Element<'_, Message> {
        let wifi = self.wifi.ready();
        let enabled = wifi.is_some_and(|w| w.enabled);
        let back = ui::icon_button(Icon::Back, words::BACK, Some(Message::ShowNetworks(false)));
        let title = text(words::WIFI)
            .font(typography::TITLE)
            .size(panels::TITLE_SIZE);
        let switch: Option<fn(bool) -> Message> = wifi.map(|_| switch_wifi as fn(bool) -> _);
        let head = row![
            back,
            title,
            space::horizontal(),
            ui::toggle(enabled, switch)
        ]
        .spacing(metrics::GAP_ICON)
        .align_y(Alignment::Center);
        let body: Element<'_, Message> = match wifi {
            Some(wifi) if wifi.enabled => {
                let mut rows: Vec<Element<'_, Message>> =
                    wifi.networks.iter().map(|n| self.network_row(n)).collect();
                if wifi.networks.is_empty() {
                    rows.push(ui::description(words::NO_NETWORK).into());
                }
                rows.push(self.other_network());
                scroll(Column::with_children(rows).spacing(metrics::GAP_TIGHT))
                    .height(Length::Fill)
                    .into()
            }
            _ => notice(words::WIFI_OFF_LINE),
        };
        let mut page = column![head, body]
            .spacing(metrics::GAP_INLINE)
            .height(Length::Fill);
        if let Some(failed) = self.failed(Service::Wifi) {
            page = page.push(
                text(failed)
                    .font(typography::UI)
                    .size(typography::SMALL_SIZE)
                    .style(panels::error_text),
            );
        }
        page.into()
    }

    fn network_row<'a>(&'a self, network: &'a Network) -> Element<'a, Message> {
        let connecting = self.connecting.as_deref() == Some(network.ssid.as_str());
        let refused = self
            .refused
            .as_ref()
            .filter(|(name, _)| *name == network.ssid)
            .map(|(_, outcome)| refusal(*outcome));
        let joining = self
            .joining
            .as_ref()
            .is_some_and(|j| j.path == network.path);
        // Under a password asked again, the refusal is said once, under the field.
        let line = if connecting {
            Some(words::CONNECTING)
        } else {
            refused.filter(|_| !joining)
        };
        let forget = (network.saved || network.active).then(|| {
            let ask = Message::ForgetNetwork(network.ssid.clone());
            ui::icon_button(Icon::Trash, words::FORGET_NETWORK, Some(ask)).into()
        });
        let item = panels::network_row(
            sherlock_ui::icon::wifi_level(network.strength),
            network.security != Security::Open,
            network.active,
            network.ssid.as_str(),
            line,
            Some(Message::Join(network.clone())),
            forget,
        );
        let inset = Padding::from([metrics::GAP_TIGHT * 2.0, metrics::MENU_ITEM_PADDING]);
        if self.forgetting_network.as_deref() == Some(network.ssid.as_str()) {
            let buttons = row![
                ui::danger_button(words::FORGET, Some(Message::ConfirmForgetNetwork(true))),
                ui::secondary_button(words::CANCEL, Some(Message::ConfirmForgetNetwork(false))),
            ]
            .spacing(metrics::GAP_INLINE / 2.0);
            let ask = column![
                ui::description(words::forget_network_question(&network.ssid)),
                buttons
            ]
            .spacing(metrics::GAP_TIGHT * 2.0);
            return column![item, container(ask).padding(inset)].into();
        }
        if !joining {
            return item;
        }
        if network.security == Security::Other {
            let line = ui::description(words::ENTERPRISE);
            return column![item, container(line).padding(inset)].into();
        }
        column![item, self.password_row()].into()
    }

    /// The password field, its eye and « Rejoindre », shaking after a wrong password.
    fn password_row(&self) -> Element<'_, Message> {
        let field = ui::text_field(words::PASSWORD, self.password.as_str(), Some(typed))
            .secure(!self.reveal)
            .on_submit(Message::SubmitPassword)
            .width(Length::Fill);
        let (eye, label) = if self.reveal {
            (Icon::EyeOff, words::HIDE_PASSWORD)
        } else {
            (Icon::Eye, words::SHOW_PASSWORD)
        };
        let eye = ui::icon_button(eye, label, Some(Message::Reveal));
        let ready = (!self.password.is_empty()).then_some(Message::SubmitPassword);
        let ask = row![field, eye, ui::primary_button(words::JOIN, ready)]
            .spacing(metrics::GAP_INLINE / 2.0)
            .align_y(Alignment::Center);
        let dx = self
            .shaken
            .map_or(0.0, |at| panels::shake(self.now.saturating_sub(at)));
        let mut lines = column![ask].spacing(metrics::GAP_TIGHT * 2.0);
        if self
            .refused
            .as_ref()
            .is_some_and(|(_, o)| *o == Outcome::WrongPassword)
        {
            lines = lines.push(
                text(words::WRONG_PASSWORD)
                    .font(typography::UI)
                    .size(typography::SMALL_SIZE)
                    .style(panels::error_text),
            );
        }
        container(lines)
            .padding(Padding {
                top: metrics::GAP_TIGHT * 2.0,
                bottom: metrics::GAP_TIGHT * 2.0,
                left: metrics::MENU_ITEM_PADDING + dx,
                right: metrics::MENU_ITEM_PADDING - dx,
            })
            .into()
    }

    /// « Autre réseau… » at the end of the list, and once open the form of a hidden network:
    /// its name, its protection, its password.
    fn other_network(&self) -> Element<'_, Message> {
        let Some((name, security)) = &self.other else {
            let open = Some(Message::OtherNetwork(true));
            return ui::menu_item_with(None, words::OTHER_NETWORK, Trailing::None, open).into();
        };
        let close = Some(Message::OtherNetwork(false));
        let head = ui::menu_item_with(None, words::OTHER_NETWORK, Trailing::None, close);
        let field = ui::text_field(words::NETWORK_NAME, name, Some(Message::OtherName))
            .on_submit(Message::SubmitPassword)
            .width(Length::Fill);
        let choices = [Security::Open, Security::Psk, Security::Sae]
            .into_iter()
            .zip(words::SECURITY_CHOICES)
            .map(|(choice, label)| {
                (
                    label,
                    *security == choice,
                    Some(Message::OtherSecurity(choice)),
                )
            })
            .collect();
        let mut form = column![
            field,
            group(words::SECURITY, None, panels::segmented(choices))
        ]
        .spacing(metrics::GAP_INLINE / 2.0);
        let inset = Padding::from([metrics::GAP_TIGHT * 2.0, metrics::MENU_ITEM_PADDING]);
        if *security == Security::Open {
            let ready = (!name.is_empty()).then_some(Message::SubmitPassword);
            form = form.push(ui::primary_button(words::JOIN, ready));
            return column![head, container(form).padding(inset)].into();
        }
        column![head, container(form).padding(inset), self.password_row()].into()
    }
}

/// What a line says after a join that ended badly.
fn refusal(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::WrongPassword => words::WRONG_PASSWORD,
        Outcome::TimedOut => words::JOIN_TIMED_OUT,
        Outcome::Joined | Outcome::Failed(_) => words::JOIN_FAILED,
    }
}

fn typed(password: String) -> Message {
    Message::Password(Secret::new(password))
}

fn switch_wifi(_: bool) -> Message {
    Message::ToggleWifi
}

/// The tile of the game mode: there is no game mode yet.
fn game_tile<'a>() -> Element<'a, Message> {
    panels::quick_tile(
        Glyph::Gamepad,
        words::GAME_MODE,
        words::SOON.to_owned(),
        0.0,
        None,
        None,
    )
}

/// A labelled group (« Carte graphique », « Énergie »): its label, a note on its right, its
/// control below.
fn group<'a>(
    label: &'a str,
    note: Option<String>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut head = row![
        text(label)
            .font(typography::UI)
            .size(typography::SMALL_SIZE)
            .style(panels::secondary_text)
    ];
    if let Some(note) = note {
        head = head.push(space::horizontal()).push(
            text(note)
                .font(typography::UI)
                .size(typography::SMALL_SIZE)
                .style(panels::weekday_text),
        );
    }
    column![head, control].spacing(panels::GROUP_GAP).into()
}

/// The battery as « Énergie » says it.
fn battery_line(battery: Battery) -> String {
    use words::Charging;
    let charging = match battery.charge {
        Charge::Discharging => Charging::No(battery.minutes),
        Charge::Charging => Charging::Yes(battery.minutes),
        Charge::Full | Charge::Idle => Charging::Plugged,
    };
    words::battery(battery.percent, charging)
}

/// A short line in the middle of an empty page.
fn notice(line: &str) -> Element<'_, Message> {
    let line = ui::body(line)
        .style(panels::secondary_text)
        .align_x(Alignment::Center);
    container(line).center(Length::Fill).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Quick settings whose `pactl` is nowhere: never the machine's sound.
    fn without_sound() -> Quick {
        let pactl = Pactl {
            program: std::ffi::OsString::from("/nonexistent/pactl"),
        };
        Quick {
            pactl,
            ..Quick::default()
        }
    }

    fn output(volume: u16) -> Load<Output> {
        Load::Ready(Output {
            sound: crate::system::audio::Sound {
                volume,
                muted: false,
            },
            name: String::from("Haut-parleurs intégrés"),
        })
    }

    fn volume(quick: &Quick) -> u16 {
        quick.sound.ready().map_or(0, |o| o.sound.volume)
    }

    #[test]
    fn a_held_slider_does_not_jump_under_a_change_made_elsewhere() {
        let mut quick = without_sound();
        let _ = quick.update(Message::Sound(output(40)));
        let _ = quick.update(Message::Volume(70));
        let _ = quick.update(Message::Done(Service::Sound, Ok(())));
        // A key moved the volume meanwhile: the slider under the pointer stays.
        let _ = quick.update(Message::Sound(output(55)));
        assert_eq!(volume(&quick), 70);
        // Let go, the change made elsewhere shows.
        let _ = quick.update(Message::Release(Service::Sound));
        assert_eq!(quick.held, None);
        let _ = quick.update(Message::Sound(output(55)));
        assert_eq!(volume(&quick), 55);
        // A value still on its way to the server is not undone by an older read either.
        let _ = quick.update(Message::Volume(20));
        let _ = quick.update(Message::Release(Service::Sound));
        let _ = quick.update(Message::Sound(output(55)));
        assert_eq!(volume(&quick), 20);
    }

    #[test]
    fn the_keyboard_backlight_has_a_slider_only_when_the_machine_has_one() {
        let mut quick = without_sound();
        assert!(quick.keyboard_row().is_none());
        let absent = Problem::Absent(String::from("no keyboard backlight"));
        let _ = quick.update(Message::Keyboard(Load::Failed(absent)));
        assert!(quick.keyboard_row().is_none());
        let light = KeyboardLight {
            brightness: 1,
            max: 3,
            through: keyboard::Through::UPower,
        };
        let _ = quick.update(Message::Keyboard(Load::Ready(light.clone())));
        assert!(quick.keyboard_row().is_some());
        assert!(quick.update(Message::KeyboardLight(100)).units() > 0);
        assert_eq!(quick.keyboard.ready().map(|l| l.brightness), Some(3));
        // UPower tells of a key meanwhile: the held slider stays.
        let _ = quick.update(Message::Keyboard(Load::Ready(light)));
        assert_eq!(quick.keyboard.ready().map(|l| l.brightness), Some(3));
    }

    #[test]
    fn the_sound_and_the_brightness_changed_elsewhere_are_followed() {
        use iced::futures::executor::block_on;
        let dir = std::env::temp_dir().join(format!("quick-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let light = dir.join("backlight/amdgpu_bl1");
        std::fs::create_dir_all(&light).unwrap();
        for (file, value) in [("brightness", "51"), ("actual_brightness", "51")] {
            std::fs::write(light.join(file), value).unwrap();
        }
        std::fs::write(light.join("max_brightness"), "255").unwrap();
        // A fake `pactl subscribe`: one change of the output, then nothing.
        let pactl = dir.join("pactl");
        let script = "#!/bin/sh\necho \"Event 'change' on sink #1\"\nexec sleep 30\n";
        std::fs::write(&pactl, script).unwrap();
        let chmod = std::process::Command::new("chmod")
            .arg("+x")
            .arg(&pactl)
            .status();
        assert!(chmod.unwrap().success());
        let mut changes = local(pactl.into_os_string(), dir.join("backlight"));
        assert_eq!(block_on(changes.next()), Some(Service::Sound));
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(light.join("actual_brightness"), "200").unwrap();
        assert_eq!(block_on(changes.next()), Some(Service::Brightness));
        drop(changes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slider_sends_one_value_at_a_time_the_latest_winning() {
        let mut throttle = Throttle::default();
        assert_eq!(throttle.want(10), Some(10));
        assert_eq!(throttle.want(20), None);
        assert_eq!(throttle.want(30), None);
        assert_eq!(throttle.done(), Some(30));
        assert_eq!(throttle.done(), None);
        assert_eq!(throttle.want(40), Some(40));
    }

    #[test]
    fn the_password_never_shows_and_goes_when_the_panel_closes() {
        let typed = "hunter2-wifi";
        let mut quick = Quick::default();
        let network = Network {
            ssid: String::from("Voisin"),
            strength: 50,
            security: Security::Psk,
            active: false,
            saved: false,
            path: String::from("/ap/3"),
        };
        quick.joining = Some(network);
        let message = Message::Password(Secret::new(typed.to_owned()));
        assert!(!format!("{message:?}").contains(typed));
        let _ = quick.update(message);
        assert_eq!(quick.password.as_str(), typed);
        assert!(!format!("{quick:?}").contains(typed), "{quick:?}");
        assert!(format!("{quick:?}").contains("***"));
        let _ = quick.close();
        assert!(quick.password.is_empty());
        assert!(quick.joining.is_none());
        let mut secret = Secret::new(typed.to_owned());
        secret.wipe();
        assert_eq!(secret, Secret::default());
    }

    fn wifi_with(networks: Vec<Network>) -> Quick {
        Quick {
            wifi: Load::Ready(Wifi {
                enabled: true,
                hardware_off: false,
                device: Some(String::from("/dev/1")),
                networks,
            }),
            ..Quick::default()
        }
    }

    fn neighbour(saved: bool) -> Network {
        Network {
            ssid: String::from("Voisin"),
            strength: 50,
            security: Security::Psk,
            active: false,
            saved,
            path: String::from("/ap/3"),
        }
    }

    #[test]
    fn a_wrong_password_is_asked_again_and_the_tile_says_so() {
        let mut quick = wifi_with(vec![neighbour(false)]);
        // A new protected network asks for its password; a saved one connects at once.
        let _ = quick.update(Message::Join(neighbour(false)));
        assert!(quick.joining.is_some() && quick.connecting.is_none());
        let _ = quick.update(Message::Password(Secret::new(String::from("faux"))));
        let _ = quick.update(Message::SubmitPassword);
        assert_eq!(quick.connecting.as_deref(), Some("Voisin"));
        assert!(quick.joining.is_none() && quick.password.is_empty());
        // A second click while it connects does nothing.
        let _ = quick.update(Message::Join(neighbour(true)));
        assert!(quick.joining.is_none());

        quick.now = Duration::from_secs(10);
        let wrong = Ok(Outcome::WrongPassword);
        let _ = quick.update(Message::Joined(neighbour(false), wrong));
        assert!(quick.connecting.is_none(), "« Connexion… » ends");
        assert_eq!(
            quick.joining.as_ref().map(|n| n.ssid.as_str()),
            Some("Voisin")
        );
        assert_eq!(quick.shaken, Some(Duration::from_secs(10)));
        assert!(quick.running(), "the field shakes");
        quick.tick(Duration::from_secs(11));
        assert!(!quick.running());
        assert_eq!(refusal(Outcome::WrongPassword), words::WRONG_PASSWORD);

        // Another failure says so without asking anything.
        let _ = quick.update(Message::Joined(neighbour(true), Ok(Outcome::TimedOut)));
        assert_eq!(
            quick.refused,
            Some((String::from("Voisin"), Outcome::TimedOut))
        );
        // Reopened, the panel starts clean.
        let _ = quick.open();
        assert!(quick.refused.is_none() && quick.joining.is_none());

        // A saved network shows « Connexion… » too.
        let _ = quick.update(Message::Join(neighbour(true)));
        assert_eq!(quick.connecting.as_deref(), Some("Voisin"));
    }

    #[test]
    fn a_hidden_network_needs_a_name_and_its_password() {
        let mut quick = wifi_with(Vec::new());
        let _ = quick.update(Message::OtherNetwork(true));
        let _ = quick.update(Message::SubmitPassword);
        assert!(quick.connecting.is_none());
        let _ = quick.update(Message::OtherName("x".repeat(40)));
        assert_eq!(quick.other.as_ref().map(|(n, _)| n.len()), Some(32));
        let _ = quick.update(Message::OtherName(String::from("Cachette")));
        let _ = quick.update(Message::SubmitPassword);
        assert!(
            quick.connecting.is_none(),
            "a WPA network wants its password"
        );
        let _ = quick.update(Message::Reveal);
        assert!(quick.reveal);
        let _ = quick.update(Message::Password(Secret::new(String::from("s3cret"))));
        let _ = quick.update(Message::SubmitPassword);
        assert_eq!(quick.connecting.as_deref(), Some("Cachette"));
        assert!(quick.other.is_none() && !quick.reveal);
        // Its password refused, the form comes back with its name.
        let hidden = Network::hidden("Cachette", Security::Psk);
        let _ = quick.update(Message::Joined(hidden, Ok(Outcome::WrongPassword)));
        assert_eq!(quick.other, Some((String::from("Cachette"), Security::Psk)));
    }

    #[test]
    fn the_sound_server_gets_one_call_at_a_time_the_latest_winning() {
        let mut queue = SoundQueue::default();
        assert_eq!(
            queue.push(SoundCall::Volume(10)),
            Some(SoundCall::Volume(10))
        );
        assert_eq!(queue.push(SoundCall::Volume(20)), None);
        assert_eq!(queue.push(SoundCall::Mute(true)), None);
        assert_eq!(queue.push(SoundCall::Volume(30)), None);
        assert_eq!(queue.done(), Some(SoundCall::Mute(true)));
        assert_eq!(queue.push(SoundCall::Volume(40)), None);
        assert_eq!(queue.done(), Some(SoundCall::Volume(40)));
        assert_eq!(queue.done(), None);
        assert_eq!(
            queue.push(SoundCall::Mute(false)),
            Some(SoundCall::Mute(false))
        );
    }

    /// A session that records the commands it is given.
    #[derive(Default)]
    struct Recorded(Mutex<Vec<Command>>);

    impl Session for Recorded {
        fn run(&self, power: Command) -> Result<(), launcher::launch::LaunchError> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(power);
            Ok(())
        }
    }

    #[test]
    fn session_commands_follow_the_launcher_rules() {
        let session = Arc::new(Recorded::default());
        // Never `Quick::new`: it would point at the user's own notifications file.
        let mut quick = Quick {
            commands: Commands(session.clone()),
            ..Quick::default()
        };
        let ran = || session.0.lock().map(|v| v.clone()).unwrap_or_default();
        // Locking at once.
        let _ = quick.update(Message::Session(Command::Lock));
        assert_eq!(ran(), [Command::Lock]);
        // Powering off: the first click arms, a second one too soon does nothing.
        let _ = quick.update(Message::Session(Command::PowerOff));
        let _ = quick.update(Message::Session(Command::PowerOff));
        assert_eq!(ran(), [Command::Lock]);
        assert!(quick.armed.is_some());
        // Another click cancels it.
        let _ = quick.update(Message::ShowNetworks(false));
        assert!(quick.armed.is_none());
        // Armed long enough ago: the second click runs it.
        let earlier = Instant::now() - sherlock_ui::launcher::CONFIRM_DELAY;
        quick.armed = Some((Command::Reboot, earlier));
        let _ = quick.update(Message::Session(Command::Reboot));
        assert_eq!(ran(), [Command::Lock, Command::Reboot]);
        assert!(quick.armed.is_none());
        // A session that refuses: the footer says so.
        let mut refused = Quick::default();
        let _ = refused.update(Message::Session(Command::Lock));
        assert!(matches!(refused.failure, Some((Service::Session, _))));
    }

    #[test]
    fn do_not_disturb_is_written_to_the_notifications_file() {
        let dir = std::env::temp_dir().join(format!("bar-dnd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("notifications").join("notifications.conf");
        let mut quick = Quick {
            settings: Some(file.clone()),
            ..Quick::default()
        };
        // The tile asks for a task: nothing is read or written on the interface thread. (Its
        // thread writes a file of its own, apart from the one checked below.)
        let mut asking = Quick {
            settings: Some(dir.join("other").join("notifications.conf")),
            ..Quick::default()
        };
        assert!(asking.update(Message::ToggleDoNotDisturb).units() > 0);
        assert!(asking.do_not_disturb.is_none());
        let done = toggle_do_not_disturb(&file);
        assert_eq!(done, Ok(true));
        let _ = quick.update(Message::DoNotDisturb(done));
        assert_eq!(quick.do_not_disturb, Some(true));
        let (config, _) = NotificationsConfig::load(&file).unwrap();
        assert!(config.do_not_disturb && read_do_not_disturb(&file));
        assert_eq!(toggle_do_not_disturb(&file), Ok(false));
        assert!(!NotificationsConfig::load(&file).unwrap().0.do_not_disturb);
        // A file that cannot be written: « Échec » under the tile.
        let mut blocked = Quick::default();
        let done = toggle_do_not_disturb(Path::new("/proc/no/notifications.conf"));
        assert!(done.is_err());
        let _ = blocked.update(Message::DoNotDisturb(done));
        assert!(matches!(blocked.failure, Some((Service::DoNotDisturb, _))));
        assert_eq!(blocked.failed(Service::DoNotDisturb), Some(words::FAILED));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_burst_of_changes_reads_a_service_twice_at_most() {
        let mut quick = Quick::default();
        // A first read goes out, the next ones of the burst wait for it.
        assert!(quick.update(Message::Changed(Service::Bluetooth)).units() > 0);
        for _ in 0..50 {
            assert_eq!(
                quick.update(Message::Changed(Service::Bluetooth)).units(),
                0
            );
        }
        let absent = || Load::Failed(Problem::Absent(String::from("test")));
        // It ends: one more read for all of them, then nothing.
        assert!(quick.update(Message::Bluetooth(absent())).units() > 0);
        assert_eq!(quick.update(Message::Bluetooth(absent())).units(), 0);
        assert!(quick.update(Message::Changed(Service::Bluetooth)).units() > 0);
        // Another service is not held back.
        assert!(quick.update(Message::Changed(Service::Energy)).units() > 0);
    }

    #[test]
    fn a_switched_tile_mixes_its_colors_then_rests() {
        let ms = Duration::from_millis;
        let mut quick = Quick {
            now: ms(1000),
            ..Quick::default()
        };
        let bluetooth = |powered| {
            Message::Bluetooth(Load::Ready(Bluetooth {
                adapter: String::from("/org/bluez/hci0"),
                powered,
                connected: 0,
                devices: Vec::new(),
            }))
        };
        // The first state read shows at once.
        let _ = quick.update(bluetooth(false));
        assert_eq!(quick.lit[1].value(quick.now), 0.0);
        assert!(!quick.running());
        let _ = quick.update(bluetooth(true));
        assert!(quick.running());
        quick.tick(ms(1060));
        let halfway = quick.lit[1].value(quick.now);
        assert!(halfway > 0.0 && halfway < 1.0, "{halfway}");
        quick.tick(ms(1000) + panels::TILE_FADE);
        assert_eq!(quick.lit[1].value(quick.now), 1.0);
        assert!(!quick.running(), "no frame at rest");
        // The Wi-Fi page slides in, then rests too.
        let _ = quick.update(Message::ShowNetworks(true));
        assert!(quick.running());
        quick.tick(ms(3000));
        assert!(!quick.running());
    }

    /// The control centre with three fictitious devices: « Casque » paired and connected at
    /// 80 %, « Souris » paired, « Clavier » found by a scan.
    pub(super) fn with_devices() -> Quick {
        use crate::system::bluetooth::{Device, Kind};
        let device = |path: &str, name: &str, kind, paired, connected, battery| Device {
            path: format!("/org/bluez/hci0/{path}"),
            name: String::from(name),
            kind,
            paired,
            connected,
            battery,
        };
        let mut quick = Quick::default();
        let _ = quick.update(Message::Bluetooth(Load::Ready(Bluetooth {
            adapter: String::from("/org/bluez/hci0"),
            powered: true,
            connected: 1,
            devices: vec![
                device("dev_1", "Casque", Kind::Audio, true, true, Some(80)),
                device("dev_2", "Souris", Kind::Mouse, true, false, None),
                device("dev_3", "Clavier", Kind::Keyboard, false, false, None),
            ],
        })));
        quick
    }

    #[test]
    fn the_device_list_scans_while_open_and_stops_when_it_closes() {
        let mut quick = with_devices();
        assert!(quick.update(Message::ShowDevices(true)).units() > 0);
        assert_eq!(quick.page, Page::Devices);
        assert_eq!(quick.scanning.as_deref(), Some("/org/bluez/hci0"));
        // Back to the main page: the scan stops.
        assert!(quick.update(Message::ShowDevices(false)).units() > 0);
        assert_eq!(quick.scanning, None);
        // The panel closes with the list open: the scan stops too.
        let _ = quick.update(Message::ShowDevices(true));
        assert!(quick.close().units() > 0);
        assert_eq!(quick.scanning, None);
        assert_eq!(quick.close().units(), 0, "stopped once");
    }

    #[test]
    fn a_device_is_connected_paired_or_forgotten_one_at_a_time() {
        use crate::system::bluetooth::{Asked, Pairing};
        let mut quick = with_devices();
        let mouse = String::from("/org/bluez/hci0/dev_2");
        assert!(quick.update(Message::Device(mouse.clone())).units() > 0);
        assert_eq!(quick.device_busy.as_deref(), Some(mouse.as_str()));
        // Another click waits for the first one.
        let keyboard = String::from("/org/bluez/hci0/dev_3");
        assert_eq!(quick.update(Message::Device(keyboard.clone())).units(), 0);
        // Another Bluetooth action ending (a scan, the switch) does not free the list.
        let _ = quick.update(Message::Done(Service::Bluetooth, Ok(())));
        assert_eq!(quick.device_busy.as_deref(), Some(mouse.as_str()));
        let _ = quick.update(Message::DeviceDone(Ok(())));
        assert_eq!(quick.device_busy, None);
        // A device found pairs; the agent asks, the panel answers what the user clicks.
        assert!(quick.update(Message::Device(keyboard.clone())).units() > 0);
        let (sender, answer) = iced::futures::channel::oneshot::channel();
        let reply = crate::system::bluetooth::Reply::new(sender);
        let asked = Asked::Confirm(String::from("123456"));
        let _ = quick.update(Message::Pairing(Pairing::Ask(asked, reply)));
        assert!(quick.asked.is_some());
        let _ = quick.update(Message::Answer(true));
        assert_eq!(iced::futures::executor::block_on(answer), Ok(true));
        let refused = Err(Problem::Refused(String::from("Rejected")));
        let _ = quick.update(Message::Pairing(Pairing::Done(refused)));
        assert_eq!(quick.device_busy, None);
        assert_eq!(quick.failed(Service::Bluetooth), Some(words::REFUSED));
        // The panel closes while a device pairs: the pairing stops and the list is free.
        assert!(quick.update(Message::Device(keyboard.clone())).units() > 0);
        let pairing = quick.pairing.clone().expect("a pairing under way");
        let _ = quick.close();
        assert!(pairing.is_aborted());
        assert_eq!(
            (quick.device_busy.as_ref(), quick.pairing.is_none()),
            (None, true)
        );
        // « Oublier » asks first; « Annuler » keeps the device.
        let _ = quick.update(Message::Forget(mouse.clone()));
        assert_eq!(quick.forgetting.as_deref(), Some(mouse.as_str()));
        assert_eq!(quick.update(Message::ConfirmForget(false)).units(), 0);
        assert_eq!(quick.forgetting, None);
        let _ = quick.update(Message::Forget(mouse));
        assert!(quick.update(Message::ConfirmForget(true)).units() > 0);
    }

    #[test]
    fn a_refusal_shows_under_its_tile_only() {
        let mut quick = Quick::default();
        let _ = quick.done(
            Service::Bluetooth,
            Err(Problem::Refused(String::from("polkit"))),
        );
        assert_eq!(quick.failed(Service::Bluetooth), Some(words::REFUSED));
        assert_eq!(quick.failed(Service::Wifi), None);
        let _ = quick.done(Service::Bluetooth, Ok(()));
        assert_eq!(quick.failed(Service::Bluetooth), None);
    }
}
