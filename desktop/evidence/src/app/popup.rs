//! The right-click menu and its submenu as real `xdg_popup`s on Wayland (ADR 0029, lot E-02b):
//! they leave the window, and the compositor flips and slides them at the screen's edges. Under
//! winit, the view still draws them inside the window.
//!
//! Each popup is larger than its menu by the room of the menu's shadow. Its anchor rectangle is
//! chosen so that the menu's corner lands on the clicked point both ways: on the right and
//! below, or, flipped by the compositor, on the left and above.

use iced::widget::container;
use iced::{Element, Length, Padding, Point, Task, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{PixelSize, PopupAnchor, PopupGravity, Position};
use sherlock_ui::metrics;
use sherlock_ui::surface::Elevation;

use super::{App, MenuItem, Message, Sub};

/// The popups open now: the menu's, and its submenu's with the submenu it shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Popups {
    /// The menu's popup and where its menu was opened, to move it when it opens elsewhere.
    pub menu: Option<(window::Id, Point)>,
    pub submenu: Option<(window::Id, Sub)>,
    /// The compositor closed the menu's popup: the menu goes without folding back.
    pub dismissed: bool,
    /// The popup of the folders folded behind « … » (lot E-04d).
    pub folded: Option<window::Id>,
}

/// A rectangle in a surface: x, y, width, height, in logical pixels.
pub type Rect = (i32, i32, u32, u32);

/// The anchor rectangle that puts the menu box, `margins` inside its popup, at `at`: the
/// popup's top left corner at the rectangle's (below right of the point), or its bottom right
/// corner at the rectangle's when the compositor flips it (above left of the point).
pub fn menu_anchor(at: Point, margins: Padding) -> Rect {
    (
        (at.x - margins.left).round() as i32,
        (at.y - margins.top).round() as i32,
        (margins.left + margins.right).round().max(1.0) as u32,
        (margins.top + margins.bottom).round().max(1.0) as u32,
    )
}

/// The anchor rectangle of a submenu in its menu's popup: beside the entry whose top is
/// `row_top` in the menu box, on the right, or on the left once flipped; its first row level
/// with the entry, or its last one when flipped up.
pub fn submenu_anchor(parent: Padding, row_top: f32, sub: Padding) -> Rect {
    let row_left = parent.left;
    let row_right = parent.left + metrics::CONTEXT_MENU_WIDTH;
    let gap = metrics::SUBMENU_GAP;
    let top = parent.top + row_top - metrics::MENU_PADDING - metrics::BORDER_WIDTH - sub.top;
    let bottom = parent.top
        + row_top
        + metrics::MENU_ITEM_HEIGHT
        + metrics::MENU_PADDING
        + metrics::BORDER_WIDTH
        + sub.bottom;
    let left = row_left - gap + sub.right;
    let right = row_right + gap - sub.left;
    (
        left.round() as i32,
        top.round() as i32,
        (right - left).round().max(1.0) as u32,
        (bottom - top).round().max(1.0) as u32,
    )
}

/// The size of a popup holding a menu box of `width` by `height`.
pub(super) fn popup_size(width: f32, height: f32, margins: Padding) -> Option<PixelSize> {
    PixelSize::try_px(
        (width + margins.left + margins.right).ceil() as u32,
        (height + margins.top + margins.bottom).ceil() as u32,
    )
}

/// Clicks reach the menu box only: the shadow around it is not part of the menu.
pub(super) fn input_region(
    id: window::Id,
    width: f32,
    height: f32,
    margins: Padding,
) -> Task<Message> {
    let (x, y) = (margins.left as i32, margins.top as i32);
    let (w, h) = (width.ceil() as i32, height.ceil() as i32);
    let region = RegionActionCB::new(move |region| {
        if let Err(error) = region.add(x, y, w, h) {
            tracing::warn!(%error, "cannot limit the menu's input region");
        }
    });
    Task::done(Message::SetInputRegion {
        id,
        callback: region,
    })
}

pub(super) fn position((x, y, _, _): Rect) -> Position {
    Position::new(x, y)
}

pub(super) fn extent((_, _, width, height): Rect) -> Option<PixelSize> {
    PixelSize::try_px(width, height)
}

impl App {
    /// Behaves as on Wayland, with a main window of this id: for the tests.
    #[cfg(test)]
    pub(crate) fn pretend_wayland(&mut self) -> window::Id {
        let id = window::Id::unique();
        self.main = Some(id);
        id
    }

