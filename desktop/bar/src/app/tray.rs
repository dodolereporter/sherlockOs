//! The notification area in the bar (lot D-45): the chevron before the indicators, and the
//! popup it opens under itself, a grid of the icons applications put there. A click activates
//! an item, the middle button runs its secondary action, the wheel scrolls it; the line under
//! the grid says what the icon under the pointer is. An icon that needs attention breathes a
//! few times, and the chevron carries a dot meanwhile. Taller than the screen allows, the
//! popup stops above its bottom and its grid scrolls (lot D-45c).

use std::time::Duration;

use iced::mouse::ScrollDelta;
use iced::widget::{
    Column, Row, Stack, column, container, image, mouse_area, opaque, row, scrollable,
};
use iced::window::Id;
use iced::{Alignment, Element, Length, Padding, Rectangle, Task, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{PixelSize, PopupAnchor, PopupGravity, Position};
use sherlock_ui::component::{self as ui};
use sherlock_ui::motion::{Animation, SCALE_PRECISION};
use sherlock_ui::surface::{self, Elevation};
use sherlock_ui::{animated, color, metrics, panels, raster, theme, tray as look, typography};

use super::{Bar, Message};
use crate::dbusmenu;
use crate::locate::locate;
use crate::tray::{Event, Icon, Item, Request};
use crate::unfold::Unfold;

/// How many breaths an icon that needs attention takes, when it starts to and each time the
/// popup opens: then it rests, the dot of the chevron still there, and no frame is drawn.
const BREATHS: u32 = 3;

/// A button of the mouse on an icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Left: `Activate`.
    Primary,
    /// Middle: `SecondaryActivate`.
    Secondary,
}

/// The notification area: its items, and the popup.
#[derive(Debug, Default)]
pub struct Tray {
    /// The items, in the order they came.
    pub items: Vec<Item>,
    /// The popup while it is shown, open or folding back.
    pub popup: Option<Id>,
    pub unfold: Unfold,
    /// The chevron's half turn: 0 pointing down, 1 up while the popup is open.
    turn: Option<Animation>,
    /// Where the chevron is drawn in the bar surface.
    pub chevron: Option<Rectangle>,
    /// The item under the pointer.
    pub hovered: Option<String>,
    /// Room around the popup in its surface, and the size of that surface.
    pub margins: Padding,
    pub size: (u32, u32),
    /// When each item that needs attention started to breathe.
    breathing: Vec<(String, Duration)>,
    /// The time of the last frame.
    now: Duration,
}

impl Tray {
    /// The items shown: not passive.
    pub fn shown(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|i| i.shown())
    }

    /// Some item needs attention: the chevron carries a dot.
    pub fn attention(&self) -> bool {
        self.shown().any(Item::needs_attention)
    }

    pub fn open(&self) -> bool {
        self.popup.is_some() && !self.unfold.folding()
    }

    /// Something moves: the popup, the chevron, an icon that breathes in the open popup.
    pub fn running(&self) -> bool {
        let turning = self
            .turn
            .is_some_and(|t| !t.is_done(self.now, SCALE_PRECISION));
        let breathing = self.popup.is_some() && self.breathing.iter().any(|b| self.breathes(b));
        turning || breathing || (self.popup.is_some() && self.unfold.running())
    }

    fn breathes(&self, (_, since): &(String, Duration)) -> bool {
        self.now.saturating_sub(*since) < look::PULSE * BREATHS
    }

    /// How much the icon of `key` has grown in its breath now.
    fn growth(&self, key: &str) -> f32 {
        let breath = self.breathing.iter().find(|b| b.0 == key);
        breath
            .filter(|b| self.breathes(b))
            .map_or(0.0, |(_, since)| {
                look::breath(self.now.saturating_sub(*since))
            })
    }

    /// The chevron's half turn now.
    fn turned(&self) -> f32 {
        self.turn.map_or(0.0, |t| t.value(self.now) as f32)
    }

    /// The chevron turns towards `open`.
    fn turn_to(&mut self, open: bool, now: Duration) {
        let target = if open { 1.0 } else { 0.0 };
        match &mut self.turn {
            Some(turn) => turn.retarget(now, target),
            None => self.turn = Some(Animation::new(look::CHEVRON_SPRING, 0.0, target, now)),
        }
    }

    /// The items that need attention breathe again from `now`; those that no longer do rest.
    fn breathe_again(&mut self, now: Duration) {
        self.breathing = self
            .shown()
            .filter(|i| i.needs_attention())
            .map(|i| (i.key.clone(), now))
            .collect();
    }

    pub fn tick(&mut self, now: Duration) {
        self.now = now;
        self.unfold.tick(now);
    }
}

