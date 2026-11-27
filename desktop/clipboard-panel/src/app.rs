//! The panel: state, messages, update and view.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use iced::keyboard::{self, key::Named};
use iced::widget::{
    Space, button, column, container, mouse_area, operation, row, stack, svg, text, text_input,
};
use iced::window::{self, Id};
use iced::{Alignment, Element, Event, Length, Size, Subscription, Task, event, mouse};
use iced_exwlshell::reexport::{Anchor, LayerSize};
use iced_exwlshell::to_layer_message;
use sherlock_ui::animated::reveal;
use sherlock_ui::clipboard::{self as look, Glyph, text as texts};
use sherlock_ui::component as ui;
use sherlock_ui::ellipsis::{MARK, ellipsis};
use sherlock_ui::icon::Icon;
use sherlock_ui::launcher as glass;
use sherlock_ui::motion::Animation;
use sherlock_ui::{color, metrics, typography};

use crate::history::{self, Event as History};
use crate::items::{self, Item, Kind};

/// Id of the search field, to give it the keyboard.
pub const QUERY: &str = "query";

/// The keys the panel acts on, whatever has the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Escape,
    Delete,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    History(History),
    Query(String),
    Key(Key),
    /// A row clicked: put back.
    Pick(u32),
    Pin(u32, bool),
    Remove(u32),
    Clear,
    /// The wheel over the list: rows to move by.
    Scroll(i32),
    Resized(Size),
    /// A click beside the panel: it closes.
    Dismiss,
    Inside,
    Frame(Instant),
    Theme(color::Theme),
}

/// What the panel shows besides the items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Waiting for the compositor.
    Waiting,
    On,
    Off,
    Unavailable,
}

