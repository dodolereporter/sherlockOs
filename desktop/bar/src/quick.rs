//! The quick settings of the control centre (Panels board): Wi-Fi, Bluetooth, Do not disturb,
//! game mode, volume, brightness, graphics card, energy, and the session. Their state, their
//! messages, the calls to the services (`system`), and their view.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use iced::futures::stream::{self, BoxStream};
use iced::futures::{Future, StreamExt};
use iced::widget::{Column, column, container, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding, Task};
use launcher::source::Power as Command;
use launcher::system::{NoSession, Session};
use notifications_config::NotificationsConfig;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, Glyph, text as words};
use sherlock_ui::{color, metrics, typography};

use crate::system::audio::{Output, Pactl};
use crate::system::backlight::{self, Backlight};
use crate::system::bluetooth::{self, Bluetooth};
use crate::system::network::{self, Network, Security, Wifi};
use crate::system::power::{self, Power, Profile};
use crate::system::{self, Problem};

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
}

/// The page of the panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Main,
    /// The Wi-Fi networks around.
    Networks,
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

#[derive(Debug, Default)]
pub struct Quick {
    pub wifi: Load<Wifi>,
    pub bluetooth: Load<Bluetooth>,
    pub sound: Load<Output>,
    pub brightness: Load<Backlight>,
    pub energy: Load<Power>,
    pub do_not_disturb: Option<bool>,
    pub page: Page,
    /// The network chosen that needs its password, and the password typed.
    pub joining: Option<Network>,
    pub password: String,
    /// The network being joined, until NetworkManager reports it.
    pub connecting: Option<String>,
    /// A session command waiting for its second click, and when it was armed.
    pub armed: Option<(Command, Instant)>,
    /// The last action that failed.
    pub failure: Option<(Service, Problem)>,
    pub commands: Commands,
    /// `pactl`, the Do not disturb file and the backlights: the real ones but in tests.
    pub pactl: Pactl,
    pub settings: Option<PathBuf>,
    pub sysfs: PathBuf,
    volume: Throttle,
    light: Throttle,
}

#[derive(Debug, Clone)]
pub enum Message {
    Wifi(Load<Wifi>),
    Bluetooth(Load<Bluetooth>),
    Sound(Load<Output>),
    Brightness(Load<Backlight>),
    Energy(Load<Power>),
    ToggleWifi,
    ShowNetworks(bool),
    Join(Network),
    /// The network chosen has no saved connection: its password is asked for.
    AskPassword,
    Password(String),
    SubmitPassword,
    ToggleBluetooth,
    ToggleDoNotDisturb,
    Volume(u8),
    Mute,
    Light(u8),
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
    on_system(call, move |done| Message::Done(service, done))
}

/// Runs blocking `work` (a command, a file) in an iced task.
fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl Fn(T) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(async move { work() }, done)
}

impl Quick {
    /// The quick settings, the session commands run by `commands`.
    pub fn new(commands: Arc<dyn Session>) -> Self {
        Self {
            commands: Commands(commands),
            settings: notifications_config::path(),
            sysfs: PathBuf::from(backlight::SYSFS),
            ..Self::default()
        }
    }

    /// Reads every service: when the panel opens.
    pub fn open(&mut self) -> Task<Message> {
        self.page = Page::Main;
        self.joining = None;
        self.password.clear();
        self.armed = None;
        self.failure = None;
        self.do_not_disturb = self.settings.as_deref().map(|path| {
            NotificationsConfig::load(path).is_ok_and(|(config, _)| config.do_not_disturb)
        });
        Task::batch([
            self.read(Service::Wifi),
            self.read(Service::Bluetooth),
            self.read(Service::Sound),
            self.read(Service::Brightness),
            self.read(Service::Energy),
        ])
    }