    /// Whether the menus are popups of their own: on Wayland, with `iced_exwlshell`.
    pub(crate) fn menus_in_popups(&self) -> bool {
        self.main.is_some()
    }

    /// Opens, moves or closes the popups to match the menu: called after each message.
    pub(super) fn sync_popups(&mut self) -> Task<Message> {
        let Some(main) = self.main else {
            return Task::none();
        };
        let margins = Elevation::Menu.margins();
        // What is shown: the open menu, or the one folding back until it has faded out.
        let shown = match (&self.menu, &self.closing) {
            (Some(menu), _) => Some((menu.clone(), self.menu_items(menu.on_selection))),
            (None, Some(closing)) => Some((closing.menu.clone(), closing.items.clone())),
            (None, None) => None,
        };
        let mut tasks = vec![self.sync_folded(main, margins), self.sync_unfocused(main)];
        // The menu's popup: closed when nothing shows, opened again where the menu moved.
        let wanted = shown.as_ref().map(|(menu, _)| menu.at);
        if let Some((id, at)) = self.popups.menu
            && wanted != Some(at)
        {
            tasks.push(self.remove_submenu());
            tasks.push(Task::done(Message::RemoveWindow(id)));
            self.popups.menu = None;
        }
        let Some((menu, items)) = shown else {
            return Task::batch(tasks);
        };
        if self.popups.menu.is_none() {
            let height = Self::menu_height(&items);
            let width = metrics::CONTEXT_MENU_WIDTH;
            let anchor = menu_anchor(menu.at, margins);
            if let (Some(size), Some(rect)) = (popup_size(width, height, margins), extent(anchor)) {
                let settings = IcedNewPopupSettings::new(main, size, position(anchor), rect)
                    .anchor(PopupAnchor::TopLeft)
                    .gravity(PopupGravity::BottomRight);
                let id = window::Id::unique();
                self.popups.menu = Some((id, menu.at));
                tasks.push(Task::done(Message::NewPopUp { settings, id }));
                tasks.push(input_region(id, width, height, margins));
            }
        }
        // The submenu's popup, a child of the menu's.
        if self.popups.submenu.map(|(_, sub)| sub) != menu.submenu {
            tasks.push(self.remove_submenu());
        }
        if let (Some(sub), None, Some((parent, _))) =
            (menu.submenu, self.popups.submenu, self.popups.menu)
            && let Some(entry) = items.iter().position(|i| *i == MenuItem::Submenu(sub))
        {
            let entries = self.submenu_items(sub);
            let edge = metrics::MENU_PADDING + metrics::BORDER_WIDTH;
            let row_top = Self::menu_height(&items[..entry]) - edge;
            let anchor = submenu_anchor(margins, row_top, margins);
            let width = crate::menu::submenu_width(Some(sub));
            let height = Self::menu_height(&entries);
            if let (Some(size), Some(rect)) = (popup_size(width, height, margins), extent(anchor)) {
                let settings = IcedNewPopupSettings::new(parent, size, position(anchor), rect)
                    .anchor(PopupAnchor::TopRight)
                    .gravity(PopupGravity::BottomRight);
                let id = window::Id::unique();
                self.popups.submenu = Some((id, sub));
                tasks.push(Task::done(Message::NewPopUp { settings, id }));
                tasks.push(input_region(id, width, height, margins));
            }
        }
        Task::batch(tasks)
    }

    /// The menu of « … » in a popup of its own, under « … », while it shows or fades out.
    fn sync_folded(&mut self, main: window::Id, margins: Padding) -> Task<Message> {
        let shown = self.folded.level(self.now()) > 0.0 && !self.folded.items.is_empty();
        match (shown, self.popups.folded) {
            (false, Some(id)) => {
                self.popups.folded = None;
                Task::done(Message::RemoveWindow(id))
            }
            (true, None) => {
                let rows = self.folded.items.len() as f32;
                let height = rows * metrics::MENU_ITEM_HEIGHT
                    + 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
                let width = metrics::CONTEXT_SUBMENU_WIDTH;
                let at = Point::new(self.folded.x, crate::view::folded_top());
                let anchor = menu_anchor(at, margins);
                let (Some(size), Some(rect)) = (popup_size(width, height, margins), extent(anchor))
                else {
                    return Task::none();
                };
                let settings = IcedNewPopupSettings::new(main, size, position(anchor), rect)
                    .anchor(PopupAnchor::TopLeft)
                    .gravity(PopupGravity::BottomRight);
                let id = window::Id::unique();
                self.popups.folded = Some(id);
                Task::batch([
                    Task::done(Message::NewPopUp { settings, id }),
                    input_region(id, width, height, margins),
                ])
            }
            _ => Task::none(),
        }
    }

