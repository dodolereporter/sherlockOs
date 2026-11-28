//! Dock state, messages and update. The views are in `view.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dock_config::{DockConfig, Position as Edge};
use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::window::Id;
use iced::{Point, Size, Subscription, Task};
use iced::{keyboard, mouse, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{PixelSize, PopupAnchor, PopupGravity, Position};
use iced_exwlshell::settings::LayerShellSettings;
use iced_exwlshell::shell::{ShellEvent, ShellReceiver};
use iced_exwlshell::to_layer_message;
use sherlock_ui::color::{Theme, ThemeFade};
use sherlock_ui::dock;
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::motion::Fade;

use crate::anim::{Bounce, Launch, Leaving, Poof, Track};
use crate::drag::{self, Gesture, Press};
use crate::entry::Catalog;
use crate::hide::{self, Hide};
use crate::icons::IconLookup;
use crate::items::{self, Builder, Glyph, Item, Kind, Target};
use crate::launch::{self, What};
use crate::layout::{self, Flow, Geometry, Layout};
use crate::menu::{self, Abilities, Entry, Key, Line, Listed, Unfold};
use crate::windows::{self, Click, Control, Place, Request, Window, Windows};

/// The menu of an icon, or the dock's own, while it is open.
#[derive(Debug, Clone)]
pub(crate) struct OpenMenu {
    pub(crate) id: Id,
    /// The icon the menu belongs to, by identity; `None` for the dock's own menu.
    pub(crate) target: Option<Target>,
    pub(crate) entries: Vec<Line>,
    pub(crate) cursor: Option<usize>,
    /// The grid of a stack instead of entries (lot D-21k): how many items it was opened for.
    pub(crate) grid: Option<usize>,
    /// Opening, open, or closing before its popup goes.
    pub(crate) unfold: Unfold,
}

/// A pointer event on the dock surface, in its coordinates.
#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Moved(Point),
    Left,
    Pressed(mouse::Button),
    /// The left button came up over the dock. Also seen by the gesture subscription, which
    /// starts after the press and might miss a quick click.
    Released,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    /// The trash filled or emptied (lot D-21j).
    Trash(bool),
    /// The downloads stack read again, or none (lot D-21k).
    Stack(Option<crate::stack::Stack>),
    /// An item of the open stack picked: opened with its application.
    StackOpen(usize),
    /// « Ouvrir dans Evidence » in the open stack.
    StackFolder,
    /// The applications that open with the session, read (lot D-21i).
    Autostart(std::collections::BTreeSet<String>),
    /// The Wayland handles of the dock's surface, for the applications dragged from the
    /// Applications view (lot D-41d).
    DndReady(Option<crate::dnd::Handles>),
    /// An application dragged over the dock, dropped on it, or gone.
    Dnd(crate::dnd::Event),
    Surface(Id),
    Pointer(Pointer),
    Frame(Instant),
    Windows(Windows),
    /// The screen a surface is on, from iced_exwlshell.
    Screen(Id, Option<Screen>),
    /// The pointer while the button is held on an icon.
    Gesture(Gesture),
    /// The configuration file changed.
    Config(Box<DockConfig>),
    /// The icons looked up again, off the interface thread, for the icon theme of the file.
    Icons(Option<String>, Box<IconLookup>),
    Key(Key),
    /// The modifiers held: Ctrl with a click asks for a new window.
    Modifiers(keyboard::Modifiers),
    /// A scroll over the dock: a swipe down over an icon opens App Expose.
    Scroll(mouse::ScrollDelta),
    Focus(Direction),
    MenuHover(usize),
    MenuPick(Entry),
    Closed(Id),
    /// The theme of the appearance file, at start then at each change.
    Theme(Theme),
    /// An icon drawn by the worker thread (`raster::drawn`): it shows at the next frame.
    IconDrawn,
}

/// A screen, as the dock needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    /// Its `wl_output` name.
    pub name: Option<String>,
    /// Its logical width and height.
    pub size: Option<(i32, i32)>,
}

/// An icon on its screen: the icon, the screen's name, its rectangle there.
type IconPlace = (Target, String, (i32, i32, i32, i32));

/// What the compositor was last told of the surface.
#[derive(Debug, Clone, Copy, Default)]
struct Sent {
    margin: Option<i32>,
    zone: Option<i32>,
    /// Whether the input region was the band of a hidden dock.
    hidden: Option<bool>,
}

/// What the dock reads at startup.
pub struct Sources {
    pub catalog: Catalog,
    pub lookup: IconLookup,
    pub config: DockConfig,
    pub config_path: Option<PathBuf>,
    /// The pinned applications: from the configuration, or chosen among the installed ones.
    pub pinned: Vec<String>,
    /// Data directories and home, to look icons up again when the theme changes.
    pub data_dirs: Vec<PathBuf>,
    pub home: Option<PathBuf>,
    pub gtk_theme: Option<String>,
    /// What acts on the windows: the compositor's protocol, or a fake in tests.
    pub control: Arc<dyn Control>,
    /// The « Applications » button first (D-41): always in the session; the tests of the other
    /// icons leave it out, so that their places stay those of the design board.
    pub applications: bool,
}

pub struct Dock {
    pub items: Vec<Item>,
    /// Index of the first open application that is not pinned: the separator goes before it.
    pub separator: Option<usize>,
    /// Desktop file ids of the pinned applications, from the configuration or the defaults,
    /// with Sherlock's token (`dock_config::SHERLOCK`) once it has a place of its own.
    pub pinned: Vec<String>,
    /// Sherlock is in the dock.
    pub sherlock: bool,
    pub(crate) sources: Sources,
    glyphs: HashMap<String, Glyph>,
    pub geometry: Geometry,
    /// The trash at the end of the dock, full or empty, once its watcher said (lot D-21j).
    pub trash: Option<bool>,
    /// The downloads stack, once its watcher read it (lot D-21k).
    pub stack: Option<crate::stack::Stack>,
    /// « Vider la corbeille » clicked once, and when: a second click empties it.
    trash_armed: Option<Instant>,
    /// The applications that open with the session, read off the interface thread (lot
    /// D-21i): « Ouvrir à l'ouverture de session » is checked for them.
    pub autostart: std::collections::BTreeSet<String>,
    /// The geometry of the settings, before it is fitted to the screen (`Geometry::fitted`).
    pub base: Geometry,
    /// Size of the dock surface.
    pub surface_size: Size,
    surface: Option<Id>,
    /// The animations of each icon: size under the pointer, arrival, offset from its place.
    pub(crate) tracks: Vec<Track>,
    /// Icons that left the dock, drawn until they are gone.
    pub leaving: Vec<Leaving>,
    /// Applications started from the dock, bouncing until their first window.
    pub launches: Vec<Launch>,
    /// Launches waiting for the activation token of their click (lot Q-52).
    asks: Vec<iced_exwlshell::reexport::ActivationTokenSink>,
    /// Applications whose window asks for attention, bouncing a few hops (lot Q-52), and the
    /// windows that asked at the last list: a window that keeps asking bounces once.
    pub calls: Vec<(Target, Bounce)>,
    calling: Vec<u64>,
    /// The icon let go out of the dock, which vanishes where it is.
    dropping: Option<Target>,
    /// The puffs of smoke of the applications unpinned by a drag out of the dock.
    pub poofs: Vec<Poof>,
    /// The left button held on an icon: a click, or a drag.
    pub(crate) press: Option<Press>,
    /// Opacity of the label, and the icon it names.
    pub(crate) label: Fade,
    pub(crate) label_item: Option<usize>,
    /// The pointer is over the dock.
    hovering: bool,
    pointer: Option<Point>,
    origin: Instant,
    /// Time of the last event, from `origin`: the moment the view draws.
    pub(crate) now: Duration,
    pub windows: Option<Windows>,
    /// Icon with the keyboard focus, kept in our state like the bar does.
    pub(crate) focus: Option<usize>,
    pub(crate) menu: Option<OpenMenu>,
    /// The modifiers held, as the keyboard last told the dock.
    modifiers: keyboard::Modifiers,
    /// The swipe down in progress: its icon, how far down it went (`NEG_INFINITY` once it
    /// opened App Expose), when its last scroll came.
    swipe: Option<(Target, f32, Duration)>,
    /// Frames drawn since `frames_since`, for the frame rate in the debug log.
    frames: u32,
    frames_since: Duration,
    /// The theme of the appearance file, shown with a fade.
    pub theme: ThemeFade,
    /// When the dock hides, and its slide.
    pub hide: Hide,
    /// The screen of each surface, reported by `screens`.
    outputs: HashMap<Id, Option<Screen>>,
    screens: Option<ShellReceiver>,
    sent: Sent,
    /// The icons last placed on their screen (`Control::place`).
    placed: Vec<IconPlace>,
    /// Writes the pinned list on a thread of its own.
    saver: crate::save::Saver,
    /// The thread of the drags from the Applications view, kept alive with its link.
    dnd: Option<crate::dnd::Link>,
}

impl Dock {
    pub fn new(sources: Sources) -> Self {
        let geometry = Geometry::from_config(&sources.config);
        let sources_auto_hide = sources.config.auto_hide;
        let mut dock = Self {
            items: Vec::new(),
            separator: None,
            pinned: sources.pinned.clone(),
            sherlock: sources.config.sherlock,
            sources,
            glyphs: HashMap::new(),
            geometry,
            trash: None,
            stack: None,
            trash_armed: None,
            autostart: std::collections::BTreeSet::new(),
            base: geometry,
            surface_size: Size::ZERO,
            surface: None,
            tracks: Vec::new(),
            leaving: Vec::new(),
            launches: Vec::new(),
            asks: Vec::new(),
            calls: Vec::new(),
            calling: Vec::new(),
            dropping: None,
            poofs: Vec::new(),
            press: None,
            // Hidden, and already still: no frame for it at startup.
            label: Fade::new(0.0, 0.0, Duration::ZERO, Duration::ZERO),
            label_item: None,
            hovering: false,
            pointer: None,
            origin: Instant::now(),
            now: Duration::ZERO,
            windows: None,
            focus: None,
            menu: None,
            modifiers: keyboard::Modifiers::empty(),
            swipe: None,
            frames: 0,
            frames_since: Duration::ZERO,
            theme: ThemeFade::default(),
            hide: Hide::new(sources_auto_hide),
            outputs: HashMap::new(),
            screens: None,
            sent: Sent::default(),
            placed: Vec::new(),
            saver: crate::save::Saver::default(),
            dnd: None,
        };
        dock.rebuild(false);
        dock
    }

