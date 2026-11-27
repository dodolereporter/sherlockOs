//! The application: its state, its messages and what each one changes. A setting changed is
//! written to its file at once, or `SAVE_DELAY` after the last step of a slider; the program
//! that reads the file follows it live.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use appearance_config::{Mode, TimeOfDay};
use desktop_config::touchpad::{self, Click};
use dock_config::{AutoHide, Position};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{image, operation};
use iced::{Event, Subscription, Task, event, window};
use sherlock_ui::color::{Theme, ThemeFade};
use sherlock_ui::control_room::{self as cr, texts};
use sherlock_ui::focus;
use sherlock_ui::motion::{Animation, Fade, SCALE_PRECISION};
use sherlock_ui::{dock, wallpapers};

use mime_apps::Catalog;
use system_services::Problem;
use system_services::audio::{Device, Pactl};

use crate::apps::{self, Choice, Kind};
use crate::search::{self, Hit, Page, Setting};
use crate::settings::{File, Paths, Settings};
use crate::sound::{self, Audio};
use crate::thumbnails::{self, Thumbnail};

/// Identifier of the search field.
pub const SEARCH: &str = "search";

/// What the application starts with.
#[derive(Debug, Clone)]
pub struct Config {
    pub paths: Paths,
    pub page: Page,
}

impl Config {
    /// The files of the environment, the first page open. `None` without a home folder.
    pub fn from_env() -> Option<Self> {
        Some(Self {
            paths: Paths::from_env()?,
            page: Page::Appearance,
        })
    }
}

/// An hour of the automatic theme, in minutes since midnight, as a dropdown menu shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hour(pub u32);

impl std::fmt::Display for Hour {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&texts::hour(self.0))
    }
}

/// A delay before the lock, in minutes (0: never), as a dropdown menu shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Delay(pub u32);

impl std::fmt::Display for Delay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            0 => f.write_str(texts::NEVER),
            60 => f.write_str(texts::ONE_HOUR),
            n => f.write_str(&texts::minutes(n)),
        }
    }
}

/// The delays the lock page offers, and `current` if it is another one.
pub(crate) fn delays(current: u32) -> Vec<Delay> {
    let mut delays: Vec<Delay> = cr::LOCK_DELAYS.iter().copied().map(Delay).collect();
    if !delays.contains(&Delay(current)) {
        delays.push(Delay(current));
        // « Jamais » stays last.
        delays.sort_by_key(|d| if d.0 == 0 { u32::MAX } else { d.0 });
    }
    delays
}

/// What the sound page knows of the devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sound {
    Loading,
    Ready(Audio),
    Unavailable,
}

