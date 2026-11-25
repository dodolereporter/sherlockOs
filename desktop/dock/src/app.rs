//! Dock state, messages, and its two views: the dock and the menu of an icon.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::widget::{container, mouse_area, pin, row, space, stack, svg, text};
use iced::window::Id;
use iced::{Alignment, Element, Length, Point, Rectangle, Size, Subscription, Task};
use iced::{keyboard, mouse, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{Anchor, LayerSize, PixelSize, PopupAnchor, PopupGravity, Position};
use iced_exwlshell::to_layer_message;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::logo::Logo;
use sherlock_ui::motion::{Animation, Fade, GEOMETRY_PRECISION};
use sherlock_ui::{color, dock, metrics, typography};

use crate::config::Config;
use crate::entry::{Catalog, DesktopEntry};
use crate::icons::IconLookup;
use crate::launch::{self, What};
use crate::layout::{self, Layout};
use crate::menu::{self, Abilities, Entry, Key};
use crate::windows::{self, Windows};

/// What an icon stands for.
#[derive(Debug, Clone)]
pub enum Kind {
    /// Sherlock, in the middle of the dock. It has no window to open yet.
    Sherlock,
    App(Box<DesktopEntry>),
}

/// How an icon is drawn.
#[derive(Debug, Clone)]
pub enum Glyph {
    /// An icon file. A PNG goes through an SVG that embeds it: tiny-skia's SVG renderer draws
    /// both, without iced's `image` feature and the decoders it would add.
    File(svg::Handle),
    /// The color logo of `sherlock-ui` on its night blue tile, until Sherlock has its icon.
    Sherlock,
    /// No icon found: the initial of the name on a tile.
    Letter(char),
}

/// An icon of the dock.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: Kind,
    pub name: String,
    pub glyph: Glyph,
}

impl Item {
    pub fn sherlock() -> Self {
        Self {
            kind: Kind::Sherlock,
            name: String::from(dock::text::SHERLOCK),
            glyph: Glyph::Sherlock,
        }
    }

    /// An application, its icon found by `lookup` at the size of a magnified icon.
    pub fn app(entry: DesktopEntry, lookup: &IconLookup) -> Self {
        let size = (metrics::DOCK_ICON * metrics::DOCK_MAGNIFICATION).round() as u32;
        let file = entry
            .icon
            .as_deref()
            .and_then(|name| lookup.find(name, size));
        let glyph = match file {
            Some(path) => Glyph::File(icon_handle(&path)),
            None => Glyph::Letter(entry.name.chars().next().unwrap_or('?')),
        };
        Self {
            name: entry.name.clone(),
            kind: Kind::App(Box::new(entry)),
            glyph,
        }
    }

    fn entry(&self) -> Option<&DesktopEntry> {
        match &self.kind {
            Kind::App(entry) => Some(entry),
            Kind::Sherlock => None,
        }
    }
}

/// An SVG file as it is; any other image inside an SVG.
fn icon_handle(path: &Path) -> svg::Handle {
    if path.extension().is_some_and(|e| e == "svg") {
        return svg::Handle::from_path(path);
    }
    let href = path
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"{href}\"/></svg>"
    );
    svg::Handle::from_memory(document.into_bytes())
}

/// The icons of the dock: the pinned applications that are installed, Sherlock in the middle.
pub fn items(catalog: &Catalog, pinned: &[String], lookup: &IconLookup) -> Vec<Item> {
    let mut items: Vec<Item> = pinned
        .iter()
        .filter_map(|id| match catalog.get(id) {
            Some(entry) => Some(Item::app(entry.clone(), lookup)),
            None => {
                tracing::warn!(id, "pinned application not installed");
                None
            }
        })
        .collect();
    items.insert(items.len().div_ceil(2), Item::sherlock());
    items
}

/// The menu of an icon, while it is open.
#[derive(Debug, Clone)]
struct OpenMenu {
    id: Id,
    item: usize,
    entries: Vec<(Entry, bool)>,
    cursor: Option<usize>,
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
    Key(Key),
    Focus(Direction),
    MenuHover(usize),
    MenuPick(Entry),
    Closed(Id),
}

pub struct Dock {
    pub items: Vec<Item>,
    /// Desktop file ids of the pinned applications, from the configuration or the defaults.
    pub pinned: Vec<String>,
    config: Config,
    config_path: Option<PathBuf>,
    /// Size of the dock surface.
    pub surface_size: Size,
    surface: Option<Id>,
    /// Side of each icon, led by a spring towards the size the pointer gives it.
    sizes: Vec<Animation>,
    /// Opacity of the label, and the icon it names.
    label: Fade,
    label_item: Option<usize>,
    /// The pointer is over the dock.
    hovering: bool,
    pointer: Option<Point>,
    origin: Instant,
    /// Time of the last event, from `origin`: the moment the view draws.
    now: Duration,
    pub windows: Option<Windows>,
    /// Icon with the keyboard focus, kept in our state like the bar does.
    focus: Option<usize>,
    menu: Option<OpenMenu>,
    /// Frames drawn since `frames_since`, for the frame rate in the debug log.
    frames: u32,
    frames_since: Duration,
}