    /// The dock, told of the screen of its surface by `screens`.
    pub fn with_screens(mut self, screens: ShellReceiver) -> Self {
        self.screens = Some(screens);
        self
    }

    /// The surface settings for the dock as it is: shown, hidden or on its way.
    pub fn layer_settings(&self) -> LayerShellSettings {
        let mut settings = crate::surface(self.geometry, self.surface_size);
        settings.margin = crate::margin(self.geometry.position, self.margin());
        settings.exclusive_zone = self.zone();
        settings
    }

    /// Distance from the screen edge to the surface, as the dock slides.
    fn margin(&self) -> i32 {
        let depth = if self.geometry.position.is_vertical() {
            self.surface_size.width
        } else {
            self.surface_size.height
        };
        hide::margin(self.hide.progress(), depth)
    }

    /// The exclusive zone: the gap and the bar's thickness in the `never` mode; otherwise -1,
    /// so that the compositor places it against the screen edge itself, not against what other
    /// surfaces reserve (the top bar, for a dock at the top), and reserves nothing for it: a
    /// dock that hides by itself goes over the windows (lot D-21g).
    fn zone(&self) -> i32 {
        if self.hide.exclusive() {
            (self.geometry.bar_thickness() + self.geometry.gap).round() as i32
        } else {
            -1
        }
    }

    /// The settings' geometry fitted to the screen's length for `count` icons; as it is while
    /// the screen is not known.
    fn fit(&self, count: usize, separator: bool) -> Geometry {
        let Some((width, height)) = self.screen().and_then(|s| s.size) else {
            return self.base;
        };
        let room = if self.base.position.is_vertical() {
            height
        } else {
            width
        };
        self.base.fitted(room as f32, count, separator)
    }

    /// The screen of the dock's surface, once known.
    pub fn screen(&self) -> Option<&Screen> {
        self.surface
            .and_then(|id| self.outputs.get(&id))
            .and_then(Option::as_ref)
    }

    /// A maximized, fullscreen or snapped window is on the dock's screen.
    pub fn covered(&self) -> bool {
        let name = self.screen().and_then(|s| s.name.as_deref());
        match (name, &self.windows) {
            (Some(name), Some(windows)) => windows.covered.contains(name),
            _ => false,
        }
    }

    /// The windows or the screen changed: the dock hides or comes back.
    fn follow_windows(&mut self) -> Task<Message> {
        let now = self.now;
        self.hide.set_covered(self.covered(), self.hovering, now);
        self.place_icons();
        self.apply_hide()
    }

    /// Tells the compositor what changed of the margin, the exclusive zone and the input
    /// region as the dock hides or comes back.
    fn apply_hide(&mut self) -> Task<Message> {
        let Some(id) = self.surface else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        let margin = self.margin();
        if self.sent.margin != Some(margin) {
            self.sent.margin = Some(margin);
            let margin = crate::margin(self.geometry.position, margin);
            tasks.push(Task::done(Message::MarginChange { id, margin }));
        }
        let zone = self.zone();
        if self.sent.zone != Some(zone) {
            self.sent.zone = Some(zone);
            tasks.push(Task::done(Message::ExclusiveZoneChange {
                id,
                zone_size: zone,
            }));
        }
        let hidden = self.hide.hidden();
        if self.sent.hidden != Some(hidden) {
            self.sent.hidden = Some(hidden);
            tasks.push(self.input_region());
            // The genie goes to the edge while hidden, to the icon once shown.
            self.place_icons();
        }
        Task::batch(tasks)
    }

    /// Tells the control where each icon is on its screen, at rest and shown, when that
    /// changed: the compositor keeps it for each window, for the minimize animation. While the
    /// dock is hidden, each icon is laid flat along the edge (lot D-19e): a window minimized
    /// then goes into the edge, where the dock hides, not to an empty place above it.
    fn place_icons(&mut self) {
        let sizes = self.geometry.targets(None, self.items.len());
        let rest = Layout::new(self.geometry, self.surface_size, &sizes, self.separator);
        self.place_icons_as(&rest);
    }

    /// Tells the compositor where the icons are drawn now, magnified under the pointer, before
    /// a window is minimized into one of them (lot D-21d): the genie lands on the icon as it
    /// is seen, not a few pixels beside it where it rests. Sent once, as the request goes; the
    /// icons at rest are sent again when the pointer leaves the dock.
    fn place_drawn_icons(&mut self) {
        let drawn = self.layout(self.now);
        self.place_icons_as(&drawn);
    }

    /// Tells the control where the icons of `layout` are on their screen, when that changed.
    fn place_icons_as(&mut self, layout: &Layout) {
        let Some((name, size)) = self.screen().and_then(|s| Some((s.name.clone()?, s.size?)))
        else {
            return;
        };
        let rest = layout;
        let distance = 0;
        let hidden = self.hide.hidden();
        let placed: Vec<_> = self
            .items
            .iter()
            .zip(&rest.icons)
            .map(|(item, icon)| {
                let rectangle = layout::on_screen(
                    self.geometry.position,
                    self.surface_size,
                    size,
                    distance,
                    *icon,
                );
                let rectangle = if hidden {
                    layout::at_edge(self.geometry.position, size, rectangle)
                } else {
                    rectangle
                };
                (item.target(), name.clone(), rectangle)
            })
            .collect();
        if placed == self.placed {
            return;
        }
        let places = self
            .items
            .iter()
            .zip(&placed)
            .map(|(item, (_, output, rectangle))| {
                let item = item.clone();
                Place {
                    owns: Box::new(move |app_id| item.owns(app_id)),
                    output: output.clone(),
                    rectangle: *rectangle,
                }
            })
            .collect();
        self.placed = placed;
        self.sources.control.place(places);
    }

    /// Builds the icons again from the pinned list and the open windows. Each icon keeps its
    /// animations by identity. With `animate`, a new icon arrives, a gone one leaves, and the
    /// icons that moved start where they were drawn; otherwise everything is in place.
    fn rebuild(&mut self, animate: bool) {
        let now = self.now;
        let before = animate.then(|| self.layout(now));
        let icon_size = (self.geometry.icon * self.geometry.factor).round() as u32;
        let mut builder = Builder {
            applications: self.sources.applications,
            sherlock: self.sherlock,
            trash: self.trash,
            stack: self
                .stack
                .as_ref()
                .map(|s| (s.folder.as_path(), s.name.as_str())),
            catalog: &self.sources.catalog,
            lookup: &self.sources.lookup,
            icon_size,
            glyphs: &mut self.glyphs,
        };
        let (items, separator) = builder.build(&self.pinned, self.windows.as_ref());
        // As many icons as there are now, in the length of the screen (lot D-21l).
        self.geometry = self.fit(items.len(), separator.is_some());
        let old = std::mem::take(&mut self.items);
        let old_separator = self.separator;
        let mut old_tracks: Vec<Option<Track>> = std::mem::take(&mut self.tracks)
            .into_iter()
            .map(Some)
            .collect();
        let side = self.geometry.icon;
        let mut moved = Vec::new();
        for item in &items {
            let place = old.iter().position(|o| o.target() == item.target());
            let track = match place.and_then(|j| old_tracks.get_mut(j)?.take()) {
                Some(track) if animate => track,
                _ if animate => Track::arriving(side, now),
                _ => Track::settled(side),
            };
            moved.push(place);
            self.tracks.push(track);
        }
        let dropping = self.dropping.take();
        match &before {
            Some(before) => {
                for (j, (item, track)) in old.iter().zip(old_tracks).enumerate() {
                    let Some(mut track) = track else { continue };
                    let target = item.target();
                    let next = old[j + 1..]
                        .iter()
                        .map(Item::target)
                        .find(|t| items.iter().any(|i| i.target() == *t));
                    let dropped = (dropping.as_ref() == Some(&target))
                        .then(|| before.icons.get(j).map(|r| before.offset(*r)))
                        .flatten();
                    match dropped {
                        Some(_) => track.vanish(now, dock::DRAG_OUT_OPACITY),
                        None => track.show(now, false),
                    }
                    self.leaving.push(Leaving {
                        item: item.clone(),
                        track,
                        next,
                        unpinned: old_separator.is_some_and(|s| j >= s),
                        dropped,
                    });
                }
            }
            None => self.leaving.clear(),
        }
        if items.len() != old.len() {
            self.label_item = None;
            self.focus = None;
        }
        self.items = items;
        self.separator = separator;
        self.show_dots(animate);
        self.surface_size = self.wanted_surface();
        // Each icon that moved starts where it was drawn, relative to the middle of the dock.
        if let Some(before) = before {
            let after = self.layout(now);
            for (i, place) in moved.into_iter().enumerate() {
                let (Some(j), Some(new)) = (place, after.icons.get(i)) else {
                    continue;
                };
                let Some(old) = before.icons.get(j) else {
                    continue;
                };
                let (from, to) = (before.offset(*old), after.offset(*new));
                self.shift(i, (from.0 - to.0, from.1 - to.1), now);
            }
        }
    }

    /// Each icon's dot shows while its application has a window, green in front; it fades in
    /// and out with `animate`, otherwise it is in place at once.
    fn show_dots(&mut self, animate: bool) {
        let now = self.now;
        for index in 0..self.items.len() {
            let (count, active) = self.open_windows(index).unwrap_or((0, false));
            if let Some(track) = self.tracks.get_mut(index) {
                track.show_dot(now, count > 0, active, animate);
            }
        }
    }

