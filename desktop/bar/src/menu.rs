//! Sherlock menu: its entries (Menus design board), the size of its surface and its keyboard
//! navigation.

use iced::Padding;
use iced::keyboard::{self, Key as IcedKey, key::Named};
use sherlock_ui::component::Trailing;
use sherlock_ui::surface::Elevation;
use sherlock_ui::{metrics, strings::top_bar as text};

/// The menu entries. None of them is wired yet: activating one logs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    About,
    Settings,
    Updates,
    GameMode,
    Lock,
    LogOut,
    PowerOff,
}

impl Action {
    pub const ALL: [Self; 7] = [
        Self::About,
        Self::Settings,
        Self::Updates,
        Self::GameMode,
        Self::Lock,
        Self::LogOut,
        Self::PowerOff,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::About => text::ABOUT,
            Self::Settings => text::SETTINGS,
            Self::Updates => text::UPDATES,
            Self::GameMode => text::GAME_MODE,
            Self::Lock => text::LOCK,
            Self::LogOut => text::LOG_OUT,
            Self::PowerOff => text::POWER_OFF,
        }
    }

    /// The board also shows the shortcuts Super+, Super+G and Super+L. No compositor binds
    /// them yet, so they are not shown: a shortcut that does nothing would mislead.
    pub fn trailing(self) -> Trailing<'static> {
        match self {
            // The design board announces a submenu that it does not draw.
            Self::PowerOff => Trailing::Submenu,
            _ => Trailing::None,
        }
    }

    /// First entry of a group: a separator comes before it.
    pub fn starts_group(self) -> bool {
        self == Self::GameMode
    }
}

/// Room around the menu in its surface. In software, only the gap below the bar: the surface
/// is opaque there and holds no shadow (ADR 0018). On the GPU, the blurred shadow of the Menus
/// board, cut on the left where the screen ends.
pub fn margins(gpu: bool) -> Padding {
    if !gpu {
        return Padding {
            top: metrics::TOP_BAR_MENU_GAP,
            ..Padding::ZERO
        };
    }
    let shadow = Elevation::Menu.margins();
    Padding {
        left: shadow
            .left
            .min(metrics::TOP_BAR_MARGIN_SIDE + metrics::TOP_BAR_PADDING.0),
        ..shadow
    }
}

/// Size of the menu surface, `margins` included.
pub fn surface_size(margins: Padding) -> (u32, u32) {
    let (top, _, bottom) = metrics::MENU_HEADER_PADDING;
    let header = top + metrics::MENU_AVATAR + bottom;
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let separators = 1 + Action::ALL.iter().filter(|a| a.starts_group()).count();
    let items = Action::ALL.len() as f32 * metrics::MENU_ITEM_HEIGHT;
    let height = 2.0 * metrics::MENU_PADDING + header + separators as f32 * separator + items;
    (
        (metrics::MENU_WIDTH + margins.left + margins.right) as u32,
        (height + margins.top + margins.bottom).ceil() as u32,
    )
}

/// Anchor rectangle of the menu surface in the bar surface (x, y, width, height). The menu
/// itself must land 6 below the Sherlock button, but its shadow starts left of and above the
/// bar surface, where xdg_positioner forbids an anchor rectangle. So the rectangle ends where
/// the surface starts, at the centre of its top edge, and the popup grows down from that point,
/// centred (anchor and gravity `Bottom`).
pub fn anchor_rect(margins: Padding) -> (i32, i32, u32, u32) {
    let (width, _) = surface_size(margins);
    let left = metrics::TOP_BAR_PADDING.0 - margins.left;
    let top = metrics::TOP_BAR_HEIGHT + metrics::TOP_BAR_MENU_GAP - margins.top;
    let centre = left as i32 + (width / 2) as i32;
    (centre - 1, 0, 2, top as u32)
}

/// The menu itself in its surface (x, y, width, height): the input region of the popup.
pub fn input_rect(margins: Padding) -> (i32, i32, i32, i32) {
    let (width, height) = surface_size(margins);
    (
        margins.left as i32,
        margins.top as i32,
        width as i32 - (margins.left + margins.right) as i32,
        height as i32 - (margins.top + margins.bottom) as i32,
    )
}

/// A key that matters in the open menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Next,
    Previous,
    First,
    Last,
    Activate,
    Close,
}