/// A setting changed by the interface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    Mode(Mode),
    LightFrom(Hour),
    DarkFrom(Hour),
    Wallpaper(&'static str),
    FollowTheme(bool),
    DockSize(u8),
    Magnification(bool),
    MagnificationFactor(u8),
    AutoHide(AutoHide),
    Position(Position),
    Click(Click),
    TapToClick(bool),
    NaturalScroll(bool),
    Speed(u8),
    Typing(bool),
    ShowClicks(bool),
    LockAfter(Delay),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowAction {
    Drag,
    Minimize,
    Maximize,
    Close,
}

#[derive(Debug, Clone)]
pub enum Message {
    Go(Page),
    Search(String),
    /// Enter in the search field: the first setting found.
    OpenFirst,
    Open(Setting),
    ToggleSidebar,
    Escape,
    FocusSearch,
    Focus(focus::Direction),
    Frame(Instant),
    Theme(Theme),
    Window(WindowAction),
    Thumbnails(Arc<Vec<Thumbnail>>),
    Set(Change),
    /// The devices read, or the reason they are not.
    Audio(Result<Audio, Problem>),
    SetSound(sound::Change),
    /// `pactl` made the change at the head of the queue, and read the devices again.
    SoundSet(Result<Audio, Problem>),
    Catalog(Arc<Catalog>),
    SetApp(Kind, Choice),
}

/// Something that comes in and goes out: a spring from 0 to 1, and a fade.
#[derive(Debug, Clone, Copy)]
pub struct Slide {
    spring: Animation,
    fade: Fade,
}

impl Slide {
    pub fn at_rest(shown: bool) -> Self {
        let value = if shown { 1.0 } else { 0.0 };
        Self {
            spring: Animation::new(cr::PAGE_SPRING, value, value, Duration::ZERO),
            fade: Fade::new(value as f32, value as f32, Duration::ZERO, Duration::ZERO),
        }
    }

    /// Coming in from nothing at `now`.
    pub fn coming(now: Duration) -> Self {
        Self {
            spring: Animation::new(cr::PAGE_SPRING, 0.0, 1.0, now),
            fade: Fade::new(0.0, 1.0, now, cr::PAGE_FADE),
        }
    }

    /// Heads for shown or hidden from where it is.
    pub fn show(&mut self, now: Duration, shown: bool) {
        let target = if shown { 1.0 } else { 0.0 };
        if self.spring.target() != target {
            self.spring.retarget(now, target);
            self.fade.retarget(now, target as f32, cr::SIDEBAR_FADE);
        }
    }

    pub fn shown(&self) -> bool {
        self.spring.target() == 1.0
    }

    pub fn running(&self, now: Duration) -> bool {
        !self.fade.is_done(now) || !self.spring.is_done(now, SCALE_PRECISION)
    }

    /// From 0, hidden, to 1, in place: a little beyond on the spring's overshoot.
    pub fn progress(&self, now: Duration) -> f32 {
        self.spring.value(now) as f32
    }

    pub fn opacity(&self, now: Duration) -> f32 {
        self.fade.value(now).clamp(0.0, 1.0)
    }
}

pub struct App {
    pub(crate) paths: Paths,
    pub(crate) settings: Settings,
    pub(crate) page: Page,
    /// The page coming in.
    pub(crate) page_slide: Slide,
    pub(crate) query: String,
    pub(crate) hits: Vec<Hit>,
    /// The setting opened from the search, lit while it fades.
    pub(crate) spotlight: Option<(Setting, Fade)>,
    /// The folded sidebar, over the page in a narrow window.
    pub(crate) sidebar: Slide,
    pub(crate) thumbnails: HashMap<&'static str, image::Handle>,
    pub(crate) sound: Sound,
    /// The installed applications and their associations, once read.
    pub(crate) catalog: Option<Arc<Catalog>>,
    /// Files that cannot be changed: unreadable, not text, or not writable. Their settings
    /// are shown dimmed, and never written.
    pub(crate) locked: Vec<File>,
    /// Changes not written yet, in their order, and when to write them.
    pending: Vec<Change>,
    /// Volumes waiting for their slider to rest.
    volumes: Vec<(Device, u8)>,
    /// What `pactl` is still to be asked, in order, the last volume of each device only, and
    /// whether it is busy: one call at a time, as the media keys do.
    pub(crate) sound_queue: Vec<sound::Change>,
    pub(crate) sound_busy: bool,
    save_at: Option<Instant>,
    pub(crate) theme: ThemeFade,
    origin: Instant,
}

impl App {
    pub fn new(config: Config) -> (Self, Task<Message>) {
        let (settings, locked) = Settings::load(&config.paths);
        let dirs = config.paths.wallpaper_dirs.clone();
        let app = Self {
            paths: config.paths,
            settings,
            page: config.page,
            page_slide: Slide::at_rest(true),
            query: String::new(),
            hits: Vec::new(),
            spotlight: None,
            sidebar: Slide::at_rest(false),
            thumbnails: HashMap::new(),
            sound: Sound::Loading,
            catalog: None,
            locked,
            sound_queue: Vec::new(),
            sound_busy: false,
            pending: Vec::new(),
            volumes: Vec::new(),
            save_at: None,
            theme: ThemeFade::default(),
            origin: Instant::now(),
        };
        let load = Task::perform(async move { thumbnails::load(&dirs) }, |found| {
            Message::Thumbnails(Arc::new(found))
        });
        let pactl = app.pactl();
        let audio = Task::perform(async move { sound::read(&pactl) }, Message::Audio);
        let catalog = app.load_catalog();
        (app, Task::batch([load, audio, catalog]))
    }

    fn pactl(&self) -> Pactl {
        Pactl {
            program: self.paths.pactl.clone(),
        }
    }

    /// Reads the applications and their associations off the interface thread.
    fn load_catalog(&self) -> Task<Message> {
        let (dirs, desktops, locales) = (
            self.paths.apps.clone(),
            self.paths.desktops.clone(),
            self.paths.locales.clone(),
        );
        Task::perform(
            async move { Arc::new(Catalog::load(dirs, &desktops, &locales)) },
            Message::Catalog,
        )
    }

    /// Time on the clock of the animations.
    pub(crate) fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    pub fn title(&self) -> String {
        texts::APP_NAME.to_owned()
    }

    fn animating(&self) -> bool {
        let now = self.now();
        self.page_slide.running(now)
            || self.sidebar.running(now)
            || self.spotlight.is_some()
            || self.theme.running()
            || self.save_at.is_some()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let tab = keyboard::listen()
            .filter_map(focus::tab)
            .map(Message::Focus);
        let keys = event::listen_with(|event, _, _| match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                match key.as_ref() {
                    Key::Named(Named::Escape) => Some(Message::Escape),
                    Key::Character("f") if modifiers.command() => Some(Message::FocusSearch),
                    _ => None,
                }
            }
            Event::Window(window::Event::CloseRequested) => {
                Some(Message::Window(WindowAction::Close))
            }
            _ => None,
        });
        let appearance = Subscription::run_with(self.paths.appearance.clone(), |path| {
            let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
            let follow = appearance_config::follow_themes(path.clone(), move |theme| {
                sender.unbounded_send(theme).is_ok()
            });
            if let Err(error) = follow {
                tracing::warn!(%error, "Control Room will not follow the appearance file");
            }
            receiver
        })
        .map(Message::Theme);
        Subscription::batch([frames, tab, keys, appearance])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let now = self.now();
        match message {
            Message::Go(page) => self.go(page),
            Message::Search(query) => {
                self.hits = search::search(&query);
                self.query = query;
            }
            Message::OpenFirst => {
                if let Some(hit) = self.hits.first() {
                    return self.update(Message::Open(hit.setting));
                }
            }
            Message::Open(setting) => {
                self.go(setting.page());
                let fade = Fade::new(1.0, 0.0, now, cr::SPOTLIGHT);
                self.spotlight = Some((setting, fade));
            }
            Message::ToggleSidebar => {
                let shown = !self.sidebar.shown();
                self.sidebar.show(now, shown);
            }
            Message::Escape => {
                if self.sidebar.shown() {
                    self.sidebar.show(now, false);
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.hits.clear();
                }
            }
            Message::FocusSearch => return operation::focus(SEARCH),
            Message::Focus(direction) => return focus::advance(direction),
            Message::Frame(instant) => {
                self.theme.tick(instant);
                if self.spotlight.as_ref().is_some_and(|(_, f)| f.is_done(now)) {
                    self.spotlight = None;
                }
                if self.save_at.is_some_and(|at| instant >= at) {
                    return self.flush();
                }
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Window(action) => {
                // Closing sets the volumes still waiting first, then closes.
                let waiting = match action {
                    WindowAction::Close => self.flush(),
                    _ => Task::none(),
                };
                let window = window::latest().and_then(move |id| match action {
                    WindowAction::Drag => window::drag(id),
                    WindowAction::Minimize => window::minimize(id, true),
                    WindowAction::Maximize => window::toggle_maximize(id),
                    WindowAction::Close => window::close(id),
                });
                return waiting.chain(window);
            }
            Message::Thumbnails(found) => {
                self.thumbnails = found
                    .iter()
                    .map(|t| {
                        let pixels = t.pixels.clone();
                        (t.id, image::Handle::from_rgba(t.width, t.height, pixels))
                    })
                    .collect();
            }
            Message::Set(change) => {
                if !self.can_change(file_of(change)) {
                    return Task::none();
                }
                // Shown at once; written from the file as it is on the disk when it is written.
                apply(&mut self.settings, change);
                self.pending.push(change);
                if is_continuous(change) {
                    self.save_at = Some(Instant::now() + cr::SAVE_DELAY);
                } else {
                    return self.flush();
                }
            }
            Message::SoundSet(read) => {
                self.sound_busy = false;
                let next = self.next_sound();
                // While other changes wait, the page keeps showing them, not this answer.
                if self.sound_busy {
                    return next;
                }
                return self.update(Message::Audio(read));
            }
            Message::Audio(read) => {
                self.sound = match read {
                    Ok(audio) => Sound::Ready(audio),
                    Err(problem) => {
                        tracing::warn!(%problem, "no sound devices");
                        Sound::Unavailable
                    }
                };
            }
            Message::SetSound(change) => {
                // Shown at once; the volume is set once the slider rests.
                if let Sound::Ready(audio) = &mut self.sound {
                    audio.show(&change);
                }
                match change {
                    sound::Change::Volume(device, v) => {
                        self.volumes.retain(|(d, _)| *d != device);
                        self.volumes.push((device, v));
                        self.save_at = Some(Instant::now() + cr::SAVE_DELAY);
                    }
                    change => {
                        self.queue_sound(change);
                        return self.next_sound();
                    }
                }
            }
            Message::Catalog(catalog) => self.catalog = Some(catalog),
            Message::SetApp(kind, choice) => {
                let paths = self.paths.clone();
                let reload = self.load_catalog();
                let write = Task::perform(
                    async move { apps::set(&paths.apps, &paths.desktops, kind, &choice.id) },
                    move |written| {
                        if let Err(error) = written {
                            tracing::warn!(%error, ?kind, "default application not written");
                        }
                    },
                );
                return write.discard().chain(reload);
            }
        }
        Task::none()
    }

    /// Asks `pactl` for `change` off the interface thread, then shows the devices again.
    /// Puts `change` in the queue of `pactl`: a volume replaces the one of its device still
    /// waiting, in its place; a choice of device goes last.
    fn queue_sound(&mut self, change: sound::Change) {
        if let sound::Change::Volume(device, _) = change {
            let waiting = self
                .sound_queue
                .iter_mut()
                .find(|c| matches!(c, sound::Change::Volume(d, _) if *d == device));
            if let Some(waiting) = waiting {
                *waiting = change;
                return;
            }
        }
        self.sound_queue.push(change);
    }

    /// Asks `pactl` for the change at the head of the queue, unless it is busy: the changes
    /// reach it one after the other, in the order they were made.
    fn next_sound(&mut self) -> Task<Message> {
        if self.sound_busy || self.sound_queue.is_empty() {
            return Task::none();
        }
        let change = self.sound_queue.remove(0);
        self.sound_busy = true;
        let pactl = self.pactl();
        Task::perform(
            async move { sound::apply(&pactl, &change) },
            Message::SoundSet,
        )
    }

    /// Opens `page`: it comes in, and the folded sidebar goes.
    fn go(&mut self, page: Page) {
        let now = self.now();
        if page != self.page {
            self.page = page;
            self.page_slide = Slide::coming(now);
            self.spotlight = None;
        }
        self.sidebar.show(now, false);
    }

    /// The message of `change`, none when its file is locked: the control is disabled.
    pub(crate) fn message(&self, change: Change) -> Option<Message> {
        self.can_change(file_of(change))
            .then_some(Message::Set(change))
    }

    /// `on_change` when the settings of `file` can be changed: a switch or a slider then
    /// answers, and is disabled otherwise.
    pub(crate) fn enabled<T>(
        &self,
        file: File,
        on_change: fn(T) -> Message,
    ) -> Option<fn(T) -> Message> {
        self.can_change(file).then_some(on_change)
    }

    /// Whether the settings of `file` can be changed.
    pub(crate) fn can_change(&self, file: File) -> bool {
        !self.locked.contains(&file)
    }

    /// Writes the changes waiting, file by file: each file is read again as it is now, takes
    /// its changes, and only the keys they touch are written (`Settings::update`). A file that
    /// cannot be read or written goes to the log and becomes locked.
    /// The volumes waiting go to `pactl`, in order: the task returned.
    pub fn flush(&mut self) -> Task<Message> {
        self.save_at = None;
        let changes = std::mem::take(&mut self.pending);
        for file in File::ALL {
            let mine: Vec<Change> = changes
                .iter()
                .copied()
                .filter(|c| file_of(*c) == file)
                .collect();
            if mine.is_empty() || !self.can_change(file) {
                continue;
            }
            let updated = self.settings.update(file, &self.paths, |settings| {
                for change in mine {
                    apply(settings, change);
                }
            });
            if let Err(error) = updated {
                tracing::warn!(%error, ?file, "setting not written: the file is now locked");
                self.locked.push(file);
            }
        }
        for (device, v) in std::mem::take(&mut self.volumes) {
            self.queue_sound(sound::Change::Volume(device, v));
        }
        self.next_sound()
    }

    /// Puts the animations at their end, for a capture.
    pub fn settle(&mut self) {
        self.page_slide = Slide::at_rest(true);
        self.sidebar = Slide::at_rest(self.sidebar.shown());
        self.spotlight = None;
    }
}