pub struct Panel {
    pub items: Vec<Item>,
    pub status: Status,
    pub query: String,
    /// The chosen row, among the items the query finds.
    pub selected: usize,
    /// The first row shown.
    pub first: usize,
    /// Rows shown at once, for the height of the screen.
    pub rows: usize,
    thumbnails: HashMap<u32, svg::Handle>,
    surface: Option<Id>,
    /// The surface was made the panel's own size: the renderer has no alpha.
    pub shrunk: bool,
    pub closing: bool,
    presence: Animation,
    origin: Instant,
    now: Duration,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            status: Status::Waiting,
            query: String::new(),
            selected: 0,
            first: 0,
            rows: look::VISIBLE_ROWS,
            thumbnails: HashMap::new(),
            surface: None,
            shrunk: false,
            closing: false,
            presence: Animation::new(look::OPEN_SPRING, 0.0, 1.0, Duration::ZERO),
            origin: Instant::now(),
            now: Duration::ZERO,
        }
    }
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Panel {
    /// The items the query finds, in order.
    pub fn found(&self) -> Vec<&Item> {
        items::found(&self.items, &self.query)
    }

    pub fn presence(&self) -> f64 {
        self.presence.value(self.now)
    }

    fn animating(&self) -> bool {
        self.surface.is_some() && !self.presence.is_done(self.now, look::OPEN_PRECISION)
    }

    fn clock(&mut self) -> Duration {
        self.now = self.now.max(self.origin.elapsed());
        self.now
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let frames = match self.animating() {
            true => window::frames().map(Message::Frame),
            false => Subscription::none(),
        };
        Subscription::batch([
            event::listen_with(on_event),
            frames,
            Subscription::run(history::events).map(Message::History),
            Subscription::run(launcher::appearance).map(Message::Theme),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surface = Some(id);
                let now = self.clock();
                self.presence = Animation::new(look::OPEN_SPRING, 0.0, 1.0, now);
                return operation::focus(QUERY);
            }
            Message::History(event) => self.learn(event),
            Message::Query(query) => {
                self.query = query;
                self.selected = 0;
                self.first = 0;
            }
            Message::Key(Key::Down) | Message::Scroll(1..) => self.select(1),
            Message::Key(Key::Up) | Message::Scroll(..=-1) => self.select(-1),
            Message::Key(Key::Enter) => {
                if let Some(id) = self.found().get(self.selected).map(|i| i.id) {
                    return self.put_back(id);
                }
            }
            Message::Key(Key::Delete) => {
                if let Some(id) = self.found().get(self.selected).map(|i| i.id) {
                    self.remove(id);
                }
            }
            Message::Key(Key::Escape) | Message::Dismiss => return self.close(),
            Message::Pick(id) => return self.put_back(id),
            Message::Pin(id, pinned) => {
                if let Some(control) = history::control() {
                    control.set_pinned(id, pinned);
                }
            }
            Message::Remove(id) => self.remove(id),
            Message::Clear => {
                if let Some(control) = history::control() {
                    control.clear();
                }
            }
            Message::Resized(size) => return self.resized(size),
            Message::Frame(at) => return self.frame(at),
            Message::Theme(theme) => color::set_active(theme),
            _ => {}
        }
        Task::none()
    }

    /// A new state of the history: the selection stays on its item when it can.
    fn learn(&mut self, event: History) {
        let History::State { enabled, items } = event else {
            self.status = Status::Unavailable;
            return;
        };
        let kept = self.found().get(self.selected).map(|i| i.id);
        self.status = if enabled { Status::On } else { Status::Off };
        self.thumbnails
            .retain(|id, _| items.iter().any(|i| i.id == *id));
        for item in &items {
            if let Some(png) = &item.thumbnail {
                self.thumbnails
                    .entry(item.id)
                    .or_insert_with(|| svg::Handle::from_memory(picture(png).into_bytes()));
            }
        }
        self.items = items;
        let found = self.found();
        self.selected = kept
            .and_then(|id| found.iter().position(|i| i.id == id))
            .unwrap_or(self.selected)
            .min(found.len().saturating_sub(1));
        self.follow();
    }

    /// Moves the selection by `step` rows, within the list.
    fn select(&mut self, step: i32) {
        let last = self.found().len().saturating_sub(1);
        self.selected = match step {
            1.. => (self.selected + 1).min(last),
            _ => self.selected.saturating_sub(1),
        };
        self.follow();
    }

    /// The rows shown follow the selection.
    fn follow(&mut self) {
        let rows = self.rows.max(1);
        if self.selected < self.first {
            self.first = self.selected;
        } else if self.selected >= self.first + rows {
            self.first = self.selected + 1 - rows;
        }
        let count = self.found().len();
        self.first = self.first.min(count.saturating_sub(rows));
    }

    fn remove(&mut self, id: u32) {
        if let Some(control) = history::control() {
            control.remove(id);
        }
    }

    /// Puts item `id` back on the clipboard, then closes: the keyboard goes back to the window
    /// that had it, where the user pastes (ADR 0029).
    fn put_back(&mut self, id: u32) -> Task<Message> {
        if self.closing {
            return Task::none();
        }
        if let Some(control) = history::control() {
            control.paste(id);
        }
        self.close()
    }

    fn close(&mut self) -> Task<Message> {
        if !self.closing {
            self.closing = true;
            let now = self.clock();
            self.presence.retarget(now, 0.0);
        }
        Task::none()
    }

    /// The surface's size is the screen's height: the rows that fit. Drawn in software, its
    /// surface has no alpha: it becomes the panel's own size.
    fn resized(&mut self, size: Size) -> Task<Message> {
        if self.shrunk {
            return Task::none();
        }
        self.rows = look::rows_for_screen(size.height);
        self.follow();
        let software = sherlock_ui::surface::gpu_drawn() == Some(false);
        match self.surface {
            Some(id) if software => {
                self.shrunk = true;
                Task::done(Message::LayoutChange {
                    id,
                    anchor: Anchor::empty(),
                    size: LayerSize::px(
                        look::WIDTH as u32,
                        look::panel_height(self.rows).ceil() as u32,
                    ),
                })
            }
            _ => Task::none(),
        }
    }

    fn frame(&mut self, at: Instant) -> Task<Message> {
        self.now = self.now.max(at.saturating_duration_since(self.origin));
        if self.closing && self.presence.is_done(self.now, look::OPEN_PRECISION) {
            return iced::exit();
        }
        Task::none()
    }

    pub fn view(&self, _: Id) -> Element<'_, Message> {
        let shown = self.presence() as f32;
        let scale = look::OPEN_SCALE + (1.0 - look::OPEN_SCALE) * shown;
        let panel = reveal(self.view_panel())
            .opacity(shown.clamp(0.0, 1.0))
            .scale(scale, (0.5, 0.5));
        if self.shrunk {
            return panel.into();
        }
        let panel = mouse_area(container(panel).center(Length::Fill))
            .on_press(Message::Inside)
            .on_right_press(Message::Inside);
        let beside = mouse_area(Space::new().width(Length::Fill).height(Length::Fill))
            .on_press(Message::Dismiss)
            .on_right_press(Message::Dismiss);
        stack![beside, panel]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    pub fn view_panel(&self) -> Element<'_, Message> {
        let p = color::palette();
        let field = text_input(texts::PLACEHOLDER, &self.query)
            .id(QUERY)
            .on_input(Message::Query)
            .size(glass::QUERY_SIZE)
            .padding(0)
            .style(glass::query);
        let search = row![
            ui::icon(Icon::Search, glass::SEARCH_ICON, p.accent_text),
            field,
            key_cap(glass::text::ESCAPE),
        ]
        .spacing(glass::GAP)
        .align_y(Alignment::Center)
        .padding([0.0, glass::SEARCH_PADDING])
        .height(glass::SEARCH_HEIGHT);
        let divider = container(Space::new())
            .width(Length::Fill)
            .height(metrics::BORDER_WIDTH)
            .style(glass::divider);
        let found = self.found();
        let mut list = column![];
        match self.message(found.is_empty()) {
            Some((line, second)) => {
                let mut words = column![ui::body(line).color(p.text_secondary)];
                if let Some(second) = second {
                    words = words.push(
                        text(second)
                            .size(typography::SMALL_SIZE)
                            .color(p.text_tertiary),
                    );
                }
                list = list.push(
                    container(words)
                        .height(look::ROW_HEIGHT)
                        .padding([0.0, glass::ROW_PADDING])
                        .center_y(look::ROW_HEIGHT),
                );
            }
            None => {
                let now = now_seconds();
                for (index, item) in found.iter().enumerate().skip(self.first).take(self.rows) {
                    list = list.push(self.view_item(item, index == self.selected, now));
                }
            }
        }
        let list = mouse_area(list).on_scroll(|delta| {
            let y = match delta {
                mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => y,
            };
            Message::Scroll(if y > 0.0 { -1 } else { 1 })
        });
        let shown = found.len().clamp(1, self.rows);
        let clearable = self.items.iter().any(|i| !i.pinned);
        let clear = button(text(texts::CLEAR).size(look::FOOTER_SIZE))
            .on_press_maybe(clearable.then_some(Message::Clear))
            .padding([4.0, 10.0])
            .style(glass::row(false));
        let footer = row![
            text(texts::FOOTER)
                .size(look::FOOTER_SIZE)
                .color(p.text_secondary),
            Space::new().width(Length::Fill),
            clear,
        ]
        .align_y(Alignment::Center)
        .padding([0.0, glass::ROW_PADDING])
        .height(look::FOOTER_HEIGHT);
        let content = column![
            search,
            divider,
            container(list).height(shown as f32 * look::ROW_HEIGHT),
            footer
        ];
        container(content)
            .width(look::WIDTH)
            .height(look::panel_height(shown))
            .padding(glass::PADDING + metrics::BORDER_WIDTH)
            .style(glass::panel)
            .into()
    }

    /// The line shown instead of the items, and its second line.
    fn message(&self, nothing: bool) -> Option<(&'static str, Option<&'static str>)> {
        match self.status {
            Status::Off => Some((texts::OFF, Some(texts::OFF_LINE))),
            Status::Unavailable => Some((texts::UNAVAILABLE, None)),
            _ if !nothing => None,
            _ if self.query.trim().is_empty() => Some((texts::EMPTY, None)),
            _ => Some((texts::NOTHING, None)),
        }
    }

    fn view_item<'a>(&'a self, item: &'a Item, selected: bool, now: u64) -> Element<'a, Message> {
        let p = color::palette();
        let mut lines = preview_lines(item).into_iter();
        let mut words = column![];
        for line in lines.by_ref().take(look::PREVIEW_LINES) {
            words = words.push(
                ellipsis(line)
                    .font(typography::UI_MEDIUM)
                    .color(p.text_primary)
                    .width(Length::Fill),
            );
        }
        let app = match item.app_name() {
            name if name.is_empty() && item.kind == Kind::Image => texts::SCREENSHOT.to_owned(),
            name => name,
        };
        let age = look::age(now.saturating_sub(item.time));
        let meta = match app.is_empty() {
            true => age,
            false => format!("{app}{}{age}", texts::SEPARATOR),
        };
        words = words.push(
            ellipsis(meta)
                .size(typography::SMALL_SIZE)
                .color(p.text_secondary)
                .width(Length::Fill),
        );
        let mut line = row![self.tile(item), words.width(Length::Fill)]
            .spacing(glass::GAP)
            .height(Length::Fill)
            .align_y(Alignment::Center);
        if selected || item.pinned {
            let label = if item.pinned {
                texts::UNPIN
            } else {
                texts::PIN
            };
            line = line.push(action(
                Glyph::Pin,
                label,
                item.pinned,
                Message::Pin(item.id, !item.pinned),
            ));
        }
        if selected {
            line = line.push(action(
                Glyph::Remove,
                texts::REMOVE,
                false,
                Message::Remove(item.id),
            ));
        }
        button(line)
            .on_press(Message::Pick(item.id))
            .width(Length::Fill)
            .height(look::ROW_HEIGHT)
            .padding([0.0, glass::ROW_PADDING])
            .style(glass::row(selected))
            .into()
    }

    /// The thumbnail of a picture, or the glyph of the item's kind.
    fn tile<'a>(&self, item: &Item) -> Element<'a, Message> {
        if let Some(handle) = self.thumbnails.get(&item.id) {
            return container(svg(handle.clone()).width(look::TILE).height(look::TILE))
                .center(look::TILE)
                .into();
        }
        let glyph = match item.kind {
            Kind::Text => Glyph::Text,
            Kind::Link => Glyph::Link,
            Kind::Files => Glyph::Files,
            Kind::Image => Glyph::Image,
        };
        let stroke = glass::Colors::of(color::palette()).tile_glyph;
        container(glyph_svg(glyph, glass::TILE_GLYPH, stroke))
            .center(look::TILE)
            .style(glass::glyph_tile(launcher_glyph(item.kind)))
            .into()
    }
}

