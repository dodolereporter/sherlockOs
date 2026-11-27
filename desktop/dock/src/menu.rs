//! Menu of a dock icon (right click): its entries from the dock menu of the Menus board, the
//! size of its surface and its keyboard navigation.

use std::time::Duration;

use iced::keyboard::{self, Key as IcedKey, key::Named};
use sherlock_ui::dock::{self, text};
use sherlock_ui::metrics;
use sherlock_ui::motion::{Animation, Fade, SCALE_PRECISION};

/// What an entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// Brings this window forward, by its id (`windows::Window::id`).
    Window(u64),
    /// Starts the application, when it has no window.
    Open,
    /// Starts it again for a new window: its « new window » action, or its `Exec`.
    NewWindow,
    /// Runs the action of this index in `DesktopEntry::actions`.
    Action(usize),
    /// Adds an open application to the pinned list.
    Keep,
    /// Removes it from the pinned list.
    Unpin,
    /// Closes its windows. Needs a source of windows that can close them.
    Quit,
}

/// An entry as the menu shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub entry: Entry,
    pub label: String,
    pub enabled: bool,
    /// The active window, checked as on macOS.
    pub checked: bool,
    /// A separator comes before it.
    pub starts_group: bool,
}

impl Line {
    fn new(entry: Entry, label: impl Into<String>, enabled: bool) -> Self {
        Self {
            entry,
            label: label.into(),
            enabled,
            checked: false,
            starts_group: false,
        }
    }

    fn group(mut self) -> Self {
        self.starts_group = true;
        self
    }
}

/// What the menu needs to know about the icon it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Abilities {
    /// It has a command to start.
    pub launch: bool,
    /// Open but not pinned, with a desktop entry: « Garder dans le Dock ».
    pub keep: bool,
    /// Pinned, and not Sherlock: « Retirer du Dock ».
    pub unpin: bool,
    /// Its windows can be acted on: brought forward, closed.
    pub act: bool,
}

/// A window listed at the top of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: u64,
    pub title: String,
    pub active: bool,
}

/// The entries of an icon's menu, after the dock menu of macOS: the open windows (the active
/// one checked), « Nouvelle fenêtre » (« Ouvrir » without a window), the other actions of the
/// desktop entry (`actions`, by their index), « Garder dans le Dock » or « Retirer du Dock »,
/// « Quitter ». An action that cannot be done is disabled, never pretended.
pub fn entries(can: Abilities, windows: &[Listed], actions: &[(usize, String)]) -> Vec<Line> {
    let mut lines: Vec<Line> = windows
        .iter()
        .map(|w| Line {
            checked: w.active,
            ..Line::new(Entry::Window(w.id), w.title.clone(), can.act)
        })
        .collect();
    let start = if windows.is_empty() {
        Line::new(Entry::Open, text::OPEN, can.launch)
    } else {
        Line::new(Entry::NewWindow, text::NEW_WINDOW, can.launch).group()
    };
    lines.push(start);
    lines.extend(
        actions
            .iter()
            .map(|(index, name)| Line::new(Entry::Action(*index), name.clone(), can.launch)),
    );
    lines.push(if can.keep {
        Line::new(Entry::Keep, text::KEEP, true).group()
    } else {
        Line::new(Entry::Unpin, text::UNPIN, can.unpin).group()
    });
    lines.push(Line::new(Entry::Quit, text::QUIT, can.act && !windows.is_empty()).group());
    lines
}

/// Size of the menu surface for `lines`: width, then height.
pub fn surface_size(lines: &[Line]) -> (u32, u32) {
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let separators = lines.iter().filter(|l| l.starts_group).count();
    let height = 2.0 * metrics::MENU_PADDING
        + dock::MENU_HEADER_HEIGHT
        + lines.len() as f32 * metrics::MENU_ITEM_HEIGHT
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
pub fn move_cursor(lines: &[Line], cursor: Option<usize>, key: Key) -> Option<usize> {
    let enabled: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].enabled).collect();
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

/// The menu opening, from its icon, and closing back: a fade, and a spring that scales it from
/// `dock::MENU_SCALE`, as the menus of the bar do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unfold {
    spring: Animation,
    fade: Fade,
}

impl Unfold {
    /// Folded at `now`, opening.
    pub fn opening(now: Duration) -> Self {
        Self {
            spring: Animation::new(dock::MENU_SPRING, 0.0, 1.0, now),
            fade: Fade::new(0.0, 1.0, now, dock::MENU_FADE),
        }
    }

    /// Heads back to folded from `now`, from where it is.
    pub fn close(&mut self, now: Duration) {
        if self.spring.target() != 0.0 {
            self.spring.retarget(now, 0.0);
        }
        if self.fade.target() != 0.0 {
            self.fade.retarget(now, 0.0, dock::MENU_FADE);
        }
    }

    /// Heading for folded: its entries no longer act.
    pub fn closing(&self) -> bool {
        self.fade.target() == 0.0
    }

    /// Folded and faded out at `now`: the popup can go.
    pub fn closed(&self, now: Duration) -> bool {
        self.closing() && self.fade.is_done(now)
    }