/// The file that holds a change.
pub(crate) fn file_of(change: Change) -> File {
    match change {
        Change::Mode(_) | Change::LightFrom(_) | Change::DarkFrom(_) => File::Appearance,
        Change::Wallpaper(_) | Change::FollowTheme(_) => File::Wallpaper,
        Change::DockSize(_)
        | Change::Magnification(_)
        | Change::MagnificationFactor(_)
        | Change::AutoHide(_)
        | Change::Position(_) => File::Dock,
        Change::ShowClicks(_) => File::Pointer,
        Change::LockAfter(_) => File::Lock,
        _ => File::Touchpad,
    }
}

/// Makes the change in the settings.
fn apply(s: &mut Settings, change: Change) {
    let time = |Hour(minutes): Hour| TimeOfDay::new(minutes / 60 % 24, minutes % 60, 0);
    match change {
        Change::Mode(mode) => s.appearance.mode = mode,
        Change::LightFrom(hour) => {
            if let Some(t) = time(hour).filter(|t| *t != s.appearance.dark_from) {
                s.appearance.light_from = t;
            }
        }
        Change::DarkFrom(hour) => {
            if let Some(t) = time(hour).filter(|t| *t != s.appearance.light_from) {
                s.appearance.dark_from = t;
            }
        }
        Change::Wallpaper(id) => {
            s.wallpaper.wallpaper = Some(id.to_owned());
            s.wallpaper.path = None;
        }
        Change::FollowTheme(on) => s.wallpaper.follow_theme = on,
        Change::DockSize(v) => s.dock.icon_size = from_share(v, ICON_SIZES).round(),
        Change::Magnification(on) => s.dock.magnification = on,
        Change::MagnificationFactor(v) => {
            s.dock.magnification_factor = (from_share(v, FACTORS) * 10.0).round() / 10.0;
        }
        Change::AutoHide(when) => s.dock.auto_hide = when,
        Change::Position(edge) => s.dock.position = edge,
        Change::Click(click) => s.touchpad.click = click,
        Change::TapToClick(on) => s.touchpad.tap_to_click = on,
        Change::NaturalScroll(on) => s.touchpad.natural_scroll = on,
        Change::Speed(v) => {
            let speed = f64::from(from_share(v, SPEEDS));
            s.touchpad.speed = (speed * 100.0).round() / 100.0;
        }
        Change::Typing(on) => s.touchpad.disable_while_typing = on,
        Change::ShowClicks(on) => s.pointer.show_clicks = on,
        Change::LockAfter(Delay(minutes)) => s.lock.idle_minutes = minutes,
    }
}