impl Bar {
    pub fn tray_open(&self) -> bool {
        self.tray.open()
    }

    /// The notification area learnt something.
    pub(super) fn tray_event(&mut self, event: Event) -> Task<Message> {
        let now = animated::clock();
        match event {
            Event::Changed(item) => {
                let item = *item;
                let was = self.tray.items.iter().position(|i| i.key == item.key);
                let called = item.needs_attention()
                    && was.is_none_or(|at| !self.tray.items[at].needs_attention());
                if called {
                    self.tray.breathing.retain(|b| b.0 != item.key);
                    self.tray.breathing.push((item.key.clone(), now));
                }
                if !item.needs_attention() {
                    self.tray.breathing.retain(|b| b.0 != item.key);
                }
                match was {
                    Some(at) => self.tray.items[at] = item,
                    None => self.tray.items.push(item),
                }
            }
            Event::Gone(key) => {
                self.tray.items.retain(|i| i.key != key);
                self.tray.breathing.retain(|b| b.0 != key);
                if self.tray.hovered.as_ref() == Some(&key) {
                    self.tray.hovered = None;
                }
            }
        }
        self.tray.now = now;
        self.resize_tray()
    }

    /// Opens the popup under the chevron, or closes it when it is open. Clicked again while it
    /// folds back, it unfolds again in the same popup.
    pub(super) fn toggle_tray(&mut self) -> Task<Message> {
        let now = animated::clock();
        self.tray.now = now;
        if self.tray.open() {
            return self.close_tray();
        }
        if self.tray.popup.is_some() {
            self.tray.unfold.fold(now, true);
            self.tray.turn_to(true, now);
            self.tray.breathe_again(now);
            return Task::none();
        }
        // One popup at a time holds the grab.
        let closed = self
            .drop_panel()
            .chain(self.drop_menu())
            .chain(self.drop_app_menus());
        let Some(surface) = self.surface else {
            tracing::warn!("bar surface unknown: the notification area cannot open");
            return closed;
        };
        let gpu = surface::gpu_drawn() == Some(true);
        self.tray.margins = margins(gpu);
        let (settings, region) = self.tray_placement(surface);
        let id = Id::unique();
        self.tray.popup = Some(id);
        self.tray.unfold = if self.animates() {
            Unfold::opening(now)
        } else {
            Unfold::default()
        };
        self.tray.turn_to(true, now);
        self.tray.breathe_again(now);
        closed
            .chain(Task::done(Message::NewPopUp { settings, id }))
            .chain(input_region(id, region))
    }

