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
use sherlock_ui::motion::{Animation, Fade, GEOMETRY_PRECISION};

use crate::entry::Catalog;
use crate::icons::IconLookup;
use crate::items::{Builder, Glyph, Item, Target};
use crate::launch::{self, What};
use crate::layout::{self, Geometry, Layout};
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
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Pointer(Pointer),
    Frame(Instant),
    Windows(Windows),
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
    /// Side of each icon, led by a spring towards the size the pointer gives it.
    sizes: Vec<Animation>,
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
            sizes: Vec::new(),
            label: Fade::new(0.0, 0.0, Duration::ZERO, fade),
            label_item: None,
            hovering: false,
            pointer: None,
            origin: Instant::now(),
            now: Duration::ZERO,
            windows: None,
            focus: None,
            menu: None,
            frames: 0,
            frames_since: Duration::ZERO,
            theme: ThemeFade::default(),
        };
        dock.rebuild();
        dock
    }

    /// Builds the icons again from the pinned list and the open windows. Springs are kept when
    /// the number of icons stays, reset to rest otherwise.
    fn rebuild(&mut self) {
        let icon_size = (self.geometry.icon * self.geometry.factor).round() as u32;
        let mut builder = Builder {
            catalog: &self.sources.catalog,
            lookup: &self.sources.lookup,
            icon_size,
            glyphs: &mut self.glyphs,
        };
        let (items, separator) = builder.build(&self.pinned, self.windows.as_ref());
        if items.len() != self.sizes.len() {
            self.sizes = rest(items.len(), self.geometry.icon);
            self.label_item = None;
            self.focus = None;
        }
        self.items = items;
        self.separator = separator;
        self.surface_size = self
            .geometry
            .surface_size(self.items.len(), self.separator.is_some());
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Whether something still moves: the subscription to frames lives only that long.
    pub fn animating(&self) -> bool {
        self.theme.running()
            || !self.label.is_done(self.now)
            || self
                .sizes
                .iter()
                .any(|s| !s.is_done(self.now, GEOMETRY_PRECISION))
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
        Subscription::batch([
            appearance,
            Subscription::run(|| windows::watch(windows::source)).map(Message::Windows),
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            config,
            frames,
        ])
    }

    /// Layout of the dock at `now`.
    pub fn layout(&self, now: Duration) -> Layout {
        let sizes: Vec<f32> = self.sizes.iter().map(|s| s.value(now) as f32).collect();
        Layout::new(self.geometry, self.surface_size, &sizes, self.separator)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surface = Some(id);
                return self.input_region();
            }
            Message::Pointer(event) => {
                let now = self.origin.elapsed();
                return self.pointer(event, now);
            }
            Message::Frame(instant) => {
                self.theme.tick(instant);
                self.frame(instant.saturating_duration_since(self.origin));
            }
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Windows(windows) => {
                self.windows = Some(windows);
                return self.refresh(self.geometry);
            }
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
        if self.geometry.icon != previous.icon {
            self.sizes = rest(self.sizes.len(), self.geometry.icon);
        }
        self.sources.config = config;
        self.refresh(previous)
    }

    /// Rebuilds the icons and, when the surface changed, tells the compositor: its size, and
    /// its anchor, margin and exclusive zone when the edge changed.
    fn refresh(&mut self, previous: Geometry) -> Task<Message> {
        let size = self.surface_size;
        self.rebuild();
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
            .sizes
            .iter_mut()
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
                match (button, target) {
                    (mouse::Button::Left, Some(index)) => self.click(index),
                    (mouse::Button::Right, Some(index)) => return self.open_menu(index),
                    _ => {}
                }
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
    pub fn click(&self, index: usize) {
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

    /// Starts `what` for icon `index`.
    pub fn activate(&self, index: usize, what: What) {
        let Some(item) = self.items.get(index) else {
            return;
        };
        let Some(entry) = item.entry() else {
            tracing::info!(name = item.name, "nothing to start for this icon yet");
            return;
        };
        match launch::launch(entry, what) {
            Ok(pid) => tracing::info!(app = entry.id, pid, ?what, "started"),
            Err(error) => tracing::warn!(app = entry.id, %error, "cannot start"),
        }
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

/// Every icon at rest.
fn rest(count: usize, side: f32) -> Vec<Animation> {
    let side = f64::from(side);
    (0..count)
        .map(|_| Animation::new(dock::SPRING, side, side, Duration::ZERO))
        .collect()
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
