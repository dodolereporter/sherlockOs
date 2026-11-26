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
use iced_exwlshell::to_layer_message;
use sherlock_ui::color::{Theme, ThemeFade};
use sherlock_ui::dock;
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::motion::Fade;

use crate::anim::{Launch, Leaving, Track};
use crate::drag::{self, Gesture, Press};
use crate::entry::Catalog;
use crate::icons::IconLookup;
use crate::items::{Builder, Glyph, Item, Kind, Target};
use crate::launch::{self, What};
use crate::layout::{self, Flow, Geometry, Layout};
use crate::menu::{self, Abilities, Entry, Key};
use crate::windows::{self, Click, Control, Request, Windows};

/// The menu of an icon, while it is open.
#[derive(Debug, Clone)]
pub(crate) struct OpenMenu {
    pub(crate) id: Id,
    /// The icon the menu belongs to, by identity.
    pub(crate) target: Target,
    pub(crate) entries: Vec<(Entry, bool)>,
    pub(crate) cursor: Option<usize>,
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
    Surface(Id),
    Pointer(Pointer),
    Frame(Instant),
    Windows(Windows),
    /// The windows of the compositor's shell socket, for the bounce of a starting application.
    Shell(Windows),
    /// The pointer while the button is held on an icon.
    Gesture(Gesture),
    /// The configuration file changed.
    Config(Box<DockConfig>),
    Key(Key),
    Focus(Direction),
    MenuHover(usize),
    MenuPick(Entry),
    Closed(Id),
    /// The theme of the appearance file, at start then at each change.
    Theme(Theme),
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
}

pub struct Dock {
    pub items: Vec<Item>,
    /// Index of the first open application that is not pinned: the separator goes before it.
    pub separator: Option<usize>,
    /// Desktop file ids of the pinned applications, from the configuration or the defaults.
    pub pinned: Vec<String>,
    pub(crate) sources: Sources,
    glyphs: HashMap<String, Glyph>,
    pub geometry: Geometry,
    /// Size of the dock surface.
    pub surface_size: Size,
    surface: Option<Id>,
    /// The animations of each icon: size under the pointer, arrival, offset from its place.
    pub(crate) tracks: Vec<Track>,
    /// Icons that left the dock, drawn until they are gone.
    pub leaving: Vec<Leaving>,
    /// Applications started from the dock, bouncing until their first window.
    pub launches: Vec<Launch>,
    /// The icon let go out of the dock, which vanishes where it is.
    dropping: Option<Target>,
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
    /// The windows of the shell socket, read for the bounce only (batch D-18c uses them for the
    /// rest).
    shell: Option<Windows>,
    /// Icon with the keyboard focus, kept in our state like the bar does.
    pub(crate) focus: Option<usize>,
    pub(crate) menu: Option<OpenMenu>,
    /// Frames drawn since `frames_since`, for the frame rate in the debug log.
    frames: u32,
    frames_since: Duration,
    /// The theme of the appearance file, shown with a fade.
    pub theme: ThemeFade,
}

impl Dock {
    pub fn new(sources: Sources) -> Self {
        let geometry = Geometry::from_config(&sources.config);
        let fade = Duration::from_millis(dock::LABEL_FADE_MS);
        let mut dock = Self {
            items: Vec::new(),
            separator: None,
            pinned: sources.pinned.clone(),
            sources,
            glyphs: HashMap::new(),
            geometry,
            surface_size: Size::ZERO,
            surface: None,
            tracks: Vec::new(),
            leaving: Vec::new(),
            launches: Vec::new(),
            dropping: None,
            press: None,
            label: Fade::new(0.0, 0.0, Duration::ZERO, fade),
            label_item: None,
            hovering: false,
            pointer: None,
            origin: Instant::now(),
            now: Duration::ZERO,
            windows: None,
            shell: None,
            focus: None,
            menu: None,
            frames: 0,
            frames_since: Duration::ZERO,
            theme: ThemeFade::default(),
        };
        dock.rebuild(false);
        dock
    }