impl Dock {
    pub fn new(
        items: Vec<Item>,
        pinned: Vec<String>,
        config: Config,
        config_path: Option<PathBuf>,
    ) -> Self {
        let surface_size = layout::surface_size(items.len());
        let fade = Duration::from_millis(dock::LABEL_FADE_MS);
        Self {
            sizes: rest(items.len()),
            items,
            pinned,
            config,
            config_path,
            surface_size,
            surface: None,
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
        }
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Whether something still moves: the subscription to frames lives only that long.
    pub fn animating(&self) -> bool {
        !self.label.is_done(self.now)
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
        Subscription::batch([
            Subscription::run(|| windows::watch(windows::source)).map(Message::Windows),
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            frames,
        ])
    }

    /// Layout of the dock at `now`.
    pub fn layout(&self, now: Duration) -> Layout {
        let sizes: Vec<f32> = self.sizes.iter().map(|s| s.value(now) as f32).collect();
        Layout::new(self.surface_size, &sizes)
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
            Message::Frame(instant) => self.frame(instant.saturating_duration_since(self.origin)),
            Message::Windows(windows) => self.windows = Some(windows),
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

    /// The pointer moved to `point`, or left the surface (`None`), at `now`: the icons head for
    /// the sizes of the board's curve, the label follows the closest icon. Returns whether the
    /// pointer entered or left the dock.
    pub fn hover(&mut self, point: Option<Point>, now: Duration) -> bool {
        self.now = now;
        self.pointer = point;
        let layout = self.layout(now);
        let inside = point.filter(|p| layout.contains(*p));
        let count = self.items.len();
        let ratio = inside.map(|p| layout.ratio(p.x));
        for (size, target) in self.sizes.iter_mut().zip(layout::targets(ratio, count)) {
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
        self.now = now;
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
                    (mouse::Button::Left, Some(index)) => self.activate(index, What::App),
                    (mouse::Button::Right, Some(index)) => return self.open_menu(index),
                    _ => {}
                }
            }
        }
        Task::none()
    }