/// Menu keys: arrows, Home, End, Enter, Space, Escape. Tab goes through focus.
pub fn key(event: keyboard::Event) -> Option<Key> {
    let keyboard::Event::KeyPressed { key, .. } = event else {
        return None;
    };
    let IcedKey::Named(named) = key else {
        return None;
    };
    Some(match named {
        Named::ArrowDown => Key::Next,
        Named::ArrowUp => Key::Previous,
        Named::Home => Key::First,
        Named::End => Key::Last,
        Named::Enter | Named::Space => Key::Activate,
        Named::Escape => Key::Close,
        _ => return None,
    })
}

/// Highlighted entry after `key`. Arrows wrap around; with no highlighted entry, Down takes the
/// first one and Up the last one.
pub fn move_cursor(cursor: Option<usize>, key: Key) -> Option<usize> {
    let last = Action::ALL.len() - 1;
    match (key, cursor) {
        (Key::First, _) | (Key::Next, None) => Some(0),
        (Key::Last, _) | (Key::Previous, None) => Some(last),
        (Key::Next, Some(i)) => Some(if i >= last { 0 } else { i + 1 }),
        (Key::Previous, Some(i)) => Some(if i == 0 { last } else { i - 1 }),
        (Key::Activate | Key::Close, cursor) => cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_walk_and_wrap() {
        assert_eq!(move_cursor(None, Key::Next), Some(0));
        assert_eq!(move_cursor(None, Key::Previous), Some(6));
        assert_eq!(move_cursor(Some(0), Key::Next), Some(1));
        assert_eq!(move_cursor(Some(6), Key::Next), Some(0));
        assert_eq!(move_cursor(Some(0), Key::Previous), Some(6));
        assert_eq!(move_cursor(Some(3), Key::First), Some(0));
        assert_eq!(move_cursor(Some(3), Key::Last), Some(6));
    }

    #[test]
    fn the_menu_follows_the_design_board() {
        let labels: Vec<_> = Action::ALL.iter().map(|a| a.label()).collect();
        assert_eq!(labels[0], "À propos de cet ordinateur");
        assert_eq!(labels[6], "Éteindre");
        // Menus board: 256 wide; header of 54, two separators of 9, seven entries of 30,
        // padding of 6, plus the gap of 6 below the bar.
        assert_eq!(surface_size(margins(false)), (256, 6 + 12 + 54 + 18 + 210));
    }

    /// Where the menu itself lands on the screen, from its surface and its margins.
    fn menu_on_screen(margins: Padding) -> (f32, f32) {
        let (width, _) = surface_size(margins);
        let (x, _, w, top) = anchor_rect(margins);
        assert_eq!(w, 2);
        assert_eq!(width % 2, 0, "an odd width cannot be centred exactly");
        let surface_left = metrics::TOP_BAR_MARGIN_SIDE + (x + 1) as f32 - (width / 2) as f32;
        let surface_top = metrics::TOP_BAR_MARGIN_TOP + top as f32;
        (surface_left + margins.left, surface_top + margins.top)
    }

    #[test]
    fn the_menu_lands_where_the_board_puts_it_with_or_without_its_shadow() {
        // Menus board: the menu at (18, 52).
        for gpu in [false, true] {
            assert_eq!(menu_on_screen(margins(gpu)), (18.0, 52.0), "gpu: {gpu}");
            let (x, y, _, height) = anchor_rect(margins(gpu));
            // xdg_positioner: the anchor rectangle stays inside the bar surface.
            assert!(x >= 0 && y >= 0 && height >= 1 && height as f32 <= metrics::TOP_BAR_HEIGHT);
        }
        // The input region is the menu alone: on the GPU, the room for the shadow lies over
        // the bar (from y 20 on the screen), which must keep its pointer.
        for gpu in [false, true] {
            let (x, y, width, _) = input_rect(margins(gpu));
            let (left, top) = menu_on_screen(margins(gpu));
            let surface_top = top - margins(gpu).top;
            assert_eq!((x, width), (margins(gpu).left as i32, 256));
            assert_eq!(surface_top + y as f32, 52.0);
            assert!(
                surface_top + y as f32 >= metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT
            );
            assert_eq!(left, 18.0);
        }
        // On the GPU the surface holds the shadow: 54 of blur, offset 22 down, cut on the left
        // at the screen edge.
        let shadow = margins(true);
        assert_eq!(
            (shadow.left, shadow.top, shadow.right, shadow.bottom),
            (18.0, 32.0, 54.0, 76.0)
        );
    }
}