    /// Frames are needed at `now`. Closing, only until it has faded out.
    pub fn running(&self, now: Duration) -> bool {
        let moving = !self.spring.is_done(now, SCALE_PRECISION);
        !self.fade.is_done(now) || (moving && !self.closing())
    }

    pub fn opacity(&self, now: Duration) -> f32 {
        self.fade.value(now).clamp(0.0, 1.0)
    }

    /// Scale it is drawn at: never past 1, so that it never leaves its surface.
    pub fn scale(&self, now: Duration) -> f32 {
        let progress = (self.spring.value(now) as f32).clamp(0.0, 1.0);
        dock::MENU_SCALE + (1.0 - dock::MENU_SCALE) * progress
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: Abilities = Abilities {
        launch: true,
        keep: false,
        unpin: true,
        act: false,
    };

    fn labels(lines: &[Line]) -> Vec<(&str, bool)> {
        lines
            .iter()
            .map(|l| (l.label.as_str(), l.enabled))
            .collect()
    }

    #[test]
    fn a_closed_application_opens_and_has_its_actions() {
        let actions = [(1, String::from("Nouvel onglet"))];
        assert_eq!(
            labels(&entries(APP, &[], &actions)),
            [
                ("Ouvrir", true),
                ("Nouvel onglet", true),
                ("Retirer du Dock", true),
                ("Quitter", false),
            ]
        );
        // Sherlock: everything there, nothing enabled.
        let sherlock = Abilities {
            launch: false,
            keep: false,
            unpin: false,
            act: false,
        };
        assert_eq!(
            labels(&entries(sherlock, &[], &[])),
            [
                ("Ouvrir", false),
                ("Retirer du Dock", false),
                ("Quitter", false)
            ]
        );
    }

    #[test]
    fn an_open_application_lists_its_windows_first() {
        let can = Abilities {
            act: true,
            keep: true,
            ..APP
        };
        let windows = [
            Listed {
                id: 4,
                title: String::from("notes.txt"),
                active: false,
            },
            Listed {
                id: 9,
                title: String::from("plan.md"),
                active: true,
            },
        ];
        let lines = entries(can, &windows, &[]);
        assert_eq!(
            labels(&lines),
            [
                ("notes.txt", true),
                ("plan.md", true),
                ("Nouvelle fenêtre", true),
                ("Garder dans le Dock", true),
                ("Quitter", true),
            ]
        );
        assert_eq!(lines[0].entry, Entry::Window(4));
        assert!(!lines[0].checked && lines[1].checked, "the active window");
        let groups: Vec<bool> = lines.iter().map(|l| l.starts_group).collect();
        assert_eq!(groups, [false, false, true, true, true]);
        // Windows a source cannot act on are listed, disabled, and « Quitter » too.
        let lines = entries(APP, &windows, &[]);
        assert!(!lines[0].enabled && !lines[4].enabled);
    }

    #[test]
    fn the_keyboard_skips_disabled_entries() {
        let list = entries(APP, &[], &[]);
        assert_eq!(move_cursor(&list, None, Key::Next), Some(0));
        assert_eq!(
            move_cursor(&list, None, Key::Previous),
            Some(1),
            "Quitter is off"
        );
        assert_eq!(
            move_cursor(&list, Some(1), Key::Next),
            Some(0),
            "wraps around"
        );
        assert_eq!(move_cursor(&list, Some(0), Key::Previous), Some(1));
        assert_eq!(move_cursor(&list, Some(0), Key::Last), Some(1));
        let none = entries(
            Abilities {
                launch: false,
                keep: false,
                unpin: false,
                act: false,
            },
            &[],
            &[],
        );
        assert_eq!(move_cursor(&none, None, Key::Next), None);
    }

    #[test]
    fn the_menu_unfolds_and_folds_back() {
        let ms = Duration::from_millis;
        let mut unfold = Unfold::opening(ms(100));
        assert_eq!(unfold.opacity(ms(100)), 0.0);
        assert_eq!(unfold.scale(ms(100)), dock::MENU_SCALE);
        assert!(unfold.running(ms(150)) && !unfold.closing());
        assert_eq!(unfold.opacity(ms(100) + dock::MENU_FADE), 1.0);
        assert!(!unfold.running(ms(900)));
        assert_eq!(unfold.scale(ms(900)), 1.0);
        // Closed: it fades out and shrinks, then it can go.
        unfold.close(ms(1000));
        assert!(unfold.closing() && !unfold.closed(ms(1000)));
        assert!(unfold.opacity(ms(1050)) < 1.0 && unfold.scale(ms(1050)) < 1.0);
        assert!(unfold.closed(ms(1000) + dock::MENU_FADE));
        assert!(!unfold.running(ms(1000) + dock::MENU_FADE));
    }

    #[test]
    fn the_menu_surface_follows_the_design_board() {
        // Menus board: 240 wide; padding of 6, header of 28, entries of 30, a separator of 9
        // before « Retirer du Dock » and before « Quitter ».
        assert_eq!(
            surface_size(&entries(APP, &[], &[])),
            (240, 12 + 28 + 90 + 18)
        );
    }
}
