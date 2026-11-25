//! Sherlock menu: its entries (Menus design board), the size of its surface and its keyboard
//! navigation.

use iced::keyboard::{self, Key as IcedKey, key::Named};
use sherlock_ui::component::Trailing;
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

    pub fn trailing(self) -> Trailing<'static> {
        match self {
            Self::Settings => Trailing::Shortcut(text::SETTINGS_SHORTCUT),
            Self::GameMode => Trailing::Shortcut(text::GAME_MODE_SHORTCUT),
            Self::Lock => Trailing::Shortcut(text::LOCK_SHORTCUT),
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

/// Size of the menu surface: width, then height including the gap below the bar.
pub fn surface_size() -> (u32, u32) {
    let (top, _, bottom) = metrics::MENU_HEADER_PADDING;
    let header = top + metrics::MENU_AVATAR + bottom;
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let separators = 1 + Action::ALL.iter().filter(|a| a.starts_group()).count();
    let items = Action::ALL.len() as f32 * metrics::MENU_ITEM_HEIGHT;
    let height = metrics::TOP_BAR_MENU_GAP
        + 2.0 * metrics::MENU_PADDING
        + header
        + separators as f32 * separator
        + items;
    (metrics::MENU_WIDTH as u32, height.ceil() as u32)
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
        assert_eq!(surface_size(), (256, 6 + 12 + 54 + 18 + 210));
    }
}