impl App {
    /// The id of the wallpaper chosen in the collection, `None` for an image of the user's.
    pub(crate) fn wallpaper(&self) -> Option<&str> {
        let config = &self.settings.wallpaper;
        match config.path {
            Some(_) => None,
            None => Some(config.wallpaper.as_deref().unwrap_or(wallpapers::DEFAULT)),
        }
    }

    /// The sliders, from 0 to 100.
    pub(crate) fn dock_size(&self) -> u8 {
        to_share(self.settings.dock.icon_size, ICON_SIZES)
    }

    pub(crate) fn magnification_factor(&self) -> u8 {
        to_share(self.settings.dock.magnification_factor, FACTORS)
    }

    pub(crate) fn speed(&self) -> u8 {
        to_share(self.settings.touchpad.speed as f32, SPEEDS)
    }
}

/// The bounds of each slider.
const ICON_SIZES: (f32, f32) = (dock::ICON_SIZE_MIN, dock::ICON_SIZE_MAX);
const FACTORS: (f32, f32) = (dock::MAGNIFICATION_MIN, dock::MAGNIFICATION_MAX);
const SPEEDS: (f32, f32) = (
    touchpad::SPEED_RANGE.0 as f32,
    touchpad::SPEED_RANGE.1 as f32,
);

/// A slider moves by steps: its file is written once it rests.
fn is_continuous(change: Change) -> bool {
    matches!(
        change,
        Change::DockSize(_) | Change::MagnificationFactor(_) | Change::Speed(_)
    )
}

/// The value at `share` percent of `(low, high)`.
fn from_share(share: u8, (low, high): (f32, f32)) -> f32 {
    low + (high - low) * f32::from(share.min(100)) / 100.0
}

/// Where `value` is between `low` and `high`, in percent.
fn to_share(value: f32, (low, high): (f32, f32)) -> u8 {
    let share = (value - low) / (high - low) * 100.0;
    share.round().clamp(0.0, 100.0) as u8
}

/// The hours a dropdown of the automatic theme offers: every `HOUR_STEP` minutes, and
/// `current` if it falls between two; never `other`, the hour of the other theme.
pub(crate) fn hours(current: TimeOfDay, other: TimeOfDay) -> Vec<Hour> {
    let mut hours: Vec<Hour> = (0..24 * 60)
        .step_by(cr::HOUR_STEP as usize)
        .map(Hour)
        .collect();
    let current = Hour(current.minutes());
    if !hours.contains(&current) {
        hours.push(current);
        hours.sort();
    }
    hours.retain(|h| h.0 != other.minutes());
    hours
}