    /// Where the popup goes, and its input region: under the chevron, its centre under the
    /// chevron's, within the bar.
    fn tray_placement(&mut self, surface: Id) -> (IcedNewPopupSettings, (i32, i32, i32, i32)) {
        let count = self.tray.shown().count();
        let (width, height) = (look::width(count), self.tray_height(count));
        let margins = self.tray.margins;
        let size = (
            (width + margins.left + margins.right).ceil() as u32,
            (height + margins.top + margins.bottom).ceil() as u32,
        );
        self.tray.size = size;
        let side = self.applied.margin_side as f32;
        let bar = self.bar_width();
        let centre = self
            .tray
            .chevron
            .map_or(bar - width / 2.0, |c| c.center_x());
        // The popup ends at the latest where the panels do, under the end of the floating bar.
        let right = bar - (metrics::TOP_BAR_MARGIN_SIDE - side).max(0.0);
        let left = (centre - width / 2.0).min(right - width).max(0.0);
        let top = metrics::TOP_BAR_HEIGHT + look::BELOW_BAR - margins.top;
        let middle = (left - margins.left).round() as i32 + (size.0 / 2) as i32;
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(size.0, size.1),
            Position::new(middle - 1, 0),
            PixelSize::px(2, top.max(1.0) as u32),
        )
        .anchor(PopupAnchor::Bottom)
        .gravity(PopupGravity::Bottom);
        let region = (
            margins.left as i32,
            margins.top as i32,
            width.ceil() as i32,
            height.ceil() as i32,
        );
        (settings, region)
    }

    /// Height of the popup for `count` icons: as tall as its rows, within the screen, and at
    /// least a row and the tooltip line. Beyond, the grid scrolls.
    pub fn tray_height(&self, count: usize) -> f32 {
        let screen = self.screen_height().unwrap_or(metrics::REFERENCE_SCREEN.1);
        let top = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT + look::BELOW_BAR;
        let room = screen - top - super::global::SCREEN_ROOM;
        look::height(count).min(room).max(look::height(1))
    }

    /// The left of the popup in the bar surface, as `tray_placement` puts it: for the tests and
    /// the captures.
    pub fn tray_left(&self) -> f32 {
        let count = self.tray.shown().count();
        let width = look::width(count);
        let side = self.applied.margin_side as f32;
        let bar = self.bar_width();
        let centre = self
            .tray
            .chevron
            .map_or(bar - width / 2.0, |c| c.center_x());
        let right = bar - (metrics::TOP_BAR_MARGIN_SIDE - side).max(0.0);
        (centre - width / 2.0).min(right - width).max(0.0)
    }

    /// The icons shown changed while the popup is open: it opens again at its new size, at
    /// once, its grab kept by the new popup.
    fn resize_tray(&mut self) -> Task<Message> {
        let (Some(old), Some(surface)) = (self.tray.popup, self.surface) else {
            return Task::none();
        };
        if self.tray.unfold.folding() {
            return Task::none();
        }
        let count = self.tray.shown().count();
        let margins = self.tray.margins;
        let height = (self.tray_height(count) + margins.top + margins.bottom).ceil() as u32;
        let width = (look::width(count) + margins.left + margins.right).ceil() as u32;
        if (width, height) == self.tray.size {
            return Task::none();
        }
        let (settings, region) = self.tray_placement(surface);
        let id = Id::unique();
        self.tray.popup = Some(id);
        window::close(old)
            .chain(Task::done(Message::NewPopUp { settings, id }))
            .chain(input_region(id, region))
    }

    /// Closes the popup: it folds back first where it unfolds, then goes.
    pub(super) fn close_tray(&mut self) -> Task<Message> {
        let now = animated::clock();
        self.tray.now = now;
        self.tray.hovered = None;
        self.tray.turn_to(false, now);
        if self.tray.popup.is_some() && self.animates() {
            self.tray.unfold.fold(now, false);
            return Task::none();
        }
        self.drop_tray()
    }

    /// Closes the popup at once.
    pub(super) fn drop_tray(&mut self) -> Task<Message> {
        self.tray.unfold = Unfold::default();
        self.tray.hovered = None;
        match self.tray.popup.take() {
            Some(id) => {
                self.tray.turn_to(false, animated::clock());
                window::close(id)
            }
            None => Task::none(),
        }
    }

    /// The compositor closed the popup `id` (a click elsewhere).
    pub(super) fn tray_closed(&mut self, id: Id) {
        if self.tray.popup == Some(id) {
            self.tray.popup = None;
            self.tray.unfold = Unfold::default();
            self.tray.hovered = None;
            self.tray.turn_to(false, animated::clock());
        }
    }

    /// Next frame; the popup goes once it has faded out.
    pub(super) fn tick_tray(&mut self, at: Duration) -> Task<Message> {
        self.tray.tick(at);
        if self.tray.popup.is_some() && self.tray.unfold.folded() {
            return self.drop_tray();
        }
        Task::none()
    }

    /// A click on the icon of `key`. The popup closes after a click that activates.
    pub(super) fn tray_click(&mut self, key: &str, click: Click) -> Task<Message> {
        let Some(sent) = self.tray_request(key, click) else {
            return Task::none();
        };
        tracing::debug!(key, ?click, "tray item clicked");
        self.close_tray().chain(send(sent))
    }

    /// What a click on the icon of `key` asks of its application, and where.
    pub fn tray_request(&self, key: &str, click: Click) -> Option<(String, String, Request)> {
        let item = self.tray.items.iter().find(|i| i.key == key)?;
        let (x, y) = self.tray_point();
        let request = match click {
            Click::Primary => Request::Activate(x, y),
            Click::Secondary => Request::SecondaryActivate(x, y),
        };
        Some((item.service.clone(), item.path.clone(), request))
    }

    /// The wheel over the icon of `key`: `Scroll`.
    pub(super) fn tray_scroll(&self, key: &str, delta: ScrollDelta) -> Task<Message> {
        let item = self.tray.items.iter().find(|i| i.key == key);
        let request = item.zip(Request::scroll(delta));
        request.map_or_else(Task::none, |(item, request)| {
            send((item.service.clone(), item.path.clone(), request))
        })
    }

    /// Where the icons are on the screen, for the application: under the chevron.
    fn tray_point(&self) -> (i32, i32) {
        let side = self.applied.margin_side;
        let x = self.tray.chevron.map_or(0.0, |c| c.center_x()) as i32 + side;
        let y = self.applied.margin_top + metrics::TOP_BAR_HEIGHT as i32;
        (x, y)
    }

    /// The chevron in the bar, before the indicators; none without an item to show.
    pub(super) fn view_chevron(&self) -> Option<Element<'static, Message>> {
        self.tray.shown().next()?;
        let glyph = look::chevron(self.tray.turned(), self.tray.attention());
        let mut button = panels::bar_button(glyph, self.tray.open(), Some(Message::ToggleTray));
        if self.focused() == Some(super::Target::Tray) {
            button = button.focus_ring();
        }
        Some(locate(button, Message::TrayChevron))
    }

    /// The popup: the grid of icons, and the line of the one under the pointer.
    pub fn view_tray(&self) -> Element<'_, Message> {
        let count = self.tray.shown().count();
        let (width, height) = (look::width(count), self.tray_height(count));
        let panel = color::with_opacity(self.tray.unfold.opacity(), || {
            let cells: Vec<Element<'_, Message>> =
                self.tray.shown().map(|item| self.tray_cell(item)).collect();
            let grid: Element<'_, Message> = if cells.is_empty() {
                let empty = ui::body(look::text::EMPTY).color(color::palette().text_tertiary);
                container(empty)
                    .center_x(Length::Fill)
                    .height(look::CELL)
                    .center_y(look::CELL)
                    .into()
            } else {
                let mut rows = Column::new().spacing(look::GAP);
                let mut cells = cells.into_iter().peekable();
                while cells.peek().is_some() {
                    let line: Vec<_> = cells.by_ref().take(look::COLUMNS).collect();
                    rows = rows.push(Row::with_children(line).spacing(look::GAP));
                }
                // Within the screen, the rows that do not fit scroll above the tooltip line.
                scrollable(rows).height(Length::Fill).into()
            };
            let glass = container(column![grid, self.tray_tip()])
                .width(width)
                .height(height)
                .padding(look::PADDING + metrics::BORDER_WIDTH)
                .style(theme::menu);
            opaque(surface::shadowed(glass, look::RADIUS, Elevation::Menu))
        });
        // It unfolds from the middle of its top edge, under the chevron.
        let panel = self.tray.unfold.panel(panel, (0.5, 0.0));
        mouse_area(container(panel).padding(self.tray.margins))
            .on_press(Message::ToggleTray)
            .into()
    }

    /// The cell of `item`: its icon, its overlay, its breath; the buttons and the wheel.
    fn tray_cell<'a>(&'a self, item: &'a Item) -> Element<'a, Message> {
        let side = look::POPUP_ICON;
        let mut icon = icon_view(item.drawn(), side, &item.properties.label().0);
        if let Some(overlay) = &item.overlay {
            let small = side * look::OVERLAY_SHARE;
            let badge = container(icon_view(Some(overlay), small, ""))
                .align_right(side)
                .align_bottom(side);
            icon = Stack::with_children([icon, badge.into()]).into();
        }
        let growth = self.tray.growth(&item.key);
        if growth > 0.0 {
            icon = animated::reveal(icon)
                .scale(1.0 + growth, (0.5, 0.5))
                .into();
        }
        let key = item.key.as_str();
        let press = Message::TrayClick(key.to_owned(), Click::Primary);
        let cell = look::cell(icon, Some(press));
        let scroll_key = key.to_owned();
        mouse_area(cell)
            .on_middle_press(Message::TrayClick(key.to_owned(), Click::Secondary))
            .on_scroll(move |delta| Message::TrayScroll(scroll_key.clone(), delta))
            .on_enter(Message::TrayHover(Some(key.to_owned())))
            .on_exit(Message::TrayLeave(key.to_owned()))
            .into()
    }

    /// The line under the grid: the title of the icon under the pointer, and its text.
    fn tray_tip(&self) -> Element<'_, Message> {
        let p = color::palette();
        let hovered = self.tray.hovered.as_ref();
        let item = hovered.and_then(|key| self.tray.shown().find(|i| &i.key == key));
        let line: Element<'_, Message> = match item.map(|i| i.properties.label()) {
            Some((title, text)) => {
                let mut line = row![
                    ui::ellipsis(title)
                        .font(typography::UI_SEMIBOLD)
                        .color(p.text_primary)
                ]
                .spacing(metrics::GAP_INLINE)
                .align_y(Alignment::Center);
                if !text.is_empty() {
                    line = line.push(
                        ui::ellipsis(text)
                            .width(Length::Fill)
                            .color(p.text_secondary),
                    );
                }
                line.into()
            }
            None => iced::widget::space().into(),
        };
        container(line)
            .height(look::TIP_HEIGHT)
            .center_y(look::TIP_HEIGHT)
            .padding(Padding {
                left: 4.0,
                right: 4.0,
                ..Padding::ZERO
            })
            .into()
    }
}

