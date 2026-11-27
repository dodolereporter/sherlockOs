//! Icons of the notification area kept in the bar (lot D-45a): an icon dragged from the popup
//! onto the bar stays there, before the chevron; dragged out of the bar, it goes back into the
//! popup; dragged along the bar, it changes place. The choice and the order go to the bar's
//! settings file (`tray::pins`), read again just before it is written.
//!
//! During a drag the pointer stays with the surface it was pressed in (Wayland's implicit
//! grab): the popup keeps getting the moves when the pointer is over the bar, at coordinates
//! outside its own surface, and the bar when the pointer leaves it. Their places in the bar
//! surface are known, so each move is placed there.

use std::sync::atomic::{AtomicBool, Ordering};

use iced::mouse::{self, Button};
use iced::widget::{Row, mouse_area};
use iced::window::Id;
use iced::{Alignment, Element, Point, Task};
use sherlock_ui::{animated, color, metrics, panels, tray as look};

use super::tray::{Click, icon_view};
use super::{Bar, Message};
use crate::locate::locate;
use crate::tray::Item;
use crate::tray::pins::{self, Change};

/// Width of an icon kept in the bar: a ghost button around it, as the indicators.
pub const SLOT: f32 = look::ICON + 2.0 * panels::BAR_BUTTON_PADDING;
/// Opacity of an icon being dragged where it was, or of one about to leave the bar.
pub const DRAGGED: f32 = 0.35;
/// Opacity of the icon shown where a drop would put it.
const PREVIEW: f32 = 0.6;
/// How far the pointer goes, pressed, before a press becomes a drag.
const THRESHOLD: f32 = 6.0;

/// Whether the moves of the pointer are wanted: only while an icon is pressed, so that a
/// pointer moving over the bar sends no message the rest of the time. Set on the interface
/// thread, read by `on_pointer`, which iced calls with no state of its own.
static FOLLOW: AtomicBool = AtomicBool::new(false);

/// Where a dragged icon comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Popup,
    Bar,
}

/// An icon pressed, and dragged once the pointer moved far enough.
#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub key: String,
    pub from: Place,
    /// The first and the last place of the pointer, in the surface it was pressed in.
    pub start: Option<Point>,
    pub at: Option<Point>,
    pub moving: bool,
    /// Let go: kept until the next press, so that the release is not taken for a click.
    pub done: bool,
}

/// The buttons and, while an icon is pressed, the moves of the pointer over the bar's surfaces.
pub fn on_pointer(event: iced::Event, _: iced::event::Status, id: Id) -> Option<Message> {
    let iced::Event::Mouse(event) = event else {
        return None;
    };
    let wanted = match event {
        mouse::Event::ButtonPressed(Button::Left) | mouse::Event::ButtonReleased(Button::Left) => {
            true
        }
        mouse::Event::CursorMoved { .. } => FOLLOW.load(Ordering::Relaxed),
        _ => false,
    };
    wanted.then_some(Message::TrayPointer(id, event))
}

/// Reads the icons kept in the bar, off the interface thread.
pub fn load() -> Task<Message> {
    Task::perform(
        async {
            let Some(path) = pins::path() else {
                return Vec::new();
            };
            pins::load(&path).unwrap_or_else(|error| {
                tracing::warn!(%error, "cannot read the icons kept in the bar");
                Vec::new()
            })
        },
        Message::Pins,
    )
}

