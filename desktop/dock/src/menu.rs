//! Menu of a dock icon (right click): its entries from the dock menu of the Menus board, the
//! size of its surface and its keyboard navigation.

use iced::keyboard::{self, Key as IcedKey, key::Named};
use sherlock_ui::dock::{self, text};
use sherlock_ui::metrics;

/// What an entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// Starts the application.
    Open,
    /// Runs its « new window » action. Only there when the desktop entry declares one.
    NewWindow,
    /// Adds an open application to the pinned list.
    Keep,
    /// Removes it from the pinned list.
    Unpin,
    /// Closes its windows. Needs a source of windows that can close them.
    Quit,
}

impl Entry {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => text::OPEN,
            Self::NewWindow => text::NEW_WINDOW,
            Self::Keep => text::KEEP,
            Self::Unpin => text::UNPIN,
            Self::Quit => text::QUIT,
        }
    }

    /// First entry of a group: a separator comes before it.
    pub fn starts_group(self) -> bool {
        self == Self::Quit
    }
}

/// What the menu needs to know about the icon it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Abilities {
    pub launch: bool,
    pub new_window: bool,
    /// Open but not pinned, with a desktop entry: « Garder dans le Dock ».
    pub keep: bool,
    /// Pinned, and not Sherlock: « Retirer du Dock ».
    pub unpin: bool,
    pub quit: bool,
}

/// The entries of an icon's menu, each with whether it is enabled. An action that does not
/// exist is disabled, never pretended. An open application that is not pinned gets « Garder
/// dans le dock » where a pinned one gets « Retirer du Dock ».
pub fn entries(can: Abilities) -> Vec<(Entry, bool)> {
    let mut entries = vec![(Entry::Open, can.launch)];
    if can.new_window {
        entries.push((Entry::NewWindow, true));
    }
    if can.keep {
        entries.push((Entry::Keep, true));
    } else {
        entries.push((Entry::Unpin, can.unpin));
    }
    entries.push((Entry::Quit, can.quit));
    entries
}

/// Size of the menu surface for `entries`: width, then height.
pub fn surface_size(entries: &[(Entry, bool)]) -> (u32, u32) {
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let separators = entries.iter().filter(|(e, _)| e.starts_group()).count();
    let height = 2.0 * metrics::MENU_PADDING
        + dock::MENU_HEADER_HEIGHT
        + entries.len() as f32 * metrics::MENU_ITEM_HEIGHT
        + separators as f32 * separator;
    (dock::MENU_WIDTH as u32, height.ceil() as u32)
}

/// A key that matters to the dock or its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Next,
    Previous,
    First,
    Last,
    Activate,
    /// Menu key, or Shift+F10: opens the menu of the focused icon.
    Menu,
    Close,
}

/// Keys: arrows, Home, End, Enter, Space, Escape, the menu key and Shift+F10. Tab goes through
/// `focus::tab`.
pub fn key(event: keyboard::Event) -> Option<Key> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    let IcedKey::Named(named) = key else {
        return None;
    };
    Some(match named {
        Named::ArrowDown | Named::ArrowRight => Key::Next,
        Named::ArrowUp | Named::ArrowLeft => Key::Previous,
        Named::Home => Key::First,
        Named::End => Key::Last,
        Named::Enter | Named::Space => Key::Activate,
        Named::ContextMenu => Key::Menu,
        Named::F10 if modifiers.shift() => Key::Menu,
        Named::Escape => Key::Close,
        _ => return None,
    })
}

/// Highlighted entry after `key`, among the enabled ones. Arrows wrap around; with nothing
/// highlighted, Down takes the first enabled entry and Up the last one.
pub fn move_cursor(entries: &[(Entry, bool)], cursor: Option<usize>, key: Key) -> Option<usize> {
    let enabled: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].1).collect();
    let (first, last) = (enabled.first().copied(), enabled.last().copied());
    let position = cursor.and_then(|c| enabled.iter().position(|&i| i == c));
    match (key, position) {
        (Key::First, _) | (Key::Next, None) => first,
        (Key::Last, _) | (Key::Previous, None) => last,
        (Key::Next, Some(p)) => enabled.get(p + 1).copied().or(first),
        (Key::Previous, Some(p)) => p
            .checked_sub(1)
            .and_then(|p| enabled.get(p).copied())
            .or(last),
        (Key::Activate | Key::Menu | Key::Close, _) => cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: Abilities = Abilities {
        launch: true,
        new_window: true,
        keep: false,
        unpin: true,
        quit: false,
    };

    #[test]
    fn the_entries_follow_what_the_application_can_do() {
        let labels = |can| -> Vec<(&str, bool)> {
            entries(can)
                .into_iter()
                .map(|(e, on)| (e.label(), on))
                .collect()
        };
        assert_eq!(
            labels(APP),
            [
                ("Ouvrir", true),
                ("Nouvelle fenêtre", true),
                ("Retirer du Dock", true),
                ("Quitter", false),
            ]
        );
        // No « new window » action: no entry. Sherlock: everything there, nothing enabled.
        let sherlock = Abilities {
            launch: false,
            new_window: false,
            keep: false,
            unpin: false,
            quit: false,
        };
        assert_eq!(
            labels(sherlock),
            [
                ("Ouvrir", false),
                ("Retirer du Dock", false),
                ("Quitter", false)
            ]
        );
    }

    #[test]
    fn the_keyboard_skips_disabled_entries() {
        let list = entries(APP);
        assert_eq!(move_cursor(&list, None, Key::Next), Some(0));
        assert_eq!(
            move_cursor(&list, None, Key::Previous),
            Some(2),
            "Quitter is off"
        );
        assert_eq!(
            move_cursor(&list, Some(2), Key::Next),
            Some(0),
            "wraps around"
        );
        assert_eq!(move_cursor(&list, Some(0), Key::Previous), Some(2));
        assert_eq!(move_cursor(&list, Some(1), Key::Last), Some(2));
        let none = entries(Abilities {
            launch: false,
            new_window: false,
            keep: false,
            unpin: false,
            quit: false,
        });
        assert_eq!(move_cursor(&none, None, Key::Next), None);
    }

    #[test]
    fn the_menu_surface_follows_the_design_board() {
        // Menus board: 240 wide; padding of 6, header of 28, entries of 30, one separator of
        // 9 before « Quitter ».
        assert_eq!(surface_size(&entries(APP)), (240, 12 + 28 + 120 + 9));
    }
}