    fn read(&self, service: Service) -> Task<Message> {
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
                blocking(move || pactl.read(), |r| Message::Sound(Load::from_result(r)))
            }
            Service::Brightness => {
                let root = self.sysfs.clone();
                blocking(
                    move || backlight::find(&root),
                    |r| Message::Brightness(Load::from_result(r)),
                )
            }
            Service::DoNotDisturb | Service::Session => Task::none(),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        // Any other click cancels a session command waiting for its confirmation.
        let passive = matches!(
            message,
            Message::Session(_)
                | Message::Done(..)
                | Message::Changed(_)
                | Message::Wifi(_)
                | Message::Bluetooth(_)
                | Message::Sound(_)
                | Message::Brightness(_)
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
            }
            Message::Bluetooth(read) => self.bluetooth = read,
            Message::Sound(read) => self.sound = read,
            Message::Brightness(read) => self.brightness = read,
            Message::Energy(read) => self.energy = read,
            Message::ToggleWifi => {
                if let Some(wifi) = self.wifi.ready() {
                    let on = !wifi.enabled;
                    return act(Service::Wifi, move |bus| async move {
                        network::enable(&bus, on).await
                    });
                }
            }
            Message::ShowNetworks(show) => {
                self.page = if show { Page::Networks } else { Page::Main };
                self.joining = None;
                self.password.clear();
                let device = self.wifi.ready().and_then(|w| w.device.clone());
                if let (true, Some(device)) = (show, device) {
                    // The list is read again once the scan is asked for.
                    return on_system(
                        move |bus| async move {
                            network::scan(&bus, &device).await?;
                            network::read(&bus).await
                        },
                        |r| Message::Wifi(Load::from_result(r)),
                    );
                }
            }
            Message::Join(chosen) => return self.join(chosen),
            Message::AskPassword => {}
            Message::Password(typed) => self.password = typed,
            Message::SubmitPassword => {
                if let Some(chosen) = self.joining.clone()
                    && !self.password.is_empty()
                {
                    let password = std::mem::take(&mut self.password);
                    return self.connect(chosen, Some(password));
                }
            }
            Message::ToggleBluetooth => {
                if let Some(read) = self.bluetooth.ready() {
                    let (adapter, on) = (read.adapter.clone(), !read.powered);
                    return act(Service::Bluetooth, move |bus| async move {
                        bluetooth::power(&bus, &adapter, on).await
                    });
                }
            }
            Message::ToggleDoNotDisturb => self.toggle_do_not_disturb(),
            Message::Volume(value) => {
                if let Load::Ready(output) = &mut self.sound {
                    output.sound.volume = u16::from(value);
                }
                if let Some(value) = self.volume.want(value) {
                    return self.send_volume(value);
                }
            }
            Message::Mute => {
                if let Load::Ready(output) = &mut self.sound {
                    output.sound.muted = !output.sound.muted;
                    let (pactl, muted) = (self.pactl.clone(), output.sound.muted);
                    return blocking(
                        move || pactl.set_mute(muted),
                        |r| Message::Done(Service::Sound, r),
                    );
                }
            }
            Message::Light(value) => {
                if let Load::Ready(light) = &mut self.brightness {
                    light.brightness = light.raw(value);
                }
                if let Some(value) = self.light.want(value) {
                    return self.send_light(value);
                }
            }
            Message::Profile(profile) => {
                if let Load::Ready(state) = &self.energy {
                    let state = state.clone();
                    return act(Service::Energy, move |bus| async move {
                        power::choose(&bus, &state, profile).await
                    });
                }
            }
            Message::Session(command) => self.session(command),
            Message::Done(service, done) => return self.done(service, done),
            Message::Changed(service) => return self.read(service),
        }
        Task::none()
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
            Service::Sound => self.volume.done().map(|v| self.send_volume(v)),
            Service::Brightness => self.light.done().map(|v| self.send_light(v)),
            _ => None,
        };
        match next {
            Some(task) => task,
            // A slider read again in the middle of a drag would jump back.
            None => self.read(service),
        }
    }

    fn send_volume(&self, value: u8) -> Task<Message> {
        let pactl = self.pactl.clone();
        blocking(
            move || pactl.set_volume(value),
            |r| Message::Done(Service::Sound, r),
        )
    }

    fn send_light(&self, value: u8) -> Task<Message> {
        let Some(light) = self.brightness.ready().cloned() else {
            return Task::none();
        };
        act(Service::Brightness, move |bus| async move {
            backlight::set(&bus, &light, value).await
        })
    }

    /// A network of the list: joined at once when it is open or saved; else its password is
    /// asked for below it.
    fn join(&mut self, chosen: Network) -> Task<Message> {
        if chosen.active {
            return Task::none();
        }
        if chosen.security == Security::Open {
            return self.connect(chosen, None);
        }
        self.password.clear();
        self.joining = Some(chosen.clone());
        if chosen.security == Security::Other {
            return Task::none();
        }
        let Some(device) = self.wifi.ready().and_then(|w| w.device.clone()) else {
            return Task::none();
        };
        on_system(
            move |bus| async move {
                if network::saved(&bus, &chosen.ssid).await?.is_none() {
                    return Ok(false);
                }
                network::connect(&bus, &device, &chosen, None).await?;
                Ok(true)
            },
            |r| match r {
                Ok(true) => Message::Done(Service::Wifi, Ok(())),
                Ok(false) => Message::AskPassword,
                Err(problem) => Message::Done(Service::Wifi, Err(problem)),
            },
        )
    }

    fn connect(&mut self, chosen: Network, password: Option<String>) -> Task<Message> {
        let Some(device) = self.wifi.ready().and_then(|w| w.device.clone()) else {
            return Task::none();
        };
        self.joining = None;
        self.connecting = Some(chosen.ssid.clone());
        act(Service::Wifi, move |bus| async move {
            network::connect(&bus, &device, &chosen, password.as_deref()).await
        })
    }

    fn toggle_do_not_disturb(&mut self) {
        let Some(path) = self.settings.clone() else {
            return;
        };
        let (mut config, _) = NotificationsConfig::load(&path).unwrap_or_default();
        config.do_not_disturb = !config.do_not_disturb;
        match config.save(&path) {
            Ok(()) => {
                self.do_not_disturb = Some(config.do_not_disturb);
                if self
                    .failure
                    .as_ref()
                    .is_some_and(|(s, _)| *s == Service::DoNotDisturb)
                {
                    self.failure = None;
                }
            }
            Err(error) => {
                tracing::warn!(%error, "cannot write Do not disturb");
                let problem = Problem::Failed(error.to_string());
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
        self.armed = None;
        match self.commands.0.run(command) {
            Ok(()) => {
                tracing::info!(?command, "session command");
                self.failure = None;
            }
            Err(error) => {
                tracing::error!(%error, "cannot run the session command");
                let problem = Problem::Failed(error.to_string());
                self.failure = Some((Service::Session, problem));
            }
        }
    }
}

/// The changes the services announce on the system bus while the control centre is open:
/// NetworkManager's main object and the states of its devices, BlueZ's adapters,
/// power-profiles-daemon. Nothing when the bus cannot be reached (logged once).
pub fn watch() -> BoxStream<'static, Service> {
    stream::once(async {
        match changes().await {
            Ok(changes) => changes,
            Err(problem) => {
                tracing::warn!(%problem, "the quick settings will not follow the services");
                stream::empty().boxed()
            }
        }
    })
    .flatten()
    .boxed()
}