/// The launcher's tile colors: files on the file tile, the rest on the system one.
fn launcher_glyph(kind: Kind) -> glass::Glyph {
    match kind {
        Kind::Files | Kind::Image => glass::Glyph::File,
        Kind::Text | Kind::Link => glass::Glyph::Lock,
    }
}

/// The lines to show: the names of files rather than their paths, a « … » on the last line
/// shown when more follow.
pub fn preview_lines(item: &Item) -> Vec<String> {
    let mut lines: Vec<String> = item
        .preview
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| match item.kind {
            Kind::Files => l.rsplit('/').next().unwrap_or(l).to_owned(),
            _ => l.trim().to_owned(),
        })
        .collect();
    if lines.len() > look::PREVIEW_LINES {
        lines.truncate(look::PREVIEW_LINES);
        if let Some(last) = lines.last_mut() {
            last.push_str(MARK);
        }
    }
    lines
}

/// A glyph in `tint`.
fn glyph_svg<'a>(glyph: Glyph, size: f32, tint: color::Color) -> svg::Svg<'a> {
    svg(svg::Handle::from_memory(glyph.svg().into_bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style {
            color: Some(tint.into()),
        })
}

/// A small button of the selected row: pin, remove. `on`: the pin is set.
fn action<'a>(glyph: Glyph, label: &'a str, on: bool, message: Message) -> Element<'a, Message> {
    let p = color::palette();
    let tint = if on { p.accent_text } else { p.text_secondary };
    let icon = container(glyph_svg(glyph, look::ACTION_ICON, tint)).center(look::ACTION);
    let button = button(icon)
        .on_press(message)
        .padding(0)
        .style(glass::row(false));
    iced::widget::tooltip(
        button,
        text(label).size(typography::SMALL_SIZE),
        Default::default(),
    )
    .into()
}