/// An icon, `side` wide, sharp at any scale: a drawing rasterized by `sherlock_ui::raster`, a
/// symbolic one in the color of the text, or the application's pixels. Without one, the first
/// letter of `name`.
pub fn icon_view<'a>(icon: Option<&Icon>, side: f32, name: &str) -> Element<'a, Message> {
    let opacity = color::opacity();
    let drawn = match icon {
        Some(Icon::Drawing {
            handle,
            symbolic: false,
        }) => raster::raster(handle, side),
        Some(Icon::Drawing {
            handle,
            symbolic: true,
        }) => raster::tinted(handle, side, color::palette().text_primary),
        Some(Icon::Pixels(pixels)) => pixels.clone(),
        None => {
            let letter = name.chars().next().map_or('?', |c| c.to_ascii_uppercase());
            let letter = ui::body(letter.to_string())
                .font(typography::UI_SEMIBOLD)
                .color(color::palette().text_secondary);
            return container(letter).center(side).into();
        }
    };
    image(drawn)
        .width(side)
        .height(side)
        .filter_method(image::FilterMethod::Linear)
        .opacity(opacity)
        .into()
}

/// Sends `request` to the item at `path` of `service`, on the connection of the menus.
fn send((service, path, request): (String, String, Request)) -> Task<Message> {
    Task::perform(
        async move {
            let bus = dbusmenu::bus().await?;
            request.send(&bus, &service, &path).await
        },
        |sent| Message::TraySent(sent.map_err(|e| e.to_string())),
    )
}