    /// Builds the icons again from the pinned list and the open windows. Each icon keeps its
    /// animations by identity. With `animate`, a new icon arrives, a gone one leaves, and the
    /// icons that moved start where they were drawn; otherwise everything is in place.
    fn rebuild(&mut self, animate: bool) {
        let now = self.now;
        let before = animate.then(|| self.layout(now));
        let icon_size = (self.geometry.icon * self.geometry.factor).round() as u32;
        let mut builder = Builder {
            catalog: &self.sources.catalog,
            lookup: &self.sources.lookup,
            icon_size,
            glyphs: &mut self.glyphs,
        };
        let (items, separator) = builder.build(&self.pinned, self.windows.as_ref());
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
            || !self.label.is_done(self.now)
            || !self.leaving.is_empty()
            || !self.launches.is_empty()
            || self.tracks.iter().any(|t| !t.is_done(self.now))
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
        let gesture = if self.press.is_some() {
            iced::event::listen_with(drag::gesture).map(Message::Gesture)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            appearance,
            Subscription::run(|| windows::watch(windows::source)).map(Message::Windows),
            Subscription::run(|| windows::watch(crate::shell::source)).map(Message::Shell),
            gesture,
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            config,
            frames,
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
        self.launches
            .iter()
            .find(|l| l.target == target)
            .map_or(0.0, |l| l.bounce.lift(now, track.size.value(now) as f32))
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surface = Some(id);
                return self.input_region();
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
            Message::Windows(windows) => {
                self.clock();
                self.windows = Some(windows);
                self.windows_came(0);
                return self.refresh(self.geometry);
            }
            Message::Shell(windows) => {
                self.clock();
                self.shell = Some(windows);
                self.windows_came(1);
            }
            Message::Gesture(gesture) => return self.gesture(gesture),
            Message::Config(config) => return self.configure(*config),
            Message::Key(key) => return self.key(key),
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
                    && menu.entries.get(index).is_some_and(|(_, on)| *on)
                {
                    menu.cursor = Some(index);
                }
            }
            Message::MenuPick(entry) => return self.pick(entry),
            Message::Closed(id) if self.menu.as_ref().is_some_and(|m| m.id == id) => {
                self.menu = None;
            }
            _ => {}
        }
        Task::none()
    }

    /// Applies a configuration read from the file: settings, icon theme, pinned list.
    pub fn configure(&mut self, config: DockConfig) -> Task<Message> {
        let previous = self.geometry;
        if config.icon_theme != self.sources.config.icon_theme {
            let theme = config
                .icon_theme
                .clone()
                .or_else(|| self.sources.gtk_theme.clone());
            self.sources.lookup = IconLookup::new(
                &self.sources.data_dirs,
                self.sources.home.as_deref(),
                theme.as_deref(),
            );
            self.glyphs.clear();
        }
        if let Some(pinned) = &config.pinned {
            self.pinned.clone_from(pinned);
        }
        self.geometry = Geometry::from_config(&config);
        self.sources.config = config;
        self.clock();
        self.refresh(previous)
    }

    /// Rebuilds the icons and, when the surface changed, tells the compositor: its size, and
    /// its anchor, margin and exclusive zone when the edge changed.
    fn refresh(&mut self, previous: Geometry) -> Task<Message> {
        self.clock();
        let size = self.surface_size;
        self.rebuild(self.geometry == previous);
        self.resize(size, previous)
    }

    /// Tells the compositor what changed of the surface since it had `size` with `previous`.
    fn resize(&mut self, size: Size, previous: Geometry) -> Task<Message> {
        let Some(id) = self.surface else {
            return Task::none();
        };
        let mut tasks = Vec::new();
        if self.geometry.position != previous.position || self.surface_size != size {
            let settings = crate::surface(self.geometry, self.surface_size);
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
            tasks.push(self.close_menu());
            self.hovering = false;
            tasks.push(self.input_region());
        }
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
        self.settle()
    }

    /// The time the dock draws at.
    pub fn now(&self) -> Duration {
        self.now
    }

    fn settle(&mut self) -> Task<Message> {
        let now = self.now;
        for launch in &mut self.launches {
            launch.bounce.time_out(now);
        }
        self.launches.retain(|l| !l.bounce.is_done(now));
        let count = self.leaving.len();
        self.leaving.retain(|l| !l.track.is_gone(now));
        if self.leaving.len() == count {
            return Task::none();
        }
        let size = self.surface_size;
        self.surface_size = self.wanted_surface();
        self.resize(size, self.geometry)
    }

    /// A source of windows (0 the dock's, 1 the shell socket) has a new list: an application
    /// that starts and has one more window than when it started stops bouncing.
    fn windows_came(&mut self, source: usize) {
        let now = self.now;
        let windows = if source == 0 {
            self.windows.as_ref()
        } else {
            self.shell.as_ref()
        };
        let Some(windows) = windows else {
            return;
        };
        for launch in &mut self.launches {
            let Some(item) = self.items.iter().find(|i| i.target() == launch.target) else {
                continue;
            };
            let (count, _) = windows.of(|app_id| item.owns(app_id));
            if launch.windows(source, count, now) {
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
                    let draggable = self
                        .index_of(&target)
                        .and_then(|i| self.items.get(i))
                        .is_some_and(|i| i.pinned && i.entry().is_some());
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
        // Among the orders the pinned icons can take, the one that puts the icon nearest the
        // pointer.
        let pinned = self.separator.unwrap_or(self.items.len());
        let Some(sherlock) = self.items.iter().position(|i| i.kind == Kind::Sherlock) else {
            return Task::none();
        };
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
        let Some(best) = drag::orders(pinned, sherlock, index)
            .into_iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        else {
            return Task::none();
        };
        let ids: Vec<&str> = best
            .iter()
            .filter_map(|&i| self.items[i].entry().map(|e| e.id.as_str()))
            .collect();
        let Some(id) = self.items[index].entry().map(|e| e.id.clone()) else {
            return Task::none();
        };
        let before = ids
            .iter()
            .position(|i| *i == id)
            .and_then(|place| ids.get(place + 1))
            .map(|next| (*next).to_owned());
        if dock_config::move_pinned(&mut self.pinned, &id, before.as_deref()) {
            return self.refresh(self.geometry);
        }
        Task::none()
    }

    /// The button came up: a click when the icon was not dragged; otherwise the icon flies to
    /// its place and the new order is written, or, out of the dock, it is unpinned.
    fn release(&mut self, now: Duration) -> Task<Message> {
        let held = self.layout(now);
        let Some(press) = self.press.take() else {
            return Task::none();
        };
        let Some(index) = self.index_of(&press.target) else {
            return Task::none();
        };
        if !press.dragging {
            if held.icon_at(press.pointer) == Some(index) {
                self.click(index);
            }
            return Task::none();
        }
        let free = self.layout(now);
        let (Some(drawn), Some(place)) = (held.icons.get(index), free.icons.get(index)) else {
            return Task::none();
        };
        let (from, to) = (held.offset(*drawn), free.offset(*place));
        let by = (from.0 - to.0, from.1 - to.1);
        let open = self.open_windows(index).is_some_and(|(n, _)| n > 0);
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
        if self.pinned != press.pinned {
            tracing::info!(pinned = ?self.pinned, "reordered");
            return self.save_pinned();
        }
        Task::none()
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
                if self.hover(point, now) {
                    return self.input_region();
                }
            }
            Pointer::Pressed(button) => {
                // Like `:focus-visible`: the mouse removes the keyboard ring.
                self.focus = None;
                let target = self.pointer.and_then(|p| self.layout(now).icon_at(p));
                match (button, target, self.pointer) {
                    (mouse::Button::Left, Some(index), Some(point)) => {
                        let target = self.items[index].target();
                        self.press = Some(Press::new(target, point, self.pinned.clone()));
                    }
                    (mouse::Button::Right, Some(index), _) => return self.open_menu(index),
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
        let area = self.geometry.input_region(
            self.surface_size,
            self.items.len(),
            self.separator,
            self.hovering,
        );
        let callback = RegionActionCB::new(move |region| {
            let (x, y, width, height) = layout::pixels(area);
            let added = region.add(x, y, width, height);
            if let Err(error) = added {
                tracing::warn!(%error, "cannot set the input region");
            }
        });
        Task::done(Message::SetInputRegion { id, callback })
    }

    fn key(&mut self, key: Key) -> Task<Message> {
        if let Some(menu) = &mut self.menu {
            match key {
                Key::Close => return self.close_menu(),
                Key::Activate => {
                    let picked = menu.cursor.and_then(|c| menu.entries.get(c)).copied();
                    if let Some((entry, true)) = picked {
                        return self.pick(entry);
                    }
                }
                Key::Menu => {}
                key => menu.cursor = menu::move_cursor(&menu.entries, menu.cursor, key),
            }
            return Task::none();
        }
        match (key, self.focus) {
            (Key::Activate, Some(index)) => self.click(index),
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
        let controllable = self.controllable();
        let open = self.open_windows(index).is_some_and(|(n, _)| n > 0);
        let sherlock = item.is_some_and(|i| i.kind == crate::items::Kind::Sherlock);
        Abilities {
            launch: entry.is_some_and(|e| e.exec.is_some()),
            new_window: entry.is_some_and(|e| e.new_window.is_some()),
            keep: !pinned && entry.is_some(),
            unpin: pinned && entry.is_some(),
            quit: !sherlock && controllable && open,
        }
    }

    fn controllable(&self) -> bool {
        self.windows.as_ref().is_some_and(|w| w.controllable)
    }

    /// A click on icon `index`: brings its windows forward, minimizes them when they already
    /// are, or starts the application.
    pub fn click(&mut self, index: usize) {
        match windows::click(self.open_windows(index), self.controllable()) {
            Click::Launch => self.activate(index, What::App),
            Click::Activate => self.request(index, Request::Activate),
            Click::Minimize => self.request(index, Request::Minimize),
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
        match launch::launch(entry, what) {
            Ok(pid) => tracing::info!(app = entry.id, pid, ?what, "started"),
            Err(error) => {
                tracing::warn!(app = entry.id, %error, "cannot start");
                return;
            }
        }
        let count = |windows: Option<&Windows>| windows.map(|w| w.of(|a| item.owns(a)).0);
        let before = [count(self.windows.as_ref()), count(self.shell.as_ref())];
        self.launches.retain(|l| l.target != target);
        if !self.animating() {
            self.frames = 0;
            self.frames_since = now;
        }
        self.launches.push(Launch::new(target, now, before));
    }

    fn open_menu(&mut self, index: usize) -> Task<Message> {
        let close = self.close_menu();
        let Some(surface) = self.surface else {
            tracing::warn!("dock surface unknown: the menu cannot open");
            return close;
        };
        let entries = menu::entries(self.abilities(index));
        let (width, height) = menu::surface_size(&entries);
        let layout = self.layout(self.now);
        let Some(icon) = layout.icons.get(index) else {
            return close;
        };
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
            target: self.items[index].target(),
            entries,
            cursor: None,
        });
        Task::batch([close, Task::done(Message::NewPopUp { settings, id })])
    }

    /// The place of the icon of `target`, if it is still in the dock.
    pub(crate) fn index_of(&self, target: &Target) -> Option<usize> {
        self.items.iter().position(|item| item.target() == *target)
    }

    fn close_menu(&mut self) -> Task<Message> {
        match self.menu.take() {
            Some(menu) => window::close(menu.id),
            None => Task::none(),
        }
    }

    fn pick(&mut self, entry: Entry) -> Task<Message> {
        let Some(target) = self.menu.as_ref().map(|m| m.target.clone()) else {
            return Task::none();
        };
        let close = self.close_menu();
        // The icons may have moved since the menu opened: find the application again.
        let Some(index) = self.index_of(&target) else {
            tracing::info!(
                ?target,
                "the application left the dock while its menu was open"
            );
            return close;
        };
        let task = match entry {
            Entry::Open => {
                self.activate(index, What::App);
                Task::none()
            }
            Entry::NewWindow => {
                self.activate(index, What::NewWindow);
                Task::none()
            }
            Entry::Keep => self.keep(index),
            Entry::Unpin => self.unpin(index),
            Entry::Quit => {
                self.request(index, Request::Close);
                Task::none()
            }
        };
        Task::batch([close, task])
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
        self.pinned.push(id);
        self.save_pinned()
    }

    /// Removes icon `index` from the pinned list. Sherlock stays in the middle; an application
    /// still open moves after the separator.
    pub fn unpin(&mut self, index: usize) -> Task<Message> {
        let Some(id) = self
            .items
            .get(index)
            .filter(|i| i.pinned)
            .and_then(Item::entry)
            .map(|e| e.id.clone())
        else {
            return Task::none();
        };
        tracing::info!(app = id, "unpinned");
        self.pinned.retain(|p| *p != id);
        self.save_pinned()
    }

    /// Writes the pinned list to the configuration and rebuilds the dock.
    fn save_pinned(&mut self) -> Task<Message> {
        self.sources.config.pinned = Some(self.pinned.clone());
        if let Some(path) = &self.sources.config_path
            && let Err(error) = self.sources.config.save(path)
        {
            tracing::warn!(%error, path = %path.display(), "cannot save the pinned applications");
        }
        self.refresh(self.geometry)
    }
}

/// Tab moves the focus; the other keys go to the menu when it is open, to the dock otherwise.
fn on_key(event: keyboard::Event) -> Option<Message> {
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