impl Bar {
    /// A button or a move of the pointer over the surface `id`.
    pub(super) fn tray_pointer(&mut self, id: Id, event: mouse::Event) -> Task<Message> {
        let place = if self.tray.popup == Some(id) {
            Place::Popup
        } else if self.surface == Some(id) {
            Place::Bar
        } else {
            return Task::none();
        };
        match event {
            mouse::Event::ButtonPressed(_) => {
                self.tray.drag = self.pressed(place);
                FOLLOW.store(self.tray.drag.is_some(), Ordering::Relaxed);
            }
            mouse::Event::CursorMoved { position } => {
                if let Some(drag) = &mut self.tray.drag
                    && drag.from == place
                    && !drag.done
                {
                    let start = *drag.start.get_or_insert(position);
                    drag.at = Some(position);
                    drag.moving |= start.distance(position) > THRESHOLD;
                }
            }
            mouse::Event::ButtonReleased(_) => {
                FOLLOW.store(false, Ordering::Relaxed);
                let dropped = self.tray.drag.as_mut().filter(|d| d.moving && !d.done);
                let Some(drag) = dropped else {
                    self.tray.drag = None;
                    return Task::none();
                };
                drag.done = true;
                let drag = drag.clone();
                if let Some(change) = self.dropped(&drag) {
                    return self.change_pins(change);
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// The icon under the pointer when a button is pressed in `place`, if one can be dragged.
    fn pressed(&self, place: Place) -> Option<Drag> {
        let key = self.tray.hovered.clone()?;
        let found = match place {
            Place::Popup => self.tray.in_popup().any(|i| i.key == key),
            Place::Bar => self.tray.pinned().any(|i| i.key == key),
        };
        found.then_some(Drag {
            key,
            from: place,
            start: None,
            at: None,
            moving: false,
            done: false,
        })
    }

    /// Where the pointer of `drag` is in the bar surface.
    fn in_bar(&self, drag: &Drag) -> Option<Point> {
        let at = drag.at?;
        Some(match drag.from {
            Place::Bar => at,
            Place::Popup => {
                let margins = self.tray.margins;
                Point::new(
                    self.tray_left() - margins.left + at.x,
                    metrics::TOP_BAR_HEIGHT + look::BELOW_BAR - margins.top + at.y,
                )
            }
        })
    }

    /// Where an icon dropped at `x` of the bar goes among those kept there.
    fn slot_at(&self, x: f32) -> usize {
        let count = self.tray.pinned().count();
        let Some(row) = self.tray.pinned_row.filter(|_| count > 0) else {
            return 0;
        };
        let index = ((x - row.x) / SLOT).round().max(0.0) as usize;
        index.min(count)
    }

    /// What dropping `drag` changes: an icon of the popup let go over the bar is kept there, an
    /// icon of the bar let go below it goes back, or changes place along it.
    pub fn dropped(&self, drag: &Drag) -> Option<Change> {
        let at = self.in_bar(drag)?;
        let item = self.tray.shown().find(|i| i.key == drag.key)?;
        let id = item.properties.id.clone();
        if id.is_empty() {
            return None;
        }
        let over_bar = at.y < metrics::TOP_BAR_HEIGHT;
        match drag.from {
            Place::Popup if over_bar => Some(Change::Pin {
                id,
                index: self.slot_at(at.x),
            }),
            Place::Popup => None,
            Place::Bar if at.y > metrics::TOP_BAR_HEIGHT + look::BELOW_BAR => {
                Some(Change::Unpin(id))
            }
            Place::Bar => Some(Change::Pin {
                id,
                index: self.slot_at(at.x),
            }),
        }
    }

    /// Applies `change` at once, then to the settings file, read just before it is written.
    fn change_pins(&mut self, change: Change) -> Task<Message> {
        change.apply(&mut self.tray.pins);
        tracing::debug!(?change, "icons kept in the bar");
        let resized = self.resize_tray();
        let saved = Task::perform(
            async move {
                let path = pins::path().ok_or_else(|| String::from("no configuration folder"))?;
                pins::change(&path, &change).map_err(|e| e.to_string())
            },
            |saved| match saved {
                Ok(pins) => Message::Pins(pins),
                Err(error) => Message::PinsNotSaved(error),
            },
        );
        resized.chain(saved)
    }

    /// The icon of popup being dragged over the bar, and where it would land: shown there.
    fn preview(&self) -> Option<(&Item, usize)> {
        let drag = self.tray.drag.as_ref()?;
        if drag.from != Place::Popup || !drag.moving || drag.done {
            return None;
        }
        let at = self.in_bar(drag)?;
        if at.y >= metrics::TOP_BAR_HEIGHT {
            return None;
        }
        let item = self.tray.in_popup().find(|i| i.key == drag.key)?;
        Some((item, self.slot_at(at.x)))
    }

    /// The icons kept in the bar, before the chevron; none when there is none, nor one being
    /// dragged over the bar.
    pub(super) fn view_pinned(&self) -> Option<Element<'static, Message>> {
        let preview = self.preview();
        let pinned: Vec<&Item> = self.tray.pinned().collect();
        if pinned.is_empty() && preview.is_none() {
            return None;
        }
        let mut row = Row::new().align_y(Alignment::Center);
        for (index, item) in pinned.iter().enumerate() {
            if let Some((shown, _)) = preview.filter(|(_, at)| *at == index) {
                row = row.push(placeholder(shown));
            }
            row = row.push(self.pinned_button(item));
        }
        if let Some((shown, _)) = preview.filter(|(_, at)| *at == pinned.len()) {
            row = row.push(placeholder(shown));
        }
        Some(locate(row, Message::TrayPinnedRow))
    }

    /// An icon kept in the bar: the clicks and the wheel of the popup's, its breath; dragged, it
    /// follows the pointer along the bar, and fades once it would leave it.
    fn pinned_button(&self, item: &Item) -> Element<'static, Message> {
        let key = item.key.clone();
        let mut icon = icon_view(item.drawn(), look::ICON, &item.properties.label().0);
        let growth = self.tray.growth(&key);
        if growth > 0.0 {
            icon = animated::reveal(icon)
                .scale(1.0 + growth, (0.5, 0.5))
                .into();
        }
        let dragged = self
            .tray
            .drag
            .as_ref()
            .filter(|d| d.key == key && d.from == Place::Bar && d.moving && !d.done);
        if let Some((Some(start), Some(at))) = dragged.map(|d| (d.start, d.at)) {
            let leaving = at.y > metrics::TOP_BAR_HEIGHT + look::BELOW_BAR;
            let opacity = if leaving { DRAGGED } else { 1.0 };
            icon = color::with_opacity(opacity, || {
                animated::reveal(icon)
                    .opacity(opacity)
                    .offset(at.x - start.x, 0.0)
            })
            .into();
        }
        let press = Message::TrayClick(key.clone(), Click::Primary);
        let button = panels::bar_button(icon, false, Some(press));
        let scroll = key.clone();
        mouse_area(button)
            .on_middle_press(Message::TrayClick(key.clone(), Click::Secondary))
            .on_right_press(Message::TrayClick(key.clone(), Click::Context))
            .on_scroll(move |delta| Message::TrayScroll(scroll.clone(), delta))
            .on_enter(Message::TrayHover(Some(key.clone())))
            .on_exit(Message::TrayLeave(key))
            .into()
    }
}

/// Where a dropped icon would land: the icon itself, faded, in a slot of its own.
fn placeholder(item: &Item) -> Element<'static, Message> {
    let icon = color::with_opacity(PREVIEW, || {
        icon_view(item.drawn(), look::ICON, &item.properties.label().0)
    });
    let icon = animated::reveal(icon).opacity(PREVIEW);
    iced::widget::container(icon)
        .center_x(SLOT)
        .center_y(panels::BAR_BUTTON_HEIGHT)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dbusmenu;
    use crate::global_menu::Anchor;
    use crate::menu;
    use crate::source::workspaces::AppMenu;
    use crate::tray::{Event, Properties, Status};

    fn item(n: usize) -> Item {
        Item {
            key: format!(":1.{n}/StatusNotifierItem"),
            service: format!(":1.{n}"),
            path: String::from(crate::tray::ITEM_PATH),
            properties: Properties {
                id: format!("item-{n}"),
                status: Status::Active,
                menu: Some(String::from("/MenuBar")),
                ..Properties::default()
            },
            icon: None,
            attention: None,
            overlay: None,
        }
    }

    /// The bar of a 1920 screen with three icons, the popup open under its chevron.
    fn open() -> Bar {
        let mut bar = Bar::default();
        let surface = Id::unique();
        let _ = bar.update(Message::Surface(surface));
        let _ = bar.update(Message::Resized(surface, 1896.0));
        for n in 1..=3 {
            let _ = bar.update(Message::Tray(Event::Changed(Box::new(item(n)))));
        }
        let chevron = iced::Rectangle::new(Point::new(1500.0, 3.0), iced::Size::new(32.0, 30.0));
        let _ = bar.update(Message::TrayChevron(chevron));
        let _ = bar.update(Message::ToggleTray);
        assert!(bar.tray_open());
        bar
    }

    fn pointer(bar: &mut Bar, id: Id, event: mouse::Event) -> Task<Message> {
        bar.update(Message::TrayPointer(id, event))
    }

    fn moved(bar: &mut Bar, id: Id, x: f32, y: f32) {
        let position = Point::new(x, y);
        let _ = pointer(bar, id, mouse::Event::CursorMoved { position });
    }

    #[test]
    fn an_icon_dragged_from_the_popup_onto_the_bar_stays_there() {
        let mut bar = open();
        let popup = bar.tray.popup.unwrap();
        let key = item(2).key;
        let _ = bar.update(Message::TrayHover(Some(key.clone())));
        let _ = pointer(&mut bar, popup, mouse::Event::ButtonPressed(Button::Left));
        moved(&mut bar, popup, 60.0, 30.0);
        assert!(bar.preview().is_none(), "not yet a drag");
        moved(&mut bar, popup, 60.0, 0.0);
        assert!(bar.preview().is_none(), "still over the popup");
        // Over the bar: the icon shows where it would land.
        moved(&mut bar, popup, 60.0, -20.0);
        assert_eq!(
            bar.preview().map(|(i, at)| (i.key.clone(), at)),
            Some((key.clone(), 0))
        );
        assert!(bar.view_pinned().is_some());
        let saved = pointer(&mut bar, popup, mouse::Event::ButtonReleased(Button::Left));
        assert!(saved.units() > 0, "written to the settings file");
        assert_eq!(bar.tray.pins, ["item-2"]);
        // It left the popup for the bar.
        assert_eq!(bar.tray.in_popup().count(), 2);
        assert_eq!(bar.tray.pinned().count(), 1);
        // The release over its cell is not a click.
        assert_eq!(
            bar.update(Message::TrayClick(key, Click::Primary)).units(),
            0
        );
        assert!(bar.tray_open());
        // The file's answer replaces the list.
        let _ = bar.update(Message::Pins(vec![
            String::from("item-2"),
            String::from("item-3"),
        ]));
        assert_eq!(bar.tray.in_popup().count(), 1);
    }

    #[test]
    fn an_icon_of_the_bar_changes_place_or_goes_back_into_the_popup() {
        let mut bar = open();
        let _ = bar.update(Message::Pins(vec![
            String::from("item-1"),
            String::from("item-2"),
        ]));
        let row = iced::Rectangle::new(Point::new(1400.0, 3.0), iced::Size::new(2.0 * SLOT, 30.0));
        let _ = bar.update(Message::TrayPinnedRow(row));
        let surface = bar.surface.unwrap();
        let drag = |bar: &mut Bar, key: String, to: Point| {
            let _ = bar.update(Message::TrayHover(Some(key)));
            let _ = pointer(bar, surface, mouse::Event::ButtonPressed(Button::Left));
            moved(bar, surface, 1416.0, 18.0);
            moved(bar, surface, to.x, to.y);
            pointer(bar, surface, mouse::Event::ButtonReleased(Button::Left))
        };
        // « item-1 » dragged past « item-2 »: they swap.
        let _ = drag(&mut bar, item(1).key, Point::new(1400.0 + 2.0 * SLOT, 18.0));
        assert_eq!(bar.tray.pins, ["item-2", "item-1"]);
        // Dragged below the bar: back into the popup.
        let _ = drag(&mut bar, item(2).key, Point::new(1420.0, 80.0));
        assert_eq!(bar.tray.pins, ["item-1"]);
        assert_eq!(bar.tray.in_popup().count(), 2);
        // A press without a move is a click, left to the button.
        let _ = bar.update(Message::TrayHover(Some(item(1).key)));
        let _ = pointer(&mut bar, surface, mouse::Event::ButtonPressed(Button::Left));
        let released = pointer(
            &mut bar,
            surface,
            mouse::Event::ButtonReleased(Button::Left),
        );
        assert_eq!(released.units(), 0);
        assert_eq!(bar.tray.drag, None);
    }

    #[test]
    fn a_right_click_opens_the_icons_menu_under_it() {
        let mut bar = open();
        let popup = bar.tray.popup.unwrap();
        let key = item(2).key;
        let _ = bar.update(Message::TrayClick(key, Click::Context));
        let shown = bar
            .global
            .item
            .as_ref()
            .map(|m| (m.address.clone(), m.anchor));
        let address = AppMenu {
            service: String::from(":1.2"),
            path: String::from("/MenuBar"),
        };
        let Some((menu, Anchor::Popup(parent, x, _))) = shown else {
            panic!("an icon's menu from the popup: {shown:?}");
        };
        assert_eq!((menu, parent), (address.clone(), popup));
        // Under the second cell.
        let inset = look::PADDING + metrics::BORDER_WIDTH;
        assert_eq!(x, bar.tray.margins.left + inset + look::CELL + look::GAP);
        // It opens once read, in the style of our menus; the bar's menus stay closed.
        let entry = |id: i32, label: &str| dbusmenu::Item {
            id,
            label: label.to_owned(),
            ..dbusmenu::Item::default()
        };
        let root = dbusmenu::Item {
            children: vec![entry(1, "Ouvrir"), entry(2, "Quitter")],
            ..entry(0, "")
        };
        let _ = bar.update(Message::AppMenuRead(address.clone(), 0, Ok(root)));
        assert!(bar.app_menu_open());
        assert_eq!(bar.global.open_top(), None);
        assert_eq!(bar.picked(0, 1), Some((address, 2)));
        // Escape closes the menu first, then the popup.
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        let _ = bar.update(Message::Frame(
            std::time::Instant::now() + std::time::Duration::from_secs(2),
        ));
        assert!(!bar.app_menu_open() && bar.tray_open());
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert!(!bar.tray_open());
        // An item that only has a menu opens it on a left click too.
        let mut bar = open();
        let mut only_menu = item(3);
        only_menu.properties.item_is_menu = true;
        let _ = bar.update(Message::Tray(Event::Changed(Box::new(only_menu))));
        let _ = bar.update(Message::TrayClick(item(3).key, Click::Primary));
        assert!(bar.global.item.is_some());
    }
}
