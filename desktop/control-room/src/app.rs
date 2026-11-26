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

use crate::search::{self, Hit, Page, Setting};
use crate::settings::{File, Paths, Settings};
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
            page: Page::ALL[0],
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
    /// Files changed and not written yet, and when to write them.
    pending: Vec<File>,
    save_at: Option<Instant>,
    pub(crate) theme: ThemeFade,
    origin: Instant,
}

impl App {
    pub fn new(config: Config) -> (Self, Task<Message>) {
        let settings = Settings::load(&config.paths);
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
            pending: Vec::new(),
            save_at: None,
            theme: ThemeFade::default(),
            origin: Instant::now(),
        };
        let load = Task::perform(async move { thumbnails::load(&dirs) }, |found| {
            Message::Thumbnails(Arc::new(found))
        });
        (app, load)
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
                    self.flush();
                }
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Window(action) => {
                if action == WindowAction::Close {
                    self.flush();
                }
                return window::latest().and_then(move |id| match action {
                    WindowAction::Drag => window::drag(id),
                    WindowAction::Minimize => window::minimize(id, true),
                    WindowAction::Maximize => window::toggle_maximize(id),
                    WindowAction::Close => window::close(id),
                });
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
                let file = self.apply(change);
                if !self.pending.contains(&file) {
                    self.pending.push(file);
                }
                if is_continuous(change) {
                    self.save_at = Some(Instant::now() + cr::SAVE_DELAY);
                } else {
                    self.flush();
                }
            }
        }
        Task::none()
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

    /// Writes the files changed. A file that cannot be written goes to the log; the setting
    /// stays as chosen in the window.
    pub fn flush(&mut self) {
        self.save_at = None;
        for file in std::mem::take(&mut self.pending) {
            if let Err(error) = self.settings.save(file, &self.paths) {
                tracing::warn!(%error, ?file, "setting not written");
            }
        }
    }

    /// Puts the animations at their end, for a capture.
    pub fn settle(&mut self) {
        self.page_slide = Slide::at_rest(true);
        self.sidebar = Slide::at_rest(self.sidebar.shown());
        self.spotlight = None;
    }

    /// Makes the change in the settings; returns the file that holds it.
    fn apply(&mut self, change: Change) -> File {
        let s = &mut self.settings;
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
        }
        match change {
            Change::Mode(_) | Change::LightFrom(_) | Change::DarkFrom(_) => File::Appearance,
            Change::Wallpaper(_) | Change::FollowTheme(_) => File::Wallpaper,
            Change::DockSize(_)
            | Change::Magnification(_)
            | Change::MagnificationFactor(_)
            | Change::AutoHide(_)
            | Change::Position(_) => File::Dock,
            Change::ShowClicks(_) => File::Pointer,
            _ => File::Touchpad,
        }
    }

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