    fn remove_submenu(&mut self) -> Task<Message> {
        match self.popups.submenu.take() {
            Some((id, _)) => Task::done(Message::RemoveWindow(id)),
            None => Task::none(),
        }
    }

    /// The compositor closed a popup (a click outside it, `popup_done`): its menu closes.
    pub(super) fn popup_closed(&mut self, id: window::Id) {
        // The completion list and the tooltip have no grab: closed by the compositor only
        // when their window goes; they are opened again if still wanted.
        if self
            .unfocused
            .suggestions
            .is_some_and(|(list, ..)| list == id)
        {
            self.unfocused.suggestions = None;
            return;
        }
        if self.unfocused.tooltip.is_some_and(|(tip, _)| tip == id) {
            self.unfocused.tooltip = None;
            return;
        }
        // A click outside the menu of « … »: it goes at once.
        if self.popups.folded == Some(id) {
            self.popups.folded = None;
            self.folded.open = false;
            self.folded.fade = None;
            return;
        }
        if self.popups.menu.is_some_and(|(menu, _)| menu == id) {
            self.popups = Popups {
                dismissed: self.menu.is_some(),
                folded: self.popups.folded,
                ..Popups::default()
            };
            self.menu = None;
            self.closing = None;
        } else if self.popups.submenu.is_some_and(|(sub, _)| sub == id) {
            self.popups.submenu = None;
            if let Some(menu) = &mut self.menu {
                menu.submenu = None;
            }
        }
    }

    /// The view of a popup, or `None` when `id` is not one.
    pub(crate) fn popup_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        let margins = Elevation::Menu.margins();
        if self.popups.folded == Some(id) {
            let panel = container(self.folded_panel())
                .padding(margins)
                .width(Length::Fill)
                .height(Length::Fill);
            return Some(panel.into());
        }
        let (menu, items, sub, live) = match (&self.menu, &self.closing) {
            (Some(menu), _) => {
                let sub = menu.submenu.map(|s| self.submenu_items(s));
                (menu, self.menu_items(menu.on_selection), sub, true)
            }
            (None, Some(closing)) => (
                &closing.menu,
                closing.items.clone(),
                Some(closing.sub.clone()),
                false,
            ),
            (None, None) => return None,
        };
        let panel = if self.popups.menu.is_some_and(|(menu, _)| menu == id) {
            self.menu_panel(menu, &items)
        } else if self.popups.submenu.is_some_and(|(sub, _)| sub == id) {
            self.submenu_panel(menu, &sub.unwrap_or_default())
        } else {
            return None;
        };
        let panel = container(panel)
            .padding(margins)
            .width(Length::Fill)
            .height(Length::Fill);
        let panel: Element<'_, Message> = panel.into();
        Some(if live {
            panel
        } else {
            panel.map(|_| Message::Inert)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_corner_lands_on_the_point_both_ways() {
        let margins = Padding {
            top: 30.0,
            right: 50.0,
            bottom: 70.0,
            left: 50.0,
        };
        let (x, y, w, h) = menu_anchor(Point::new(400.0, 300.0), margins);
        // Not flipped: the popup's top left at the rectangle's, the menu box at the point.
        assert_eq!(
            (x + margins.left as i32, y + margins.top as i32),
            (400, 300)
        );
        // Flipped both ways: the popup's bottom right at the rectangle's; the menu box's
        // bottom right corner is then at the point.
        let (right, bottom) = (x + w as i32, y + h as i32);
        assert_eq!(
            (right - margins.right as i32, bottom - margins.bottom as i32),
            (400, 300)
        );
    }

    #[test]
    fn the_submenu_sits_beside_its_entry_on_either_side() {
        let m = Padding::new(20.0);
        let (x, y, w, _) = submenu_anchor(m, 100.0, m);
        // On the right: the submenu box starts a gap after the menu's right edge.
        let menu_right = m.left + metrics::CONTEXT_MENU_WIDTH;
        assert_eq!(
            (x + w as i32) as f32 + m.left,
            (menu_right + metrics::SUBMENU_GAP).round()
        );
        // Flipped to the left: the submenu box ends a gap before the menu's left edge.
        assert_eq!(x as f32 - m.right, (m.left - metrics::SUBMENU_GAP).round());
        // Its first row level with the entry.
        let first_row = y as f32 + m.top + metrics::MENU_PADDING + metrics::BORDER_WIDTH;
        assert_eq!(first_row, m.top + 100.0);
    }
}