    /// Icon `index` is drawn `by` away from where it was: it starts back there at `now` and
    /// springs to its place.
    fn shift(&mut self, index: usize, by: (f32, f32), now: Duration) {
        let Some(track) = self.tracks.get_mut(index) else {
            return;
        };
        if by.0.abs() < 0.5 && by.1.abs() < 0.5 {
            return;
        }
        let current = track.slot(now).shift;
        track.moved(now, (current.0 + by.0, current.1 + by.1), (0.0, 0.0));
    }

    /// The icons leaving the dock that still take room in the bar.
    pub(crate) fn flowing(&self) -> impl Iterator<Item = &Leaving> {
        self.leaving.iter().filter(|l| l.dropped.is_none())
    }

    /// Size of the surface for the icons and those still closing their room.
    fn wanted_surface(&self) -> Size {
        let separator = self.separator.is_some() || self.flowing().any(|l| l.unpinned);
        self.geometry
            .surface_size(self.items.len() + self.flowing().count(), separator)
    }

    /// The time of the clock, never before the last event's.
    fn clock(&mut self) -> Duration {
        self.now = self.now.max(self.origin.elapsed());
        self.now
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Whether something still moves: the subscription to frames lives only that long.
    pub fn animating(&self) -> bool {
        self.theme.running()
            || self
                .menu
                .as_ref()
                .is_some_and(|m| m.unfold.running(self.now))
            || !self.label.is_done(self.now)
            || !self.leaving.is_empty()
            || !self.launches.is_empty()
            || !self.calls.is_empty()
            || !self.poofs.is_empty()
            || self.tracks.iter().any(|t| !t.is_done(self.now))
            || self.hide.running(self.busy())
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = if self.animating() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        let config = match &self.sources.config_path {
            Some(path) => {
                Subscription::run_with(path.clone(), |path| crate::watch::watch(path.clone()))
                    .map(Message::Config)
            }
            None => Subscription::none(),
        };
        let appearance = match appearance_config::path() {
            Some(path) => Subscription::run_with(path, |path| appearance(path)).map(Message::Theme),
            None => Subscription::none(),
        };
        let stack = match (std::env::home_dir(), config_home()) {
            (Some(home), Some(config)) => {
                Subscription::run_with((home, config), |(home, config)| {
                    crate::stack::watch(home.clone(), config.clone())
                })
                .map(Message::Stack)
            }
            _ => Subscription::none(),
        };
        let trash = match crate::trash::folder() {
            Some(folder) => {
                Subscription::run_with(folder, |folder| crate::trash::watch(folder.clone()))
                    .map(Message::Trash)
            }
            None => Subscription::none(),
        };
        let gesture = if self.press.is_some() {
            iced::event::listen_with(drag::gesture).map(Message::Gesture)
        } else {
            Subscription::none()
        };
        let screens = self
            .screens
            .as_ref()
            .map_or_else(Subscription::none, |events| {
                events.listen().filter_map(on_screen)
            });
        Subscription::batch([
            appearance,
            screens,
            Subscription::run(|| windows::watch(windows::source)).map(Message::Windows),
            gesture,
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            config,
            frames,
            sherlock_ui::raster::drawn().map(|()| Message::IconDrawn),
            trash,
            stack,
        ])
    }

    /// Layout of the dock at `now`.
    pub fn layout(&self, now: Duration) -> Layout {
        let slots: Vec<_> = self.tracks.iter().map(|t| t.slot(now)).collect();
        let place = |next: &Option<Target>| {
            next.as_ref()
                .and_then(|t| self.index_of(t))
                .unwrap_or(self.items.len())
        };
        let leaving = self
            .flowing()
            .map(|l| (place(&l.next), l.unpinned, l.track.slot(now)))
            .collect();
        // The separator opens and closes with the open applications after it.
        let after = |from: usize| {
            slots[from.min(slots.len())..]
                .iter()
                .map(|s| s.room)
                .chain(
                    self.flowing()
                        .filter(|l| l.unpinned)
                        .map(|l| l.track.slot(now).room),
                )
                .fold(0.0, f32::max)
        };
        let separator = match self.separator {
            Some(index) => Some((index, after(index).min(1.0))),
            None if self.flowing().any(|l| l.unpinned) => {
                Some((self.items.len(), after(self.items.len()).min(1.0)))
            }
            None => None,
        };
        let held = self.held().map(|(index, _)| index);
        let flow = Flow {
            slots,
            leaving,
            separator,
            held: held.zip(self.press.as_ref().map(|p| p.pointer)),
        };
        Layout::flow(self.geometry, self.surface_size, &flow)
    }

    /// The icon the pointer drags, and whether it is out of the dock.
    pub fn held(&self) -> Option<(usize, bool)> {
        let press = self.press.as_ref().filter(|p| p.dragging)?;
        Some((self.index_of(&press.target)?, press.out))
    }

    /// The height icon `index` bounces at, at `now`.
    pub fn lift(&self, index: usize, now: Duration) -> f32 {
        let (Some(item), Some(track)) = (self.items.get(index), self.tracks.get(index)) else {
            return 0.0;
        };
        let target = item.target();
        let side = track.size.value(now) as f32;
        let launch = self.launches.iter().find(|l| l.target == target);
        let call = self.calls.iter().find(|c| c.0 == target);
        let launched = launch.map_or(0.0, |l| l.bounce.lift(now, side));
        launched.max(call.map_or(0.0, |c| c.1.lift(now, side)))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.react(message);
        // The launches of this message ask for the token of the click first (lot Q-52).
        let asks = std::mem::take(&mut self.asks);
        match self.surface {
            Some(id) if !asks.is_empty() => {
                let asks = asks
                    .into_iter()
                    .map(|sink| Task::done(Message::RequestActivationToken { id, sink }));
                Task::batch(asks).chain(task)
            }
            _ => task,
        }
    }

    fn react(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surface = Some(id);
                // What `layer_settings` gave the surface when it was made.
                self.sent = Sent {
                    margin: Some(self.margin()),
                    zone: Some(self.zone()),
                    hidden: Some(self.hide.hidden()),
                };
                let dnd = window::run(id, crate::dnd::handles).map(Message::DndReady);
                return Task::batch([
                    self.input_region(),
                    self.follow_windows(),
                    self.read_autostart(),
                    dnd,
                ]);
            }
            Message::Stack(stack) => {
                // The same folder read again: only the grid changes, the dock stays.
                let moved = self.stack.as_ref().map(|s| (&s.folder, &s.name))
                    != stack.as_ref().map(|s| (&s.folder, &s.name));
                self.stack = stack;
                if !moved {
                    return Task::none();
                }
                self.clock();
                let previous = self.geometry;
                return self.refresh(previous);
            }
            Message::StackOpen(index) => {
                if let Some(item) = self.stack.as_ref().and_then(|s| s.items.get(index)) {
                    crate::stack::open(item.path.clone(), item.folder);
                }
                return self.close_menu();
            }
            Message::StackFolder => {
                self.open_stack_folder();
                return self.close_menu();
            }
            Message::Trash(full) => {
                self.trash = Some(full);
                if !full {
                    self.trash_armed = None;
                }
                self.clock();
                let previous = self.geometry;
                return self.refresh(previous);
            }
            Message::Autostart(ids) => self.autostart = ids,
            Message::DndReady(handles) => {
                let Some((link, events)) = handles.and_then(crate::dnd::start) else {
                    return Task::none();
                };
                self.dnd = Some(link);
                return Task::run(events, Message::Dnd);
            }
            Message::Dnd(event) => return self.incoming(event),
            Message::Screen(id, screen) => {
                self.outputs.insert(id, screen);
                self.clock();
                // Another screen, another length: the icons fit it.
                let previous = self.geometry;
                let refreshed = self.refresh(previous);
                return Task::batch([refreshed, self.follow_windows()]);
            }
            Message::Pointer(event) => {
                let now = self.clock();
                return self.pointer(event, now);
            }
            Message::Frame(instant) => {
                self.theme.tick(instant);
                return self.tick(instant.saturating_duration_since(self.origin));
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::IconDrawn => {}
            Message::Windows(windows) => {
                self.clock();
                // The first list is the dock at startup: in place at once.
                let first = self.windows.is_none();
                self.windows = Some(windows);
                self.windows_came();
                let refreshed = self.refresh_as(self.geometry, !first);
                return Task::batch([refreshed, self.follow_windows()]);
            }
            Message::Gesture(gesture) => return self.gesture(gesture),
            Message::Config(config) => return self.configure(*config),
            Message::Icons(theme, lookup) => {
                // A lookup for a theme the file no longer names came late: the newer one comes.
                if theme == self.sources.config.icon_theme {
                    self.sources.lookup = *lookup;
                    self.glyphs.clear();
                    return self.refresh(self.geometry);
                }
            }
            Message::Key(key) => return self.key(key),
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::Scroll(delta) => {
                let now = self.clock();
                self.scroll(delta, now);
            }
            Message::Focus(direction) if self.menu_open() => {
                let key = match direction {
                    Direction::Next => Key::Next,
                    Direction::Previous => Key::Previous,
                };
                return self.key(key);
            }
            Message::Focus(direction) => self.move_focus(direction),
            Message::MenuHover(index) => {
                if let Some(menu) = &mut self.menu
                    && !menu.unfold.closing()
                    && menu.entries.get(index).is_some_and(|l| l.enabled)
                {
                    menu.cursor = Some(index);
                }
            }
            Message::MenuPick(entry) => return self.pick(entry),
            Message::Closed(id) if self.menu.as_ref().is_some_and(|m| m.id == id) => {
                self.menu = None;
                // A dock brought back hides the delay after its menu closes.
                let now = self.clock();
                self.hide.restart(now);
            }
            Message::Closed(id) => {
                self.outputs.remove(&id);
            }
            _ => {}
        }
        Task::none()
    }