    /// Where the dock takes the pointer: the bar at rest, the whole surface while hovered, so
    /// that a magnified icon above the bar stays reachable. Elsewhere, clicks go through.
    fn input_region(&self) -> Task<Message> {
        let Some(id) = self.surface else {
            return Task::none();
        };
        let area = if self.hovering {
            Rectangle::with_size(self.surface_size)
        } else {
            Layout::new(self.surface_size, &layout::targets(None, self.items.len())).bar
        };
        let callback = RegionActionCB::new(move |region| {
            let added = region.add(
                area.x.floor() as i32,
                area.y.floor() as i32,
                area.width.ceil() as i32,
                area.height.ceil() as i32,
            );
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
            (Key::Activate, Some(index)) => self.activate(index, What::App),
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
        Some(match self.items.get(index)?.entry() {
            Some(entry) => windows.of(|app_id| entry.owns(app_id)),
            None => (0, false),
        })
    }

    fn abilities(&self, index: usize) -> Abilities {
        let entry = self.items.get(index).and_then(Item::entry);
        let closable = self.windows.as_ref().is_some_and(|w| w.closable);
        let open = self.open_windows(index).is_some_and(|(n, _)| n > 0);
        Abilities {
            launch: entry.is_some_and(|e| e.exec.is_some()),
            new_window: entry.is_some_and(|e| e.new_window.is_some()),
            unpin: entry.is_some(),
            quit: entry.is_some() && closable && open,
        }
    }

    /// Starts `what` for icon `index`.
    pub fn activate(&self, index: usize, what: What) {
        let Some(item) = self.items.get(index) else {
            return;
        };
        let Some(entry) = item.entry() else {
            tracing::info!("Sherlock has no window to open yet");
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
        // Centered above the icon, 12 above the bar. The gap is not part of the menu surface:
        // it would be drawn black over the magnified icons (opaque software rendering).
        let anchor = Position::new(
            (icon.x + icon.width / 2.0) as i32,
            (layout.bar.y - dock::MENU_GAP) as i32,
        );
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(width, height),
            anchor,
            PixelSize::px(1, 1),
        )
        .anchor(PopupAnchor::Top)
        .gravity(PopupGravity::Top);
        let id = Id::unique();
        self.menu = Some(OpenMenu {
            id,
            item: index,
            entries,
            cursor: None,
        });
        Task::batch([close, Task::done(Message::NewPopUp { settings, id })])
    }

    fn close_menu(&mut self) -> Task<Message> {
        match self.menu.take() {
            Some(menu) => window::close(menu.id),
            None => Task::none(),
        }
    }

    fn pick(&mut self, entry: Entry) -> Task<Message> {
        let Some(index) = self.menu.as_ref().map(|m| m.item) else {
            return Task::none();
        };
        let close = self.close_menu();
        match entry {
            Entry::Open => self.activate(index, What::App),
            Entry::NewWindow => self.activate(index, What::NewWindow),
            Entry::Unpin => return Task::batch([close, self.unpin(index)]),
            Entry::Quit => self.quit(index),
        }
        close
    }

    /// Closes the windows of icon `index`, on a thread: the compositor may take its time.
    fn quit(&self, index: usize) {
        let Some(entry) = self.items.get(index).and_then(Item::entry).cloned() else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name(String::from("quit"))
            .spawn(move || {
                let closed = wayland_client::Connection::connect_to_env()
                    .map_err(windows::SourceError::from)
                    .and_then(|c| windows::close(c, &|app_id| entry.owns(app_id)));
                match closed {
                    Ok(count) => tracing::info!(app = entry.id, count, "windows asked to close"),
                    Err(error) => tracing::warn!(app = entry.id, %error, "cannot close"),
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot start the closing thread");
        }
    }

    /// Removes icon `index` from the dock and from the configuration. Sherlock stays in the
    /// middle, the surface takes its new size.
    pub fn unpin(&mut self, index: usize) -> Task<Message> {
        let Some(removed) = self
            .items
            .get(index)
            .and_then(Item::entry)
            .map(|e| e.id.clone())
        else {
            return Task::none();
        };
        self.pinned.retain(|id| *id != removed);
        if let Some(path) = &self.config_path
            && let Err(error) = self.config.save(path, &self.pinned)
        {
            tracing::warn!(%error, path = %path.display(), "cannot save the pinned applications");
        }
        let mut items: Vec<Item> = std::mem::take(&mut self.items)
            .into_iter()
            .filter(|i| i.entry().is_some_and(|e| e.id != removed))
            .collect();
        items.insert(items.len().div_ceil(2), Item::sherlock());
        self.sizes = rest(items.len());
        self.surface_size = layout::surface_size(items.len());
        self.items = items;
        self.label_item = None;
        self.focus = None;
        self.hovering = false;
        tracing::info!(app = removed, "unpinned");
        let Some(id) = self.surface else {
            return Task::none();
        };
        let size = LayerSize::px(
            self.surface_size.width as u32,
            self.surface_size.height as u32,
        );
        let resize = Task::done(Message::LayoutChange {
            id,
            anchor: Anchor::Bottom,
            size,
        });
        Task::batch([resize, self.input_region()])
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        if self.menu.as_ref().is_some_and(|m| m.id == id) {
            self.view_menu()
        } else {
            self.view_dock()
        }
    }

    pub fn view_dock(&self) -> Element<'_, Message> {
        let layout = self.layout(self.now);
        let bar = layout.bar;
        let glass = container(space())
            .width(bar.width)
            .height(bar.height)
            .style(dock::bar);
        let mut layers = vec![place(glass, bar.x, bar.y)];
        for (index, (item, icon)) in self.items.iter().zip(&layout.icons).enumerate() {
            layers.push(place(glyph(&item.glyph, icon.width), icon.x, icon.y));
            if self.focus == Some(index) {
                let pad = metrics::FOCUS_RING_GAP + metrics::FOCUS_RING_WIDTH;
                let ring = container(space())
                    .width(icon.width + 2.0 * pad)
                    .height(icon.height + 2.0 * pad)
                    .style(dock::ring(icon.width));
                layers.push(place(ring, icon.x - pad, icon.y - pad));
            }
            if let Some((count, active)) = self.open_windows(index)
                && count > 0
            {
                let dot = container(space())
                    .width(dock::DOT)
                    .height(dock::DOT)
                    .style(dock::dot(active));
                let x = icon.x + (icon.width - dock::DOT) / 2.0;
                let y = icon.y + icon.height + dock::DOT_OFFSET - dock::DOT;
                layers.push(place(dot, x, y));
            }
        }
        if let Some(label) = self.view_label(&layout) {
            layers.push(label);
        }
        // The pointer is tracked over the whole surface: `mouse_area` also reports where the
        // pointer enters, which the bare enter event does not carry.
        let layers = stack(layers).width(Length::Fill).height(Length::Fill);
        mouse_area(layers)
            .on_move(|point| Message::Pointer(Pointer::Moved(point)))
            .on_exit(Message::Pointer(Pointer::Left))
            .on_press(Message::Pointer(Pointer::Pressed(mouse::Button::Left)))
            .on_right_press(Message::Pointer(Pointer::Pressed(mouse::Button::Right)))
            .into()
    }

    /// The name of the icon closest to the pointer, centered above it (board: 108 above the
    /// bottom of the bar), kept inside the surface.
    fn view_label(&self, layout: &Layout) -> Option<Element<'_, Message>> {
        let opacity = self.label.value(self.now);
        let index = self.label_item?;
        if opacity <= 0.0 {
            return None;
        }
        let icon = layout.icons.get(index)?;
        let name = self.items.get(index)?.name.as_str();
        let (vertical, horizontal) = dock::LABEL_PADDING;
        let line = dock::LABEL_SIZE * typography::LINE_HEIGHT;
        let pill = container(
            text(name)
                .size(dock::LABEL_SIZE)
                .line_height(text::LineHeight::Relative(typography::LINE_HEIGHT))
                .font(typography::UI_SEMIBOLD),
        )
        .padding([vertical, horizontal])
        .style(dock::label(opacity));
        let center = icon.x + icon.width / 2.0;
        let span = 2.0 * center.min(self.surface_size.width - center).max(0.0);
        let bottom = self.surface_size.height - metrics::BORDER_WIDTH - dock::LABEL_BOTTOM;
        let top = bottom - line - 2.0 * vertical;
        let centered = container(pill).width(span).align_x(Alignment::Center);
        Some(place(centered, center - span / 2.0, top))
    }

    pub fn view_menu(&self) -> Element<'_, Message> {
        let Some(menu) = &self.menu else {
            return space().into();
        };
        let name = self.items.get(menu.item).map_or("", |i| i.name.as_str());
        let mut header = row![
            ui::body(name).font(typography::UI_SEMIBOLD),
            space::horizontal()
        ]
        .align_y(Alignment::Center)
        .height(dock::MENU_HEADER_HEIGHT)
        .padding([0.0, metrics::MENU_ITEM_PADDING]);
        if let Some((count, _)) = self.open_windows(menu.item)
            && count > 0
        {
            let count = text(dock::text::windows(count))
                .size(typography::SMALL_SIZE)
                .color(color::TEXT_SECONDARY);
            header = header.push(count);
        }
        let mut entries: Vec<Element<'_, Message>> = vec![header.into()];
        for (index, (entry, enabled)) in menu.entries.iter().copied().enumerate() {
            if entry.starts_group() {
                entries.push(ui::menu_separator());
            }
            let on_press = enabled.then_some(Message::MenuPick(entry));
            let mut item = ui::menu_item(entry.label(), Trailing::None, on_press);
            if menu.cursor == Some(index) {
                item = ui::open(item);
            }
            entries.push(mouse_area(item).on_enter(Message::MenuHover(index)).into());
        }
        ui::menu(entries, dock::MENU_WIDTH)
            .height(Length::Fill)
            .into()
    }
}

/// Every icon at rest.
fn rest(count: usize) -> Vec<Animation> {
    (0..count)
        .map(|_| {
            let side = f64::from(metrics::DOCK_ICON);
            Animation::new(dock::SPRING, side, side, Duration::ZERO)
        })
        .collect()
}

fn place<'a>(content: impl Into<Element<'a, Message>>, x: f32, y: f32) -> Element<'a, Message> {
    pin(content).x(x).y(y).into()
}

/// An icon of side `side`.
fn glyph(glyph: &Glyph, side: f32) -> Element<'static, Message> {
    match glyph {
        Glyph::File(handle) => svg(handle.clone()).width(side).height(side).into(),
        Glyph::Sherlock => tile(
            ui::logo(Logo::Color, side * dock::SHERLOCK_GLYPH_RATIO),
            dock::SHERLOCK_TILE,
            side,
        ),
        Glyph::Letter(letter) => {
            let letter = text(letter.to_string())
                .size(side * dock::LETTER_RATIO)
                .font(typography::TITLE)
                .color(dock::FALLBACK_LETTER);
            tile(letter, dock::FALLBACK_TILE, side)
        }
    }
}

fn tile<'a>(
    content: impl Into<Element<'a, Message>>,
    tint: color::Color,
    side: f32,
) -> Element<'a, Message> {
    container(content)
        .width(side)
        .height(side)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(dock::tile(tint, side))
        .into()
}

/// Tab moves the focus; the other keys go to the menu when it is open, to the dock otherwise.
fn on_key(event: keyboard::Event) -> Option<Message> {
    focus::tab(event.clone())
        .map(Message::Focus)
        .or_else(|| menu::key(event).map(Message::Key))
}