/// The streams of `watch`, on the system bus.
pub async fn changes() -> system::Result<BoxStream<'static, Service>> {
    let bus = system::system().await?;
    changes_on(&bus).await
}

/// The streams of `watch`, on `bus`.
pub async fn changes_on(bus: &zbus::Connection) -> system::Result<BoxStream<'static, Service>> {
    let properties = |sender: &'static str| -> zbus::Result<zbus::MatchRule<'static>> {
        Ok(zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(sender)?
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
        (properties(bluetooth::SERVICE)?, Service::Bluetooth),
    ];
    for (name, _, _) in power::NAMES {
        rules.push((properties(name)?, Service::Energy));
    }
    let mut streams = Vec::new();
    for (rule, service) in rules {
        let messages = zbus::MessageStream::for_match_rule(rule, bus, None).await?;
        streams.push(messages.map(move |_| service).boxed());
    }
    Ok(stream::select_all(streams).boxed())
}

// The view.

impl Quick {
    /// The panel: the main page or the Wi-Fi networks.
    pub fn view<'a>(&'a self, user: &'a str) -> Element<'a, Message> {
        match self.page {
            Page::Main => self.view_main(user),
            Page::Networks => self.view_networks(),
        }
    }

    /// The short word that replaces the line of `service` after its last action failed.
    fn failed(&self, service: Service) -> Option<&'static str> {
        match &self.failure {
            Some((s, Problem::Refused(_))) if *s == service => Some(words::REFUSED),
            Some((s, _)) if *s == service => Some(words::FAILED),
            _ => None,
        }
    }

    fn view_main<'a>(&'a self, user: &'a str) -> Element<'a, Message> {
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
            self.brightness_row(),
            group(words::GPU, Some(words::SOON), panels::segmented(gpu)),
            self.energy_group(),
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
                let line = match (wifi.enabled, &self.connecting, wifi.connected()) {
                    (false, _, _) => words::OFF.to_owned(),
                    (true, Some(_), _) => words::CONNECTING.to_owned(),
                    (true, None, Some(network)) => network.ssid.clone(),
                    (true, None, None) => words::NOT_CONNECTED.to_owned(),
                };
                (wifi.enabled, line, true)
            }
        };
        let line = self.failed(Service::Wifi).map_or(line, str::to_owned);
        let more = (enabled && on).then_some((words::NETWORKS, Message::ShowNetworks(true)));
        panels::tile(
            Glyph::Wifi,
            words::WIFI,
            line,
            on,
            enabled.then_some(Message::ToggleWifi),
            more,
        )
    }

    fn bluetooth_tile(&self) -> Element<'_, Message> {
        let (on, line, enabled) = match &self.bluetooth {
            Load::Loading => (false, words::LOADING.to_owned(), false),
            Load::Failed(_) => (false, words::UNAVAILABLE.to_owned(), false),
            Load::Ready(read) if !read.powered => (false, words::OFF.to_owned(), true),
            Load::Ready(read) if read.connected > 0 => {
                (true, words::devices(read.connected), true)
            }
            Load::Ready(_) => (true, words::ON.to_owned(), true),
        };
        let line = self.failed(Service::Bluetooth).map_or(line, str::to_owned);
        panels::tile(
            Glyph::Bluetooth,
            words::BLUETOOTH,
            line,
            on,
            enabled.then_some(Message::ToggleBluetooth),
            None,
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
        panels::tile(
            Glyph::Moon,
            words::DO_NOT_DISTURB,
            line.to_owned(),
            on,
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
        let control = panels::slider_row(
            panels::glyph_button(glyph, label, mute),
            volume.unwrap_or(0),
            on_change,
        );
        let caption = text(line)
            .font(typography::UI)
            .size(typography::SMALL_SIZE)
            .style(panels::secondary_text)
            .wrapping(text::Wrapping::None);
        let caption = container(caption)
            .padding(Padding {
                left: panels::SLIDER_INSET,
                ..Padding::ZERO
            })
            .clip(true);
        column![control, caption]
            .spacing(panels::CAPTION_GAP)
            .into()
    }

    fn brightness_row(&self) -> Element<'_, Message> {
        let value = self.brightness.ready().map(Backlight::percent);
        let icon = container(panels::glyph(Glyph::Sun, color::palette().text_secondary))
            .center(panels::ROW_BUTTON);
        let on_change: Option<fn(u8) -> Message> = value.map(|_| Message::Light as fn(u8) -> _);
        panels::slider_row(icon, value.unwrap_or(0), on_change)
    }

    fn energy_group(&self) -> Element<'_, Message> {
        let (state, note) = match &self.energy {
            Load::Ready(state) => (Some(state), self.failed(Service::Energy)),
            Load::Loading => (None, None),
            Load::Failed(_) => (None, Some(words::ENERGY_ABSENT)),
        };
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
            ui::body(line)
                .font(typography::UI_SEMIBOLD)
                .color(tint)
                .wrapping(text::Wrapping::None),
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
            container(name).width(Length::Fill).clip(true),
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
        let head = row![back, title, space::horizontal(), ui::toggle(enabled, switch)]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center);
        let body: Element<'_, Message> = match wifi {
            Some(wifi) if wifi.enabled && !wifi.networks.is_empty() => {
                let rows = wifi.networks.iter().map(|n| self.network_row(n));
                scrollable(Column::with_children(rows).spacing(metrics::GAP_TIGHT))
                    .height(Length::Fill)
                    .into()
            }
            Some(wifi) if wifi.enabled => notice(words::NO_NETWORK),
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
        let p = color::palette();
        let glyph = if network.security == Security::Open {
            Glyph::Wifi
        } else {
            Glyph::Padlock
        };
        let leading = panels::glyph(glyph, p.text_secondary).into();
        let trailing = if network.active {
            Trailing::Checked
        } else {
            Trailing::None
        };
        let join = Some(Message::Join(network.clone()));
        let item = ui::menu_item_with(Some(leading), network.ssid.as_str(), trailing, join);
        let joining = self
            .joining
            .as_ref()
            .is_some_and(|j| j.path == network.path);
        if !joining {
            return item.into();
        }
        let inset = Padding::from([metrics::GAP_TIGHT * 2.0, metrics::MENU_ITEM_PADDING]);
        if network.security == Security::Other {
            let line = ui::description(words::ENTERPRISE);
            return column![item, container(line).padding(inset)].into();
        }
        let field = ui::text_field(words::PASSWORD, &self.password, Some(Message::Password))
            .secure(true)
            .on_submit(Message::SubmitPassword)
            .width(Length::Fill);
        let ready = (!self.password.is_empty()).then_some(Message::SubmitPassword);
        let ask = row![field, ui::primary_button(words::JOIN, ready)]
            .spacing(metrics::GAP_INLINE / 2.0)
            .align_y(Alignment::Center);
        column![item, container(ask).padding(inset)].into()
    }
}

