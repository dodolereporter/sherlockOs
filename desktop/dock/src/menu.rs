//! Menu of a dock icon (right click): its entries from the dock menu of the Menus board, the
//! size of its surface and its keyboard navigation. Also the dock's own menu, on its separator
//! or between its icons, as the Dock options of macOS.

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
    /// App Expose: Mission Control on its windows.
    ShowAll,
    /// Closes its windows. Needs a source of windows that can close them.
    Quit,
    /// The title of the options' group: not an action (lot D-21i).
    Options,
    /// Starts it, or no longer, when the session opens: its file in the XDG autostart folder.
    OpenAtLogin,
    /// Opens Evidence on the folder of its desktop file.
    ShowInEvidence,
    /// Minimizes its windows, as macOS hides an application.
    Hide,
    /// Kills the process of its windows: the compositor sends SIGKILL to their client.
    ForceQuit,
    /// The dock's own menu: puts Sherlock back in the dock, or takes it out.
    ShowSherlock,
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
    /// Pinned, Sherlock included: « Retirer du Dock ».
    pub unpin: bool,
    /// Its windows can be acted on: brought forward, closed.
    pub act: bool,
    /// App Expose can show its windows: the source offers it, and a window is shown.
    pub expose: bool,
    /// Its desktop entry, and whether it opens with the session (`Some(true)`); `None`
    /// without an entry.
    pub autostart: Option<bool>,
    /// Evidence is installed and the desktop file is known: « Afficher dans Evidence ».
    pub evidence: bool,
    /// The source can force its windows' process to quit (`sherlock_shell_v1` version 5).
    pub force: bool,
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
/// « Afficher toutes les fenêtres », « Quitter ». An action that cannot be done is disabled,
/// never pretended.
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
    // « Options », as the submenu of macOS: what is kept, checked (lot D-21i).
    lines.push(Line::new(Entry::Options, text::OPTIONS, false).group());
    lines.push(if can.keep {
        Line::new(Entry::Keep, text::KEEP, true)
    } else {
        Line {
            checked: can.unpin,
            ..Line::new(Entry::Unpin, text::KEEP, can.unpin)
        }
    });
    lines.push(Line {
        checked: can.autostart == Some(true),
        ..Line::new(
            Entry::OpenAtLogin,
            text::OPEN_AT_LOGIN,
            can.autostart.is_some(),
        )
    });
    lines.push(Line::new(
        Entry::ShowInEvidence,
        text::SHOW_IN_EVIDENCE,
        can.evidence,
    ));
    let open = can.act && !windows.is_empty();
    lines.push(Line::new(Entry::ShowAll, text::SHOW_ALL, can.act && can.expose).group());
    lines.push(Line::new(Entry::Hide, text::HIDE, open));
    lines.push(Line::new(Entry::Quit, text::QUIT, open));
    lines.push(Line::new(
        Entry::ForceQuit,
        text::FORCE_QUIT,
        open && can.force,
    ));
    lines
}

/// The entries of the dock's own menu: « Afficher Sherlock dans le Dock », checked while it is
/// there.
pub fn dock_entries(sherlock: bool) -> Vec<Line> {
    vec![Line {
        checked: sherlock,
        ..Line::new(Entry::ShowSherlock, text::SHOW_SHERLOCK, true)
    }]
}

/// Size of the menu surface for `lines`: width, then height.
pub fn surface_size(lines: &[Line]) -> (u32, u32) {
    let separator = metrics::BORDER_WIDTH + 2.0 * metrics::MENU_SEPARATOR_MARGIN.0;
    let separators = lines.iter().filter(|l| l.starts_group).count();
    let height = 2.0 * metrics::MENU_PADDING
        + dock::MENU_HEADER_HEIGHT
        + lines.len() as f32 * metrics::MENU_ITEM_HEIGHT
        + separators as f32 * separator;
    (width(lines) as u32, height.ceil() as u32)
}