/// A key name in its cap.
fn key_cap(name: &str) -> Element<'_, Message> {
    let (vertical, side) = glass::KEY_PADDING;
    container(text(name).size(glass::KEY_SIZE))
        .padding([vertical, side])
        .style(glass::key(false))
        .into()
}

/// A small SVG document showing the PNG `png`, for the SVG renderer: iced's `image` feature
/// would bring its own decoders.
pub fn picture(png: &[u8]) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" preserveAspectRatio=\"xMidYMid meet\" \
         xlink:href=\"data:image/png;base64,{}\"/></svg>",
        base64(png)
    )
}

/// Standard base64, with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) as usize & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The arrows, Enter, Escape and Delete, even when the search field took them. A held Enter
/// repeats nothing.
pub fn on_event(event: Event, _: event::Status, _: Id) -> Option<Message> {
    match event {
        Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
        Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) => {
            let key = match key {
                keyboard::Key::Named(Named::ArrowUp) => Key::Up,
                keyboard::Key::Named(Named::ArrowDown) => Key::Down,
                keyboard::Key::Named(Named::Enter) if !repeat => Key::Enter,
                keyboard::Key::Named(Named::Escape) => Key::Escape,
                keyboard::Key::Named(Named::Delete) => Key::Delete,
                _ => return None,
            };
            Some(Message::Key(key))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, preview: &str) -> Item {
        Item::new(1, kind, false, 0, String::new(), preview.into())
    }

    fn state(items: Vec<Item>) -> History {
        History::State {
            enabled: true,
            items,
        }
    }

    #[test]
    fn base64_is_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn two_lines_at_most_and_names_of_files() {
        let text = item(Kind::Text, "a\n\n b \nc");
        assert_eq!(preview_lines(&text), ["a", "b…"]);
        let files = item(Kind::Files, "/home/u/a.txt\n/tmp/b.png");
        assert_eq!(preview_lines(&files), ["a.txt", "b.png"]);
    }

    #[test]
    fn the_keys_move_the_selection_and_the_rows_follow() {
        let mut panel = Panel {
            rows: 2,
            ..Panel::default()
        };
        let items = (1..=4)
            .map(|id| {
                Item::new(
                    id,
                    Kind::Text,
                    false,
                    u64::from(10 - id),
                    String::new(),
                    id.to_string(),
                )
            })
            .collect();
        let _ = panel.update(Message::History(state(items)));
        assert_eq!(panel.status, Status::On);
        for _ in 0..3 {
            let _ = panel.update(Message::Key(Key::Down));
        }
        assert_eq!((panel.selected, panel.first), (3, 2));
        let _ = panel.update(Message::Key(Key::Down));
        assert_eq!(panel.selected, 3);
        let _ = panel.update(Message::Key(Key::Up));
        let _ = panel.update(Message::Key(Key::Up));
        assert_eq!((panel.selected, panel.first), (1, 1));
        // The search starts from the top.
        let _ = panel.update(Message::Query("4".into()));
        assert_eq!(panel.found().len(), 1);
        assert_eq!((panel.selected, panel.first), (0, 0));
    }

    #[test]
    fn the_selection_stays_on_its_item_when_the_history_changes() {
        let mut panel = Panel::default();
        let a = Item::new(1, Kind::Text, false, 1, String::new(), "a".into());
        let b = Item::new(2, Kind::Text, false, 2, String::new(), "b".into());
        let _ = panel.update(Message::History(state(vec![b.clone(), a.clone()])));
        let _ = panel.update(Message::Key(Key::Down));
        let c = Item::new(3, Kind::Text, false, 3, String::new(), "c".into());
        let _ = panel.update(Message::History(state(vec![c, b, a])));
        assert_eq!(panel.found()[panel.selected].id, 1);
    }

    #[test]
    fn what_shows_instead_of_items() {
        let mut panel = Panel::default();
        assert_eq!(panel.message(true), Some((texts::EMPTY, None)));
        let _ = panel.update(Message::History(History::State {
            enabled: false,
            items: Vec::new(),
        }));
        assert_eq!(
            panel.message(true),
            Some((texts::OFF, Some(texts::OFF_LINE)))
        );
        let _ = panel.update(Message::History(History::Unavailable));
        assert_eq!(panel.message(true), Some((texts::UNAVAILABLE, None)));
    }

    #[test]
    fn escape_closes_with_an_animation() {
        let mut panel = Panel::default();
        let _ = panel.update(Message::Key(Key::Escape));
        assert!(panel.closing);
    }
}