fn switch_wifi(_: bool) -> Message {
    Message::ToggleWifi
}

/// The tile of the game mode: there is no game mode yet.
fn game_tile<'a>() -> Element<'a, Message> {
    panels::tile(
        Glyph::Gamepad,
        words::GAME_MODE,
        words::SOON.to_owned(),
        false,
        None,
        None,
    )
}

/// A labelled group (« Carte graphique », « Énergie »): its label, a note on its right, its
/// control below.
fn group<'a>(
    label: &'a str,
    note: Option<&'a str>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    let small = |s: &'a str| {
        text(s)
            .font(typography::UI)
            .size(typography::SMALL_SIZE)
    };
    let mut head = row![small(label).style(panels::secondary_text)];
    if let Some(note) = note {
        head = head
            .push(space::horizontal())
            .push(small(note).style(panels::weekday_text));
    }
    column![head, control].spacing(panels::GROUP_GAP).into()
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
        let _ = quick.update(Message::ToggleDoNotDisturb);
        assert_eq!(quick.do_not_disturb, Some(true));
        let (config, _) = NotificationsConfig::load(&file).unwrap();
        assert!(config.do_not_disturb);
        let _ = quick.update(Message::ToggleDoNotDisturb);
        assert!(!NotificationsConfig::load(&file).unwrap().0.do_not_disturb);
        // A file that cannot be written: « Échec » under the tile.
        let mut blocked = Quick {
            settings: Some(PathBuf::from("/proc/no/notifications.conf")),
            ..Quick::default()
        };
        let _ = blocked.update(Message::ToggleDoNotDisturb);
        assert!(matches!(blocked.failure, Some((Service::DoNotDisturb, _))));
        assert_eq!(blocked.failed(Service::DoNotDisturb), Some(words::FAILED));
        let _ = std::fs::remove_dir_all(&dir);
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