/// Room around the popup: the gap below the bar, and the menus' shadow on the GPU.
fn margins(gpu: bool) -> Padding {
    if gpu {
        return Elevation::Menu.margins();
    }
    Padding {
        top: look::BELOW_BAR,
        ..Padding::ZERO
    }
}

/// Only the glass takes the pointer: the room for its shadow lets clicks through to the bar.
fn input_region(id: Id, (x, y, width, height): (i32, i32, i32, i32)) -> Task<Message> {
    let region = RegionActionCB::new(move |region| {
        if let Err(error) = region.add(x, y, width, height) {
            tracing::warn!(%error, "cannot limit the input region of the notification area");
        }
    });
    Task::done(Message::SetInputRegion {
        id,
        callback: region,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Target;
    use crate::menu;
    use crate::tray::{Properties, Status};

    fn item(n: usize, status: Status) -> Item {
        Item {
            key: format!(":1.{n}/StatusNotifierItem"),
            service: format!(":1.{n}"),
            path: String::from(crate::tray::ITEM_PATH),
            properties: Properties {
                id: format!("item-{n}"),
                status,
                ..Properties::default()
            },
            icon: None,
            attention: None,
            overlay: None,
        }
    }

    fn with_items(items: &[Item]) -> Bar {
        let mut bar = Bar {
            panel_motion: Some(true),
            ..Bar::default()
        };
        let surface = Id::unique();
        let _ = bar.update(Message::Surface(surface));
        let _ = bar.update(Message::Resized(surface, 1896.0));
        for item in items {
            let _ = bar.update(Message::Tray(Event::Changed(Box::new(item.clone()))));
        }
        bar
    }

    fn frame_in(bar: &mut Bar, later: Duration) -> Task<Message> {
        bar.update(Message::Frame(std::time::Instant::now() + later))
    }

    #[test]
    fn the_chevron_shows_only_with_an_icon_to_show() {
        let mut bar = with_items(&[item(1, Status::Passive)]);
        assert!(bar.view_chevron().is_none());
        assert!(!bar.targets().contains(&Target::Tray));
        let _ = bar.update(Message::Tray(Event::Changed(Box::new(item(
            2,
            Status::Active,
        )))));
        assert!(bar.view_chevron().is_some());
        assert_eq!(bar.tray.shown().count(), 1);
        // Enter on the ringed chevron opens the popup.
        let at = bar
            .targets()
            .iter()
            .position(|t| *t == Target::Tray)
            .unwrap();
        for _ in 0..=at {
            let _ = bar.update(Message::Focus(sherlock_ui::focus::Direction::Next));
        }
        assert_eq!(bar.focused(), Some(Target::Tray));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert!(bar.tray_open());
    }

    #[test]
    fn the_popup_unfolds_folds_back_and_closes_with_escape_or_a_click_elsewhere() {
        let second = Duration::from_secs(2);
        let mut bar = with_items(&[item(1, Status::Active)]);
        let _ = bar.update(Message::ToggleTray);
        let popup = bar.tray.popup;
        assert!(bar.tray_open() && bar.animating());
        let _ = frame_in(&mut bar, second);
        assert!(!bar.animating(), "no frame at rest");
        // Escape folds it back, then it goes, once.
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert!(!bar.tray_open() && bar.tray.popup.is_some());
        assert!(frame_in(&mut bar, second).units() > 0);
        assert_eq!(bar.tray.popup, None);
        assert_eq!(frame_in(&mut bar, second).units(), 0);
        // Clicked again while it folds: it unfolds again in the same popup.
        let _ = bar.update(Message::ToggleTray);
        let popup_again = bar.tray.popup;
        assert_ne!(popup_again, popup);
        let _ = bar.update(Message::ToggleTray);
        let _ = bar.update(Message::ToggleTray);
        assert_eq!(bar.tray.popup, popup_again);
        assert!(bar.tray_open());
        // The compositor closes it on a click elsewhere.
        let _ = bar.update(Message::Closed(popup_again.unwrap()));
        assert_eq!(bar.tray.popup, None);
        // A panel opening takes it away at once: one grab at a time.
        let _ = bar.update(Message::ToggleTray);
        let _ = bar.update(Message::TogglePanel(crate::panel::Panel::Control));
        assert_eq!(bar.tray.popup, None);
    }

    #[test]
    fn the_popup_follows_the_icons_that_come_and_go() {
        let items: Vec<Item> = (1..=5).map(|n| item(n, Status::Active)).collect();
        let mut bar = with_items(&items);
        let _ = bar.update(Message::ToggleTray);
        let (popup, size) = (bar.tray.popup, bar.tray.size);
        // A sixth icon starts a second row: the popup opens again, taller.
        let _ = bar.update(Message::Tray(Event::Changed(Box::new(item(
            6,
            Status::Active,
        )))));
        assert_ne!(bar.tray.popup, popup);
        assert!(bar.tray.size.1 > size.1);
        // A change of the same size keeps the popup.
        let popup = bar.tray.popup;
        let _ = bar.update(Message::Tray(Event::Changed(Box::new(item(
            6,
            Status::Active,
        )))));
        assert_eq!(bar.tray.popup, popup);
        let _ = bar.update(Message::Tray(Event::Gone(item(6, Status::Active).key)));
        assert_eq!(bar.tray.size, size);
    }

    #[test]
    fn on_a_short_screen_the_popup_stops_above_its_bottom_and_its_grid_scrolls() {
        let items: Vec<Item> = (1..=crate::tray::MAX_ITEMS)
            .map(|n| item(n, Status::Active))
            .collect();
        let mut bar = with_items(&items);
        let whole = look::height(items.len());
        // Unknown screen: the reference one, tall enough for every row.
        assert_eq!(bar.tray_height(items.len()), whole);
        // A screen 400 high (800 at scale 2): the popup ends 24 above its bottom.
        let surface = bar.surface.unwrap();
        let _ = bar.update(Message::Output(
            surface,
            String::from("eDP-1"),
            Some((1280.0, 400.0)),
        ));
        let height = bar.tray_height(items.len());
        let top = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT + look::BELOW_BAR;
        assert!(
            height < whole && (top + height + 24.0 - 400.0).abs() < 0.5,
            "{height}"
        );
        let _ = bar.update(Message::ToggleTray);
        let margins = bar.tray.margins;
        let expected = (height + margins.top + margins.bottom).ceil() as u32;
        assert_eq!(bar.tray.size.1, expected);
        // A screen too small for anything: still a row and the tooltip line.
        let _ = bar.update(Message::Output(
            surface,
            String::from("eDP-1"),
            Some((320.0, 60.0)),
        ));
        assert_eq!(bar.tray_height(items.len()), look::height(1));
        // Few icons on a short screen: as tall as their rows.
        let _ = bar.update(Message::Output(
            surface,
            String::from("eDP-1"),
            Some((1280.0, 400.0)),
        ));
        assert_eq!(bar.tray_height(3), look::height(3));
    }

    #[test]
    fn an_icon_that_needs_attention_breathes_a_few_times_then_rests() {
        let mut bar = with_items(&[item(1, Status::Active)]);
        let _ = bar.update(Message::ToggleTray);
        let _ = frame_in(&mut bar, Duration::from_secs(2));
        assert!(!bar.animating());
        let _ = bar.update(Message::Tray(Event::Changed(Box::new(item(
            1,
            Status::NeedsAttention,
        )))));
        assert!(bar.tray.attention());
        let key = item(1, Status::Active).key;
        let _ = frame_in(&mut bar, look::PULSE / 2);
        assert!(bar.animating(), "it breathes");
        assert!(bar.tray.growth(&key) > 0.0);
        let _ = frame_in(&mut bar, look::PULSE * (BREATHS + 1));
        assert!(!bar.animating(), "then rests");
        assert_eq!(bar.tray.growth(&key), 0.0);
        assert!(bar.tray.attention(), "the dot stays");
    }
}