    /// Applies a configuration read from the file: settings, icon theme, pinned list.
    pub fn configure(&mut self, config: DockConfig) -> Task<Message> {
        let previous = self.geometry;
        let mut icons = Task::none();
        if config.icon_theme != self.sources.config.icon_theme {
            // Reading the themes' folders takes as long as the disk wants: on a thread of its
            // own, the icons of the old theme shown meanwhile.
            icons = self.look_up_icons(config.icon_theme.clone());
        }
        if let Some(pinned) = &config.pinned {
            self.pinned.clone_from(pinned);
        }
        self.sherlock = config.sherlock;
        self.base = Geometry::from_config(&config);
        self.geometry = self.base;
        let now = self.clock();
        self.hide.set_mode(config.auto_hide, now);
        self.sources.config = config;
        let refreshed = self.refresh(previous);
        Task::batch([refreshed, self.follow_windows(), icons])
    }

    /// Looks the icons up for the icon theme `named` by the file (the GTK one without), on a
    /// thread of its own; `Message::Icons` brings them back.
    fn look_up_icons(&self, named: Option<String>) -> Task<Message> {
        let theme = named.clone().or_else(|| self.sources.gtk_theme.clone());
        let dirs = self.sources.data_dirs.clone();
        let home = self.sources.home.clone();
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name(String::from("dock-icons"))
            .spawn(move || {
                let lookup = IconLookup::new(&dirs, home.as_deref(), theme.as_deref());
                let _ = sender.send(lookup);
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot start the thread that looks the icons up");
            return Task::none();
        }
        Task::future(async move {
            let lookup = receiver.await.ok()?;
            Some(Message::Icons(named, Box::new(lookup)))
        })
        .and_then(Task::done)
    }

    /// Rebuilds the icons and, when the surface changed, tells the compositor: its size, and
    /// its anchor, margin and exclusive zone when the edge changed.
    fn refresh(&mut self, previous: Geometry) -> Task<Message> {
        self.refresh_as(previous, self.geometry == previous)
    }

    /// `refresh`, the icons animated or put in place at once. The input region follows the
    /// icons at once, when one leaves as when one arrives.
    /// Uses the time of the event being handled (`clock`, read once when it came in), so that
    /// what an event starts all starts at one instant.
    fn refresh_as(&mut self, previous: Geometry, animate: bool) -> Task<Message> {
        let (size, count) = (self.surface_size, self.items.len());
        self.rebuild(animate);
        let resized = self.resize(size, previous);
        if self.items.len() == count {
            return resized;
        }
        Task::batch([resized, self.input_region()])
    }

    /// Tells the compositor what changed of the surface since it had `size` with `previous`.
    fn resize(&mut self, size: Size, previous: Geometry) -> Task<Message> {
        let Some(id) = self.surface else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        if self.geometry.position != previous.position || self.surface_size != size {
            let settings = self.layer_settings();
            self.sent.margin = Some(self.margin());
            self.sent.zone = Some(settings.exclusive_zone);
            tasks.push(Task::done(Message::LayoutChange {
                id,
                anchor: settings.anchor,
                size: settings.size,
            }));
            tasks.push(Task::done(Message::MarginChange {
                id,
                margin: settings.margin,
            }));
            tasks.push(Task::done(Message::ExclusiveZoneChange {
                id,
                zone_size: settings.exclusive_zone,
            }));
            tasks.push(self.drop_menu());
            self.hovering = false;
            tasks.push(self.input_region());
        }
        self.place_icons();
        Task::batch(tasks)
    }

    /// The pointer moved to `point`, or left the surface (`None`), at `now`: the icons head for
    /// the sizes of the board's curve, the label follows the closest icon. Returns whether the
    /// pointer entered or left the dock.
    pub fn hover(&mut self, point: Option<Point>, now: Duration) -> bool {
        self.now = self.now.max(now);
        let now = self.now;
        self.pointer = point;
        let layout = self.layout(now);
        // The pointer comes in over the bar or an icon; once in, it stays in over the whole
        // band of the magnified dock, gaps between grown icons included.
        let hovering = self.hovering;
        let inside =
            point.filter(|p| layout.contains(*p) || (hovering && layout.hover_band().contains(*p)));
        let count = self.items.len();
        let ratio = inside.map(|p| layout.ratio(p));
        for (size, target) in self
            .tracks
            .iter_mut()
            .map(|t| &mut t.size)
            .zip(self.geometry.targets(ratio, count))
        {
            if (size.target() - f64::from(target)).abs() > f64::EPSILON {
                size.retarget(now, f64::from(target));
            }
        }
        let fade = Duration::from_millis(dock::LABEL_FADE_MS);
        let opacity = if ratio.is_some() { 1.0 } else { 0.0 };
        if self.label.target() != opacity {
            self.label.retarget(now, opacity, fade);
        }
        if let Some(ratio) = ratio {
            self.label_item = Some(layout::nearest(ratio, count));
        }
        let entered = inside.is_some();
        let changed = entered != self.hovering;
        self.hovering = entered;
        changed
    }

    /// A frame was drawn at `now`.
    pub fn frame(&mut self, now: Duration) {
        // A frame stamped before the last pointer event must not take the time back.
        self.now = self.now.max(now);
        let now = self.now;
        self.frames += 1;
        let elapsed = now.saturating_sub(self.frames_since);
        if elapsed >= Duration::from_secs(1) || !self.animating() {
            let fps = f64::from(self.frames) / elapsed.as_secs_f64().max(1e-3);
            tracing::debug!(
                frames = self.frames,
                ms = elapsed.as_millis(),
                fps,
                "animation"
            );
            self.frames = 0;
            self.frames_since = now;
        }
    }

    /// A frame was drawn at `now`: the bounces past their timeout stop, the icons that are
    /// gone and the bounces that landed are dropped, and the surface shrinks once the icons
    /// that left are gone.
    pub fn tick(&mut self, now: Duration) -> Task<Message> {
        self.frame(now);
        let busy = self.busy();
        self.hide.tick(self.now, busy);
        let settled = self.settle();
        Task::batch([settled, self.apply_hide()])
    }

    /// A menu is open or an icon is held: a dock brought back stays.
    fn busy(&self) -> bool {
        self.menu_open() || self.press.is_some()
    }

    /// The time the dock draws at.
    pub fn now(&self) -> Duration {
        self.now
    }

    fn settle(&mut self) -> Task<Message> {
        let now = self.now;
        let dropped = if self.menu.as_ref().is_some_and(|m| m.unfold.closed(now)) {
            self.drop_menu()
        } else {
            Task::none()
        };
        for launch in &mut self.launches {
            launch.bounce.time_out(now);
        }
        self.launches.retain(|l| !l.bounce.is_done(now));
        self.calls.retain(|c| !c.1.is_done(now));
        self.poofs.retain(|p| !p.is_done(now));
        let count = self.leaving.len();
        self.leaving.retain(|l| !l.track.is_gone(now));
        if self.leaving.len() == count {
            return dropped;
        }
        let size = self.surface_size;
        self.surface_size = self.wanted_surface();
        Task::batch([dropped, self.resize(size, self.geometry)])
    }

    /// The source of windows has a new list: an application that starts and has one more
    /// window than when it started stops bouncing, and so does one whose window got the
    /// keyboard since (an application with a single instance raising the window it had).
    fn windows_came(&mut self) {
        let now = self.now;
        let Some(windows) = self.windows.as_ref() else {
            return;
        };
        // A window that starts asking for attention: its icon bounces a few hops (lot Q-52).
        let asking: Vec<&crate::windows::Window> =
            windows.list.iter().filter(|w| w.attention).collect();
        for window in &asking {
            if self.calling.contains(&window.id) {
                continue;
            }
            let item = self.items.iter().find(|i| i.owns(&window.app_id));
            if let Some(target) = item.map(|i| i.target())
                && !self.calls.iter().any(|c| c.0 == target)
            {
                tracing::info!(app = window.app_id, "a window asks for attention");
                self.calls
                    .push((target, Bounce::hops(now, dock::ATTENTION_HOPS)));
            }
        }
        self.calling = asking.iter().map(|w| w.id).collect();
        for launch in &mut self.launches {
            let Some(item) = self.items.iter().find(|i| i.target() == launch.target) else {
                continue;
            };
            let owned = windows.owned(|app_id| item.owns(app_id));
            let recent = owned.last().map_or(0, |w| w.recency);
            if launch.windows(owned.len(), recent, now) {
                tracing::info!(app = item.name, "window came: the bounce stops");
            }
        }
    }

    /// The pointer moved or the button came up while it is held on an icon.
    fn gesture(&mut self, gesture: Gesture) -> Task<Message> {
        let now = self.clock();
        let Some(press) = self.press.as_mut() else {
            return Task::none();
        };
        match gesture {
            Gesture::Moved(point) => {
                press.pointer = point;
                if !press.dragging && press.past_threshold() {
                    let target = press.target.clone();
                    // A pinned application, or an open one that a drop among the pinned ones
                    // pins (lot D-21d).
                    let draggable = self
                        .index_of(&target)
                        .and_then(|i| self.items.get(i))
                        .is_some_and(|i| i.entry().is_some() || (i.pinned && i.pin_id().is_some()));
                    if let Some(press) = self.press.as_mut() {
                        press.dragging = draggable;
                    }
                }
                self.drag(now)
            }
            Gesture::Released => self.release(now),
        }
    }

    /// The held icon follows the pointer: out of the dock its room closes, along the dock it
    /// takes the place nearest the pointer.
    fn drag(&mut self, now: Duration) -> Task<Message> {
        let Some(press) = self.press.clone().filter(|p| p.dragging) else {
            return Task::none();
        };
        let Some(index) = self.index_of(&press.target) else {
            self.press = None;
            return Task::none();
        };
        let layout = self.layout(now);
        let side = self.tracks[index].size.value(now) as f32;
        let out = layout.depth(press.pointer) > self.geometry.bar_thickness() + side / 2.0;
        if out != press.out {
            self.tracks[index].open(now, !out);
            if let Some(press) = self.press.as_mut() {
                press.out = out;
            }
        }
        if out {
            return Task::none();
        }
        if let Some(crossed) = self.cross(index, &layout, press.pointer) {
            return crossed;
        }
        // Among the orders the pinned icons can take, the one that puts the icon nearest the
        // pointer.
        let pinned = self.separator.unwrap_or(self.items.len());
        if index >= pinned {
            // An open application on its side of the separator: it only follows the pointer.
            return Task::none();
        }
        let sides: Vec<f32> = self
            .tracks
            .iter()
            .map(|t| t.size.value(now) as f32)
            .collect();
        let along = layout.along(press.pointer);
        let distance = |order: &Vec<usize>| {
            let mut ordered: Vec<f32> = order.iter().map(|&i| sides[i]).collect();
            ordered.extend_from_slice(&sides[pinned..]);
            let candidate = Layout::new(self.geometry, self.surface_size, &ordered, self.separator);
            order
                .iter()
                .position(|&i| i == index)
                .and_then(|place| candidate.middle(place))
                .map_or(f32::INFINITY, |middle| (middle - along).abs())
        };
        let fixed = self
            .items
            .iter()
            .take_while(|i| i.kind == Kind::Applications)
            .count();
        let Some(best) = drag::orders(fixed, pinned, index)
            .into_iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        else {
            return Task::none();
        };
        let ids: Vec<String> = best
            .iter()
            .filter_map(|&i| self.items[i].pin_id().map(String::from))
            .collect();
        let Some(id) = self.items[index].pin_id().map(String::from) else {
            return Task::none();
        };
        if ids == self.shown_pins() {
            // Same order on screen: nothing moves, even among pins that are not shown.
            return Task::none();
        }
        let before = ids
            .iter()
            .position(|i| *i == id)
            .and_then(|place| ids.get(place + 1))
            .cloned();
        self.list_sherlock();
        if dock_config::move_pinned(&mut self.pinned, &id, before.as_deref()) {
            return self.refresh(self.geometry);
        }
        Task::none()
    }

    /// The held icon of `index` crossed the separator (lot D-21d): an open application dragged
    /// among the pinned ones is pinned there, at the end of them, the next moves putting it in
    /// its place; a pinned application that is open, dragged after the separator, is unpinned.
    /// The others make room as the icons are built again. `None` when it did not cross.
    fn cross(&mut self, index: usize, layout: &Layout, pointer: Point) -> Option<Task<Message>> {
        let item = self.items.get(index)?;
        let id = item.entry()?.id.clone();
        let along = layout.along(pointer);
        let side = self.geometry.icon;
        // Where the two sides meet: the separator, or past the last other icon without one.
        let border = match layout.separator {
            Some(separator) => layout.along(separator.center()),
            None => {
                (0..self.items.len())
                    .filter(|&i| i != index)
                    .filter_map(|i| layout.middle(i))
                    .fold(f32::NEG_INFINITY, f32::max)
                    + side / 2.0
            }
        };
        let margin = side * dock::CROSS_MARGIN;
        let open = self.open_windows(index).is_some_and(|(n, _)| n > 0);
        let pins = !item.pinned && along < border - margin;
        let unpins = item.pinned && open && along > border + margin;
        if pins || unpins {
            // Sherlock keeps the place it is shown at (lot D-21h), whatever the list becomes.
            self.list_sherlock();
        }
        if pins {
            tracing::info!(app = id, "pinned by a drag");
            self.pinned.push(id);
        } else if unpins {
            tracing::info!(app = id, "unpinned by a drag");
            self.pinned.retain(|p| *p != id);
        } else {
            return None;
        }
        Some(self.refresh(self.geometry))
    }

    /// The button came up: a click when the icon was not dragged; otherwise the icon flies to
    /// its place and the new order is written, or, out of the dock, it is unpinned.
    fn release(&mut self, now: Duration) -> Task<Message> {
        let held = self.layout(now);
        let Some(press) = self.press.take() else {
            return Task::none();
        };
        self.hide.restart(now);
        let Some(index) = self.index_of(&press.target) else {
            return Task::none();
        };
        if !press.dragging {
            if held.icon_for(press.pointer) != Some(index) {
                return Task::none();
            }
            // Ctrl+click: a new window, as a click of the middle button.
            if self.modifiers.control() {
                self.new_window(index);
            } else {
                return self.press_icon(index);
            }
            return Task::none();
        }
        // Let go over the trash: thrown away, as out of the dock (lot D-21j).
        let over_trash = held
            .icon_for(press.pointer)
            .and_then(|i| self.items.get(i))
            .is_some_and(|i| matches!(i.kind, Kind::Trash(_)));
        let press = drag::Press {
            out: press.out || over_trash,
            ..press
        };
        let free = self.layout(now);
        let (Some(drawn), Some(place)) = (held.icons.get(index), free.icons.get(index)) else {
            return Task::none();
        };
        let (from, to) = (held.offset(*drawn), free.offset(*place));
        let by = (from.0 - to.0, from.1 - to.1);
        let open = self.open_windows(index).is_some_and(|(n, _)| n > 0);
        // A pinned application let go out of the dock goes up in smoke, as on macOS; an open
        // one comes back after the separator, out of the smoke.
        if press.out && self.items[index].pinned {
            let center = drawn.center();
            self.poofs.push(Poof {
                at: (center.x, center.y),
                side: self.geometry.icon,
                start: now,
            });
        }
        if press.out && !open {
            // It stays where it was let go, and vanishes there.
            let current = self.tracks[index].slot(now).shift;
            let at = (current.0 + by.0, current.1 + by.1);
            self.tracks[index].moved(now, at, at);
            self.dropping = Some(press.target);
            return self.unpin(index);
        }
        self.shift(index, by, now);
        self.tracks[index].open(now, true);
        if press.out {
            // Still open: it moves after the separator.
            return self.unpin(index);
        }
        if self.shown_pins() != press.shown {
            tracing::info!(pinned = ?self.pinned, "reordered");
            return self.save_pinned();
        }
        // Back where it was: the list too, and the file is not written.
        self.pinned = press.pinned;
        Task::none()
    }

    /// The pinned icons shown, in order: desktop file ids and Sherlock's token.
    fn shown_pins(&self) -> Vec<String> {
        let end = self.separator.unwrap_or(self.items.len());
        self.items[..end.min(self.items.len())]
            .iter()
            .filter_map(|i| i.pin_id().map(String::from))
            .collect()
    }

    /// Writes Sherlock's token in the pinned list where Sherlock is shown, when the list has
    /// none (a file written before it could move): the pinned list is about to change, and
    /// Sherlock must stay where it is rather than go back to the middle.
    fn list_sherlock(&mut self) {
        if !self.sherlock || self.pinned.iter().any(|p| p == dock_config::SHERLOCK) {
            return;
        }
        let shown = self.shown_pins();
        let shown: Vec<&str> = shown.iter().map(String::as_str).collect();
        items::place_sherlock(&mut self.pinned, &shown);
    }

    fn pointer(&mut self, event: Pointer, now: Duration) -> Task<Message> {
        match event {
            Pointer::Moved(_) | Pointer::Left => {
                if !self.animating() {
                    // A new animation: count its frames from here.
                    self.frames = 0;
                    self.frames_since = now;
                }
                let point = match event {
                    Pointer::Moved(point) => Some(point),
                    _ => None,
                };
                // On the dock, or on the band of a hidden one: it comes back, or stays.
                match point {
                    Some(_) => self.hide.pointer_in(now),
                    None => self.hide.pointer_out(now),
                }
                let hide = self.apply_hide();
                if self.hover(point, now) {
                    if !self.hovering {
                        // The icons are back at rest: so are their rectangles, if the drawn
                        // ones were sent for a minimize.
                        self.place_icons();
                    }
                    return Task::batch([hide, self.input_region()]);
                }
                return hide;
            }
            Pointer::Pressed(button) => {
                // Like `:focus-visible`: the mouse removes the keyboard ring.
                self.focus = None;
                let layout = self.layout(now);
                let target = self.pointer.and_then(|p| {
                    // A right click on the bar between two icons or on the separator opens the
                    // dock's own menu (lot D-21h); elsewhere a space goes to the nearest icon
                    // (lot D-21f).
                    let own = button == mouse::Button::Right
                        && layout.bar.contains(p)
                        && layout.icon_at(p).is_none();
                    if own { None } else { layout.icon_for(p) }
                });
                match (button, target, self.pointer) {
                    (mouse::Button::Left, Some(index), Some(point)) => {
                        let target = self.items[index].target();
                        let shown = self.shown_pins();
                        self.press = Some(Press::new(target, point, self.pinned.clone(), shown));
                    }
                    (mouse::Button::Right, Some(index), _) => return self.open_menu(index),
                    // On the separator or between the icons: the dock's own menu.
                    (mouse::Button::Right, None, Some(point))
                        if self.layout(now).bar.contains(point) =>
                    {
                        return self.open_dock_menu(point);
                    }
                    (mouse::Button::Middle, Some(index), _) => self.new_window(index),
                    _ => {}
                }
            }
            Pointer::Released => {
                let now = self.clock();
                return self.release(now);
            }
        }
        Task::none()
    }

    /// Tells the compositor where the dock takes the pointer (`Geometry::input_region`).
    fn input_region(&self) -> Task<Message> {
        let Some(id) = self.surface else {
            return Task::none();
        };
        let area = self.input_area();
        let callback = RegionActionCB::new(move |region| {
            let (x, y, width, height) = layout::pixels(area);
            let added = region.add(x, y, width, height);
            if let Err(error) = added {
                tracing::warn!(%error, "cannot set the input region");
            }
        });
        Task::done(Message::SetInputRegion { id, callback })
    }

    /// Where the surface takes the pointer: the band of a hidden dock, otherwise
    /// `Geometry::input_region` stretched to the screen edge.
    pub fn input_area(&self) -> iced::Rectangle {
        if self.hide.hidden() {
            return self.geometry.strip(self.surface_size);
        }
        let region = self.geometry.input_region(
            self.surface_size,
            self.items.len(),
            self.separator,
            self.hovering,
        );
        // Up to the screen edge, gap included (lot D-21f): a click pushed against the edge
        // reaches the icon of its column, and a dock brought back over a window keeps the
        // pointer there.
        self.geometry.to_edge(region, self.surface_size)
    }

    fn key(&mut self, key: Key) -> Task<Message> {
        if let Some(menu) = &mut self.menu {
            match key {
                Key::Close => return self.close_menu(),
                Key::Activate => {
                    let picked = menu.cursor.and_then(|c| menu.entries.get(c));
                    if let Some(line) = picked.filter(|l| l.enabled) {
                        let entry = line.entry;
                        return self.pick(entry);
                    }
                }
                Key::Menu => {}
                key => menu.cursor = menu::move_cursor(&menu.entries, menu.cursor, key),
            }
            return Task::none();
        }
        match (key, self.focus) {
            (Key::Activate, Some(index)) => return self.press_icon(index),
            (Key::Menu, Some(index)) => return self.open_menu(index),
            (Key::Close, _) => self.focus = None,
            (Key::Next, Some(_)) => self.move_focus(Direction::Next),
            (Key::Previous, Some(_)) => self.move_focus(Direction::Previous),
            _ => {}
        }
        Task::none()
    }

    fn move_focus(&mut self, direction: Direction) {
        let count = self.items.len();
        self.focus = match (direction, self.focus) {
            _ if count == 0 => None,
            (Direction::Next, Some(i)) if i + 1 < count => Some(i + 1),
            (Direction::Next, _) => Some(0),
            (Direction::Previous, Some(i)) if i > 0 && i <= count => Some(i - 1),
            (Direction::Previous, _) => Some(count - 1),
        };
    }

    /// The icon with the keyboard focus.
    pub fn focused(&self) -> Option<usize> {
        self.focus
    }

    /// Number of windows of icon `index` and whether one is active; `None` while the windows
    /// are unknown.
    pub fn open_windows(&self, index: usize) -> Option<(usize, bool)> {
        let windows = self.windows.as_ref()?;
        let item = self.items.get(index)?;
        Some(windows.of(|app_id| item.owns(app_id)))
    }

    pub fn abilities(&self, index: usize) -> Abilities {
        let item = self.items.get(index);
        let entry = item.and_then(Item::entry);
        let pinned = item.is_some_and(|i| i.pinned);
        Abilities {
            launch: entry.is_some_and(|e| e.exec.is_some()),
            keep: !pinned && entry.is_some(),
            unpin: pinned && item.and_then(Item::pin_id).is_some(),
            act: item.is_some_and(|i| i.entry().is_some() || matches!(i.kind, Kind::Window(_)))
                && self.controllable(),
            expose: self.windows.as_ref().is_some_and(|w| w.expose)
                && self.windows_of(index).iter().any(|w| !w.minimized),
            autostart: entry.map(|e| self.autostart.contains(&e.id)),
            evidence: entry.is_some(),
            force: self.windows.as_ref().is_some_and(|w| w.force),
        }
    }

    /// App Expose on the windows of icon `index` that are not minimized.
    pub fn expose(&mut self, index: usize) {
        let ids: Vec<u64> = self
            .windows_of(index)
            .iter()
            .filter(|w| !w.minimized)
            .map(|w| w.id)
            .collect();
        if !ids.is_empty() && self.windows.as_ref().is_some_and(|w| w.expose) {
            self.sources.control.show_windows(&ids);
        }
    }

    /// A scroll over the dock at `now`. Down over an icon with windows, past
    /// `dock::EXPOSE_SWIPE`, opens App Expose once; the swipe starts again after a pause of
    /// `dock::EXPOSE_SWIPE_GAP` or over another icon. Down is the fingers going down on a
    /// touchpad with natural scrolling (the default of the desktop settings), or a mouse wheel
    /// turned towards the user.
    fn scroll(&mut self, delta: mouse::ScrollDelta, now: Duration) {
        let down = match delta {
            mouse::ScrollDelta::Pixels { y, .. } => y,
            mouse::ScrollDelta::Lines { y, .. } => -y * dock::EXPOSE_SWIPE,
        };
        let Some(index) = self.pointer.and_then(|p| self.layout(now).icon_at(p)) else {
            self.swipe = None;
            return;
        };
        let target = self.items[index].target();
        let gone = match &self.swipe {
            Some((t, _, last)) => {
                *t != target || now.saturating_sub(*last) > dock::EXPOSE_SWIPE_GAP
            }
            None => true,
        };
        let far = if gone {
            0.0
        } else {
            self.swipe.as_ref().map_or(0.0, |s| s.1)
        };
        let far = (far + down).max(0.0);
        if far >= dock::EXPOSE_SWIPE {
            self.expose(index);
            self.swipe = Some((target, f32::NEG_INFINITY, now));
        } else {
            self.swipe = Some((target, far, now));
        }
    }

    /// Opacity of the dot under icon `index` now: 1 while its application has a window, and
    /// in between while it fades.
    pub fn dot(&self, index: usize) -> f32 {
        self.tracks
            .get(index)
            .map_or(0.0, |t| t.dot.value(self.now))
    }

    /// The windows of icon `index`, the least recent first; none while they are unknown.
    pub fn windows_of(&self, index: usize) -> Vec<Window> {
        let (Some(windows), Some(item)) = (self.windows.as_ref(), self.items.get(index)) else {
            return Vec::new();
        };
        windows
            .owned(|app_id| item.owns(app_id))
            .into_iter()
            .cloned()
            .collect()
    }

    /// The entries of the menu of icon `index`: its windows in the order they came, as on
    /// macOS, an untitled one under the application's name, then its actions.
    pub fn menu_lines(&self, index: usize) -> Vec<Line> {
        if let Some(Kind::Stack(_)) = self.items.get(index).map(|i| &i.kind) {
            return menu::stack_entries();
        }
        if let Some(Kind::Trash(full)) = self.items.get(index).map(|i| &i.kind) {
            return menu::trash_entries(*full, self.trash_armed.is_some());
        }
        let name = self.items.get(index).map_or("", |i| i.name.as_str());
        let mut windows = self.windows_of(index);
        windows.sort_by_key(|w| {
            let list = self.windows.as_ref().map_or(&[][..], |l| l.list.as_slice());
            list.iter().position(|o| o.id == w.id)
        });
        let listed: Vec<Listed> = windows
            .iter()
            .map(|w| Listed {
                id: w.id,
                title: if w.title.trim().is_empty() {
                    name.to_owned()
                } else {
                    w.title.clone()
                },
                active: w.activated,
            })
            .collect();
        let actions: Vec<(usize, String)> = self
            .items
            .get(index)
            .and_then(Item::entry)
            .map(|e| {
                e.actions
                    .iter()
                    .enumerate()
                    .filter(|(_, a)| e.other_actions().any(|o| o.id == a.id))
                    .map(|(i, a)| (i, a.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        menu::entries(self.abilities(index), &listed, &actions)
    }

    fn controllable(&self) -> bool {
        self.windows.as_ref().is_some_and(|w| w.controllable)
    }

    /// A click on icon `index`: brings its windows forward (`windows::click`), minimizes them
    /// when they already are, or starts the application.
    /// A click on icon `index`: the grid of a stack opens (lot D-21k), anything else `click`s.
    pub fn press_icon(&mut self, index: usize) -> Task<Message> {
        if self
            .items
            .get(index)
            .is_some_and(|i| matches!(i.kind, Kind::Stack(_)))
        {
            return self.open_grid(index);
        }
        self.click(index);
        Task::none()
    }

    pub fn click(&mut self, index: usize) {
        if self
            .items
            .get(index)
            .is_some_and(|i| matches!(i.kind, Kind::Trash(_)))
        {
            open_trash();
            return;
        }
        if self
            .items
            .get(index)
            .is_some_and(|i| i.kind == Kind::Applications)
        {
            // Started again while open, the view closes: the button toggles it.
            match launch::toggle_applications() {
                Ok(pid) => tracing::info!(pid, "applications view"),
                Err(error) => tracing::warn!(%error, "cannot open the applications view"),
            }
            return;
        }
        if self
            .items
            .get(index)
            .is_some_and(|i| i.kind == Kind::Sherlock)
        {
            // Sherlock has no window yet (A-19): the launcher, where it is asked (lot D-21m).
            match launch::open_launcher() {
                Ok(pid) => tracing::info!(pid, "launcher, for Sherlock"),
                Err(error) => tracing::warn!(%error, "cannot open the launcher"),
            }
            return;
        }
        let windows = self.windows_of(index);
        let owned: Vec<&Window> = windows.iter().collect();
        match windows::click(&owned, self.controllable()) {
            Click::Launch => self.activate(index, What::App),
            Click::Bring(ids) => {
                for id in ids {
                    if let Some(window) = windows.iter().find(|w| w.id == id) {
                        self.sources.control.send_to(Request::Activate, window);
                    }
                }
            }
            Click::Minimize => {
                self.place_drawn_icons();
                self.request(index, Request::Minimize);
            }
        }
    }

    /// A click of the middle button or a Ctrl+click on icon `index`: the application starts
    /// again for a new window, and bounces until it comes (`What::NewWindow`).
    pub fn new_window(&mut self, index: usize) {
        if self.items.get(index).and_then(Item::entry).is_some() {
            self.activate(index, What::NewWindow);
        }
    }

    /// Sends `request` to the windows of icon `index`.
    fn request(&self, index: usize, request: Request) {
        let Some(item) = self.items.get(index).cloned() else {
            return;
        };
        self.sources
            .control
            .send(request, Box::new(move |app_id| item.owns(app_id)));
    }

    /// Starts `what` for icon `index`, which bounces until a window of the application comes.
    /// While it bounces, it is not started again.
    pub fn activate(&mut self, index: usize, what: What) {
        let now = self.clock();
        let Some(item) = self.items.get(index) else {
            return;
        };
        let Some(entry) = item.entry() else {
            tracing::info!(name = item.name, "nothing to start for this icon yet");
            return;
        };
        let target = item.target();
        if self
            .launches
            .iter()
            .any(|l| l.target == target && !l.bounce.stopping())
        {
            tracing::info!(app = entry.id, "already starting");
            return;
        }
        // Started once the compositor gave the activation token of the click (lot Q-52): an
        // application already open that gets the launch brings its window forward with it.
        let app = entry.clone();
        let start =
            move |token: Option<String>| match launch::launch_with(&app, what, token.as_deref()) {
                Ok(pid) => tracing::info!(app = app.id, pid, ?what, "started"),
                Err(error) => tracing::warn!(app = app.id, %error, "cannot start"),
            };
        match self.surface {
            Some(_) => self.asks.push(launch::after_token(start)),
            None => start(None),
        }
        let before = self.windows.as_ref().map(|w| {
            let owned = w.owned(|a| item.owns(a));
            (owned.len(), owned.last().map_or(0, |w| w.recency))
        });
        self.launches.retain(|l| l.target != target);
        if !self.animating() {
            self.frames = 0;
            self.frames_since = now;
        }
        self.launches.push(Launch::new(target, now, before));
    }

    fn open_menu(&mut self, index: usize) -> Task<Message> {
        let entries = self.menu_lines(index);
        let Some(icon) = self.layout(self.now).icons.get(index).copied() else {
            return self.drop_menu();
        };
        let target = self.items.get(index).map(Item::target);
        self.show_menu(target, entries, icon, None)
    }

    /// Opens the grid of the stack of icon `index` over it, as a menu (lot D-21k).
    fn open_grid(&mut self, index: usize) -> Task<Message> {
        let count = self.stack.as_ref().map_or(0, |s| s.items.len());
        let Some(icon) = self.layout(self.now).icons.get(index).copied() else {
            return Task::none();
        };
        let target = self.items.get(index).map(Item::target);
        self.show_menu(target, Vec::new(), icon, Some(count))
    }

    /// Opens the downloads folder in Evidence.
    fn open_stack_folder(&self) {
        let Some(stack) = &self.stack else {
            return;
        };
        match launch::open_in_evidence(&stack.folder.to_string_lossy()) {
            Ok(pid) => tracing::info!(pid, "stack folder opened"),
            Err(error) => tracing::warn!(%error, "cannot open the stack folder"),
        }
    }

    /// The dock's own menu, from `point` on its bar, as if an icon of no width were there.
    fn open_dock_menu(&mut self, point: Point) -> Task<Message> {
        let bar = self.layout(self.now).bar;
        let at = if self.geometry.position.is_vertical() {
            iced::Rectangle::new(Point::new(bar.x, point.y), Size::new(bar.width, 0.0))
        } else {
            iced::Rectangle::new(Point::new(point.x, bar.y), Size::new(0.0, bar.height))
        };
        self.show_menu(None, menu::dock_entries(self.sherlock), at, None)
    }

    /// Opens the menu of `target` with `entries`, against `icon` on the side away from the
    /// screen edge.
    fn show_menu(
        &mut self,
        target: Option<Target>,
        entries: Vec<Line>,
        icon: iced::Rectangle,
        grid: Option<usize>,
    ) -> Task<Message> {
        let close = self.drop_menu();
        let Some(surface) = self.surface else {
            tracing::warn!("dock surface unknown: the menu cannot open");
            return close;
        };
        let (width, height) = match grid {
            Some(count) => {
                let (width, height) = dock::stack_size(count);
                (width.ceil() as u32, height.ceil() as u32)
            }
            None => menu::surface_size(&entries),
        };
        let layout = self.layout(self.now);
        // Off the bar, 12 away from it, centered on the icon. The gap is not part of the menu
        // surface: it would be drawn black over the magnified icons (opaque software
        // rendering).
        let bar = layout.bar;
        let gap = dock::MENU_GAP;
        // Past the icon too when it grew beyond the bar.
        let (point, toward) = match self.geometry.position {
            Edge::Bottom => (
                Point::new(icon.center_x(), bar.y.min(icon.y) - gap),
                PopupAnchor::Top,
            ),
            Edge::Top => (
                Point::new(
                    icon.center_x(),
                    (bar.y + bar.height).max(icon.y + icon.height) + gap,
                ),
                PopupAnchor::Bottom,
            ),
            Edge::Left => (
                Point::new(
                    (bar.x + bar.width).max(icon.x + icon.width) + gap,
                    icon.center_y(),
                ),
                PopupAnchor::Right,
            ),
            Edge::Right => (
                Point::new(bar.x.min(icon.x) - gap, icon.center_y()),
                PopupAnchor::Left,
            ),
        };
        let gravity = match toward {
            PopupAnchor::Top => PopupGravity::Top,
            PopupAnchor::Bottom => PopupGravity::Bottom,
            PopupAnchor::Right => PopupGravity::Right,
            _ => PopupGravity::Left,
        };
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(width, height),
            Position::new(point.x as i32, point.y as i32),
            PixelSize::px(1, 1),
        )
        .anchor(toward)
        .gravity(gravity);
        let id = Id::unique();
        self.menu = Some(OpenMenu {
            id,
            target,
            entries,
            cursor: None,
            grid,
            unfold: Unfold::opening(self.now),
        });
        Task::batch([close, Task::done(Message::NewPopUp { settings, id })])
    }

    /// The place of the icon of `target`, if it is still in the dock.
    pub(crate) fn index_of(&self, target: &Target) -> Option<usize> {
        self.items.iter().position(|item| item.target() == *target)
    }

    /// Closes the menu: it folds back first, and its popup goes once it has faded out
    /// (`settle`).
    fn close_menu(&mut self) -> Task<Message> {
        let now = self.clock();
        if let Some(menu) = &mut self.menu {
            menu.unfold.close(now);
            menu.cursor = None;
            if !self.animating() {
                self.frames = 0;
                self.frames_since = now;
            }
        }
        Task::none()
    }

    /// Closes the popup of the menu at once.
    fn drop_menu(&mut self) -> Task<Message> {
        match self.menu.take() {
            Some(menu) => {
                let now = self.clock();
                self.hide.restart(now);
                window::close(menu.id)
            }
            None => Task::none(),
        }
    }

    fn pick(&mut self, entry: Entry) -> Task<Message> {
        self.clock();
        let Some(target) = self
            .menu
            .as_ref()
            .filter(|m| !m.unfold.closing())
            .map(|m| m.target.clone())
        else {
            return Task::none();
        };
        if entry == Entry::EmptyTrash {
            return self.empty_trash();
        }
        if target == Some(crate::items::Target::Trash) {
            let close = self.close_menu();
            if entry == Entry::Open {
                open_trash();
            }
            return close;
        }
        if target == Some(crate::items::Target::Stack) {
            if entry == Entry::Open {
                self.open_stack_folder();
            }
            return self.close_menu();
        }
        let close = self.close_menu();
        if entry == Entry::ShowSherlock {
            let task = if self.sherlock {
                self.hide_sherlock()
            } else {
                self.show_sherlock()
            };
            return Task::batch([close, task]);
        }
        let Some(target) = target else {
            return close;
        };
        // The icons may have moved since the menu opened: find the application again.
        let Some(index) = self.index_of(&target) else {
            tracing::info!(
                ?target,
                "the application left the dock while its menu was open"
            );
            return close;
        };
        let task = match entry {
            Entry::Window(id) => {
                let window = self.windows_of(index).into_iter().find(|w| w.id == id);
                if let Some(window) = window {
                    self.sources.control.send_to(Request::Activate, &window);
                }
                Task::none()
            }
            Entry::Open => {
                self.activate(index, What::App);
                Task::none()
            }
            Entry::NewWindow => {
                self.activate(index, What::NewWindow);
                Task::none()
            }
            Entry::Action(action) => {
                self.activate(index, What::Action(action));
                Task::none()
            }
            Entry::ShowAll => {
                self.expose(index);
                Task::none()
            }
            Entry::Keep => self.keep(index),
            Entry::Unpin => self.unpin(index),
            Entry::Quit => {
                self.request(index, Request::Close);
                Task::none()
            }
            Entry::Hide => {
                self.request(index, Request::Minimize);
                Task::none()
            }
            Entry::ForceQuit => {
                self.request(index, Request::Kill);
                Task::none()
            }
            Entry::OpenAtLogin => self.toggle_autostart(index),
            Entry::ShowInEvidence => {
                let folder = self
                    .items
                    .get(index)
                    .and_then(Item::entry)
                    .and_then(|e| e.file.parent().map(PathBuf::from));
                if let Some(folder) = folder
                    && let Err(error) = crate::launch::show_in_evidence(&folder)
                {
                    tracing::warn!(%error, "cannot start Evidence");
                }
                Task::none()
            }
            Entry::Options | Entry::ShowSherlock | Entry::EmptyTrash => Task::none(),
        };
        Task::batch([close, task])
    }

    /// « Vider la corbeille »: the first click arms it, the menu staying open; a second one,
    /// past the launcher's `CONFIRM_DELAY` (a double click only arms), empties the trash on a
    /// thread of its own and closes the menu.
    fn empty_trash(&mut self) -> Task<Message> {
        let now = Instant::now();
        match self.trash_armed {
            Some(at) if now.duration_since(at) >= sherlock_ui::launcher::CONFIRM_DELAY => {}
            Some(_) => return Task::none(),
            None => {
                self.trash_armed = Some(now);
                // The menu says what the next click does.
                let trash = self
                    .items
                    .iter()
                    .position(|i| matches!(i.kind, Kind::Trash(_)));
                if let Some(index) = trash {
                    let lines = self.menu_lines(index);
                    if let Some(menu) = &mut self.menu {
                        menu.entries = lines;
                    }
                }
                return Task::none();
            }
        }
        self.trash_armed = None;
        let close = self.close_menu();
        let Some(folder) = crate::trash::folder() else {
            return close;
        };
        let emptied = Task::future(async move {
            let (sender, receiver) = iced::futures::channel::oneshot::channel();
            let spawned = std::thread::Builder::new()
                .name(String::from("empty-trash"))
                .spawn(move || {
                    let _ = sender.send(crate::trash::empty(&folder));
                });
            if spawned.is_err() {
                return None;
            }
            match receiver.await.ok()? {
                Ok(count) => tracing::info!(count, "trash emptied"),
                Err(error) => tracing::warn!(%error, "cannot empty the trash"),
            }
            None
        })
        .and_then(Task::done);
        Task::batch([close, emptied])
    }

    /// Reads which applications open with the session, on a thread of its own.
    pub fn read_autostart(&self) -> Task<Message> {
        let Some(folder) = crate::autostart::folder() else {
            return Task::none();
        };
        off_thread(move || crate::autostart::read(&folder), Message::Autostart)
    }

    /// « Ouvrir à l'ouverture de session » of icon `index`: its desktop file goes in the
    /// autostart folder, or out of it, on a thread of its own; the list is read again then.
    fn toggle_autostart(&mut self, index: usize) -> Task<Message> {
        let (Some(folder), Some(entry)) = (
            crate::autostart::folder(),
            self.items.get(index).and_then(Item::entry).cloned(),
        ) else {
            return Task::none();
        };
        let on = !self.autostart.contains(&entry.id);
        // Shown at once; the folder read again corrects it if the write failed.
        if on {
            self.autostart.insert(entry.id.clone());
        } else {
            self.autostart.remove(&entry.id);
        }
        off_thread(
            move || {
                if let Err(error) = crate::autostart::set(&folder, &entry.id, &entry.file, on) {
                    tracing::warn!(%error, id = entry.id, "cannot change the autostart folder");
                }
                crate::autostart::read(&folder)
            },
            Message::Autostart,
        )
    }

    /// Adds the open application of icon `index` to the pinned list, at its end.
    pub fn keep(&mut self, index: usize) -> Task<Message> {
        let Some(id) = self
            .items
            .get(index)
            .filter(|i| !i.pinned)
            .and_then(Item::entry)
            .map(|e| e.id.clone())
        else {
            return Task::none();
        };
        tracing::info!(app = id, "pinned");
        self.list_sherlock();
        self.pinned.push(id);
        self.save_pinned()
    }

    /// Removes icon `index` from the pinned list: Sherlock leaves the dock, the others keep
    /// their places, an application still open moves after the separator.
    pub fn unpin(&mut self, index: usize) -> Task<Message> {
        let Some(id) = self
            .items
            .get(index)
            .filter(|i| i.pinned)
            .and_then(Item::pin_id)
            .map(String::from)
        else {
            return Task::none();
        };
        if id == dock_config::SHERLOCK {
            return self.hide_sherlock();
        }
        tracing::info!(app = id, "unpinned");
        self.list_sherlock();
        self.pinned.retain(|p| *p != id);
        self.save_pinned()
    }

    /// Takes Sherlock out of the dock.
    pub fn hide_sherlock(&mut self) -> Task<Message> {
        tracing::info!("Sherlock taken out of the dock");
        self.sherlock = false;
        self.pinned.retain(|p| p != dock_config::SHERLOCK);
        self.save_pinned()
    }

    /// Puts Sherlock back in the middle of the pinned applications, where it was at first.
    pub fn show_sherlock(&mut self) -> Task<Message> {
        tracing::info!("Sherlock back in the dock");
        self.sherlock = true;
        self.pinned.retain(|p| p != dock_config::SHERLOCK);
        let shown = self.shown_pins();
        let mut shown: Vec<&str> = shown.iter().map(String::as_str).collect();
        shown.insert(shown.len().div_ceil(2), dock_config::SHERLOCK);
        items::place_sherlock(&mut self.pinned, &shown);
        self.save_pinned()
    }

    /// An application dragged from the Applications view (lot D-41d): over the dock, a hole
    /// opens among the pinned icons where it would go; dropped, it is pinned there; gone, the
    /// hole closes. The icons make room and close it on their own tracks.
    pub fn incoming(&mut self, event: crate::dnd::Event) -> Task<Message> {
        use crate::dnd::Event;
        match event {
            Event::Over(x, y) => self.hole_at(Point::new(x, y)),
            Event::Left => self.close_hole(),
            Event::Dropped(id) => self.drop_app(&id),
            Event::DragEnded(_) => Task::none(),
        }
    }

    /// Opens the hole, or moves it, among the pinned icons nearest `point`; out of the dock's
    /// band, it closes.
    fn hole_at(&mut self, point: Point) -> Task<Message> {
        let now = self.clock();
        let layout = self.layout(now);
        if layout.depth(point) > self.geometry.bar_thickness() + self.geometry.icon / 2.0 {
            return self.close_hole();
        }
        self.list_sherlock();
        let along = layout.along(point);
        let end = self.separator.unwrap_or(self.items.len());
        let fixed = self
            .items
            .iter()
            .take_while(|i| i.kind == Kind::Applications)
            .count();
        let before: Option<String> = (fixed..end.min(self.items.len()))
            .filter(|&i| self.items[i].kind != Kind::Hole)
            .find(|&i| layout.middle(i).is_some_and(|middle| middle > along))
            .and_then(|i| self.items[i].pin_id().map(String::from));
        let changed = if self.pinned.iter().any(|p| p == items::HOLE) {
            dock_config::move_pinned(&mut self.pinned, items::HOLE, before.as_deref())
        } else {
            let at = before
                .and_then(|b| self.pinned.iter().position(|p| *p == b))
                .unwrap_or(self.pinned.len());
            self.pinned.insert(at, String::from(items::HOLE));
            true
        };
        if changed {
            return self.refresh(self.geometry);
        }
        Task::none()
    }

    /// The application went elsewhere: the hole closes.
    fn close_hole(&mut self) -> Task<Message> {
        let before = self.pinned.len();
        self.pinned.retain(|p| p != items::HOLE);
        if self.pinned.len() != before {
            return self.refresh(self.geometry);
        }
        Task::none()
    }

    /// The application `id` dropped: pinned in the hole, moved there if it already was, and
    /// the list written. An application the dock does not know closes the hole.
    fn drop_app(&mut self, id: &str) -> Task<Message> {
        if self.sources.catalog.get(id).is_none() {
            tracing::warn!(id, "dropped application not installed");
            return self.close_hole();
        }
        let Some(at) = self.pinned.iter().position(|p| p == items::HOLE) else {
            return Task::none();
        };
        // Where the hole is: before the next pin that is not this application.
        let before = self.pinned[at + 1..].iter().find(|p| *p != id).cloned();
        self.pinned.retain(|p| p != items::HOLE && p != id);
        let to = before
            .and_then(|b| self.pinned.iter().position(|p| *p == b))
            .unwrap_or(self.pinned.len());
        self.pinned.insert(to, id.to_owned());
        tracing::info!(id, "pinned from the Applications view");
        self.save_pinned()
    }

    /// Writes the pinned list to the configuration and rebuilds the dock. The writing is done
    /// off the interface thread (`save::Saver`); the file is read again first and only its list
    /// and Sherlock's key change (`pins::write`): the dock never writes what it has in memory,
    /// so a file it could not read at startup is not replaced by one with the pinned list alone.
    fn save_pinned(&mut self) -> Task<Message> {
        // The hole of a drag in progress is never written.
        self.pinned.retain(|p| p != items::HOLE);
        self.sources.config.pinned = Some(self.pinned.clone());
        self.sources.config.sherlock = self.sherlock;
        if let Some(path) = &self.sources.config_path {
            self.saver
                .save(path.clone(), self.pinned.clone(), self.sherlock);
        }
        self.refresh(self.geometry)
    }
}

/// Tab moves the focus; the other keys go to the menu when it is open, to the dock otherwise.
fn on_key(event: keyboard::Event) -> Option<Message> {
    if let keyboard::Event::ModifiersChanged(modifiers) = event {
        return Some(Message::Modifiers(modifiers));
    }
    focus::tab(event.clone())
        .map(Message::Focus)
        .or_else(|| menu::key(event).map(Message::Key))
}

/// The themes of the appearance file at `path`, live (`appearance_config::follow_themes`).
pub fn appearance(path: &Path) -> UnboundedReceiver<Theme> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let follow = appearance_config::follow_themes(path.to_path_buf(), move |theme| {
        sender.unbounded_send(theme).is_ok()
    });
    if let Err(error) = follow {
        tracing::warn!(%error, "the dock will not follow the appearance file");
    }
    receiver
}

/// The screen of a surface, from iced_exwlshell's reports.
fn on_screen(event: ShellEvent) -> Option<Message> {
    match event {
        ShellEvent::WindowOutputChanged { window, output } => Some(Message::Screen(
            window,
            output.map(|o| Screen {
                name: o.info.name.clone(),
                size: o.info.logical_size,
            }),
        )),
        _ => None,
    }
}

/// Opens the trash in Evidence, logging when it cannot.
fn open_trash() {
    match launch::open_trash() {
        Ok(pid) => tracing::info!(pid, "trash opened"),
        Err(error) => tracing::warn!(%error, "cannot open the trash"),
    }
}

/// `$XDG_CONFIG_HOME`, else `~/.config`.
fn config_home() -> Option<std::path::PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
}

/// Runs blocking `work` (a folder, a file) on a thread of its own, never on the interface's.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) -> Message + Send + 'static,
) -> Task<Message> {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name(String::from("dock-files"))
        .spawn(move || {
            let _ = sender.send(work());
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start a thread for the dock's files");
        return Task::none();
    }
    Task::future(async move { receiver.await.ok().map(done) }).and_then(Task::done)
}