/// Width of the menu for `lines`: the board's, wider for the dock's own menu.
pub fn width(lines: &[Line]) -> f32 {
    if lines.iter().any(|l| l.entry == Entry::ShowSherlock) {
        dock::OWN_MENU_WIDTH
    } else {
        dock::MENU_WIDTH
    }
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
        expose: false,
        autostart: Some(false),
        evidence: true,
        force: false,
    };

    fn labels(lines: &[Line]) -> Vec<(&str, bool)> {
        lines
            .iter()
            .map(|l| (l.label.as_str(), l.enabled))
            .collect()
    }

    #[test]
    fn a_closed_application_opens_and_has_its_actions_and_options() {
        let actions = [(1, String::from("Nouvel onglet"))];
        let lines = entries(APP, &[], &actions);
        assert_eq!(
            labels(&lines),
            [
                ("Ouvrir", true),
                ("Nouvel onglet", true),
                ("Options", false),
                ("Garder dans le Dock", true),
                ("Ouvrir à l’ouverture de session", true),
                ("Afficher dans Evidence", true),
                ("Afficher toutes les fenêtres", false),
                ("Masquer", false),
                ("Quitter", false),
                ("Forcer à quitter", false),
            ]
        );
        // Pinned: « Garder dans le Dock » checked, a click unpins it.
        assert_eq!((lines[3].entry, lines[3].checked), (Entry::Unpin, true));
        assert!(!lines[4].checked);
        // Sherlock: no desktop entry, nothing to open with the session nor to show.
        let sherlock = Abilities {
            launch: false,
            autostart: None,
            evidence: false,
            ..APP
        };
        let lines = entries(sherlock, &[], &[]);
        let enabled: Vec<&str> = lines
            .iter()
            .filter(|l| l.enabled)
            .map(|l| l.label.as_str())
            .collect();
        assert_eq!(enabled, ["Garder dans le Dock"]);
    }

    #[test]
    fn the_dock_menu_shows_or_hides_sherlock() {
        let shown = dock_entries(true);
        assert_eq!(labels(&shown), [("Afficher Sherlock dans le Dock", true)]);
        assert!(shown[0].checked && !dock_entries(false)[0].checked);
        assert_eq!(shown[0].entry, Entry::ShowSherlock);
    }

    #[test]
    fn an_open_application_lists_its_windows_first_and_can_be_forced_to_quit() {
        let can = Abilities {
            act: true,
            keep: true,
            unpin: false,
            expose: true,
            autostart: Some(true),
            force: true,
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
                ("Options", false),
                ("Garder dans le Dock", true),
                ("Ouvrir à l’ouverture de session", true),
                ("Afficher dans Evidence", true),
                ("Afficher toutes les fenêtres", true),
                ("Masquer", true),
                ("Quitter", true),
                ("Forcer à quitter", true),
            ]
        );
        assert_eq!(lines[0].entry, Entry::Window(4));
        assert!(!lines[0].checked && lines[1].checked, "the active window");
        assert_eq!((lines[4].entry, lines[4].checked), (Entry::Keep, false));
        assert!(lines[5].checked, "opens with the session");
        let groups: Vec<usize> = (0..lines.len())
            .filter(|&i| lines[i].starts_group)
            .collect();
        assert_eq!(groups, [2, 3, 7]);
        // Windows a source cannot act on are listed, disabled, and their actions too; without
        // version 5 of the shell protocol, « Forcer à quitter » is off.
        let lines = entries(APP, &windows, &[]);
        assert!(!lines[0].enabled && !lines[8].enabled && !lines[9].enabled);
        let lines = entries(
            Abilities {
                force: false,
                ..can
            },
            &windows,
            &[],
        );
        assert!(lines[9].enabled && !lines[10].enabled);
    }

    #[test]
    fn the_keyboard_skips_disabled_entries() {
        let list = entries(APP, &[], &[]);
        // Ouvrir, then the options' title is skipped.
        assert_eq!(move_cursor(&list, None, Key::Next), Some(0));
        assert_eq!(move_cursor(&list, Some(0), Key::Next), Some(2));
        assert_eq!(
            move_cursor(&list, None, Key::Previous),
            Some(4),
            "App Expose, Masquer and the Quit entries are off"
        );
        assert_eq!(
            move_cursor(&list, Some(4), Key::Next),
            Some(0),
            "wraps around"
        );
        let none = entries(
            Abilities {
                launch: false,
                unpin: false,
                autostart: None,
                evidence: false,
                ..APP
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
        // before « Options » and before « Afficher toutes les fenêtres ».
        assert_eq!(
            surface_size(&entries(APP, &[], &[])),
            (240, 12 + 28 + 9 * 30 + 18)
        );
    }
}
