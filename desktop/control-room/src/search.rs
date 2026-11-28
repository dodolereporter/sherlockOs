//! The pages of Control Room, the settings each one holds, and the search over them. A query
//! finds a setting by its name, by its page or by the words people use for it; case and
//! accents do not count, and the part of the name that matched is returned to be highlighted.

use std::ops::Range;

use sherlock_ui::control_room::texts as t;
use sherlock_ui::icon::Icon;

use crate::apps::Kind;
use crate::settings::File;

/// A page of the sidebar, in its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    Sound,
    Network,
    Appearance,
    Keyboard,
    Touchpad,
    Lock,
    Clipboard,
    DefaultApps,
}

impl Page {
    pub const ALL: [Self; 8] = [
        Self::Sound,
        Self::Network,
        Self::Appearance,
        Self::Keyboard,
        Self::Touchpad,
        Self::Lock,
        Self::Clipboard,
        Self::DefaultApps,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Sound => t::SOUND,
            Self::Network => t::NETWORK,
            Self::Appearance => t::APPEARANCE,
            Self::Keyboard => t::KEYBOARD,
            Self::Touchpad => t::TOUCHPAD,
            Self::Lock => t::LOCK,
            Self::Clipboard => t::CLIPBOARD,
            Self::DefaultApps => t::DEFAULT_APPS,
        }
    }

    /// The sentence under its title.
    pub fn about(self) -> &'static str {
        match self {
            Self::Sound => t::SOUND_ABOUT,
            Self::Network => t::NETWORK_ABOUT,
            Self::Appearance => t::APPEARANCE_ABOUT,
            Self::Keyboard => t::KEYBOARD_ABOUT,
            Self::Touchpad => t::TOUCHPAD_ABOUT,
            Self::Lock => t::LOCK_ABOUT,
            Self::Clipboard => t::CLIPBOARD_ABOUT,
            Self::DefaultApps => t::DEFAULT_APPS_ABOUT,
        }
    }

    /// Its name on the command line (`control-room --page network`), and back.
    pub fn id(self) -> &'static str {
        match self {
            Self::Sound => "sound",
            Self::Network => "network",
            Self::Appearance => "appearance",
            Self::Keyboard => "keyboard",
            Self::Touchpad => "touchpad",
            Self::Lock => "lock",
            Self::Clipboard => "clipboard",
            Self::DefaultApps => "apps",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == id)
    }

    pub fn icon(self) -> Icon {
        match self {
            Self::Sound => Icon::Sound,
            Self::Network => Icon::Network,
            Self::Appearance => Icon::Image,
            Self::Keyboard => Icon::Keyboard,
            Self::Touchpad => Icon::Touchpad,
            Self::Lock => Icon::Lock,
            Self::Clipboard => Icon::Document,
            Self::DefaultApps => Icon::Grid,
        }
    }
}

/// A setting, as the search finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    Wifi,
    Networks,
    OtherNetwork,
    Theme,
    LightFrom,
    DarkFrom,
    Wallpaper,
    FollowTheme,
    DockSize,
    Magnification,
    MagnificationFactor,
    AutoHide,
    Position,
    RightClick,
    TapToClick,
    NaturalScroll,
    Speed,
    Typing,
    ShowClicks,
    TapAndDrag,
    DragLock,
    SwipeDown,
    Layout,
    RepeatDelay,
    RepeatRate,
    Shortcuts,
    OutputDevice,
    OutputVolume,
    InputDevice,
    InputVolume,
    LockAfter,
    ClipboardHistory,
    ClipboardKeep,
    App(Kind),
}

impl Setting {
    pub const ALL: [Self; 41] = [
        Self::Wifi,
        Self::Networks,
        Self::OtherNetwork,
        Self::Theme,
        Self::LightFrom,
        Self::DarkFrom,
        Self::Wallpaper,
        Self::FollowTheme,
        Self::DockSize,
        Self::Magnification,
        Self::MagnificationFactor,
        Self::AutoHide,
        Self::Position,
        Self::RightClick,
        Self::TapToClick,
        Self::NaturalScroll,
        Self::Speed,
        Self::Typing,
        Self::ShowClicks,
        Self::TapAndDrag,
        Self::DragLock,
        Self::SwipeDown,
        Self::Layout,
        Self::RepeatDelay,
        Self::RepeatRate,
        Self::Shortcuts,
        Self::OutputDevice,
        Self::OutputVolume,
        Self::InputDevice,
        Self::InputVolume,
        Self::LockAfter,
        Self::ClipboardHistory,
        Self::ClipboardKeep,
        Self::App(Kind::Browser),
        Self::App(Kind::Mail),
        Self::App(Kind::Editor),
        Self::App(Kind::Terminal),
        Self::App(Kind::Files),
        Self::App(Kind::Images),
        Self::App(Kind::Video),
        Self::App(Kind::Music),
    ];

    /// Its page, its name, and the other words it is found by.
    pub fn entry(self) -> (Page, &'static str, &'static str) {
        use Page::*;
        match self {
            Self::Wifi => (Network, t::WIFI, t::NETWORK_WORDS),
            Self::Networks => (Network, t::NETWORKS, t::NETWORK_WORDS),
            Self::OtherNetwork => (Network, t::OTHER_NETWORK_SEARCH, t::NETWORK_WORDS),
            Self::Theme => (Appearance, t::THEME, t::THEME_WORDS),
            Self::LightFrom => (Appearance, t::LIGHT_FROM, t::HOURS_WORDS),
            Self::DarkFrom => (Appearance, t::DARK_FROM, t::HOURS_WORDS),
            Self::Wallpaper => (Appearance, t::WALLPAPER, t::WALLPAPER_WORDS),
            Self::FollowTheme => (Appearance, t::FOLLOW_THEME, t::WALLPAPER_WORDS),
            Self::DockSize => (Appearance, t::DOCK_SIZE_SEARCH, t::DOCK_WORDS),
            Self::Magnification => (Appearance, t::MAGNIFICATION, t::DOCK_WORDS),
            Self::MagnificationFactor => (Appearance, t::MAGNIFICATION_SEARCH, t::DOCK_WORDS),
            Self::AutoHide => (Appearance, t::AUTO_HIDE_SEARCH, t::DOCK_WORDS),
            Self::Position => (Appearance, t::POSITION_SEARCH, t::DOCK_WORDS),
            Self::RightClick => (Touchpad, t::RIGHT_CLICK, t::CLICK_WORDS),
            Self::TapToClick => (Touchpad, t::TAP_TO_CLICK, t::CLICK_WORDS),
            Self::NaturalScroll => (Touchpad, t::NATURAL_SCROLL, t::SCROLL_WORDS),
            Self::Speed => (Touchpad, t::SPEED, t::POINTER_WORDS),
            Self::Typing => (Touchpad, t::TYPING, t::TOUCHPAD_WORDS),
            Self::ShowClicks => (Touchpad, t::SHOW_CLICKS, t::POINTER_WORDS),
            Self::TapAndDrag => (Touchpad, t::TAP_AND_DRAG, t::GESTURE_WORDS),
            Self::DragLock => (Touchpad, t::DRAG_LOCK, t::GESTURE_WORDS),
            Self::SwipeDown => (Touchpad, t::SWIPE_DOWN, t::GESTURE_WORDS),
            Self::Layout => (Keyboard, t::LAYOUT, t::KEYBOARD_WORDS),
            Self::RepeatDelay => (Keyboard, t::REPEAT_DELAY, t::REPEAT_WORDS),
            Self::RepeatRate => (Keyboard, t::REPEAT_RATE, t::REPEAT_WORDS),
            Self::Shortcuts => (Keyboard, t::SHORTCUTS, t::SHORTCUT_WORDS),
            Self::OutputDevice => (Sound, t::OUTPUT_DEVICE, t::SOUND_WORDS),
            Self::OutputVolume => (Sound, t::OUTPUT_VOLUME, t::SOUND_WORDS),
            Self::InputDevice => (Sound, t::INPUT_DEVICE, t::MICROPHONE_WORDS),
            Self::InputVolume => (Sound, t::INPUT_VOLUME, t::MICROPHONE_WORDS),
            Self::LockAfter => (Lock, t::LOCK_AFTER, t::LOCK_WORDS),
            Self::ClipboardHistory => (Clipboard, t::CLIPBOARD_HISTORY, t::CLIPBOARD_WORDS),
            Self::ClipboardKeep => (Clipboard, t::CLIPBOARD_KEEP, t::CLIPBOARD_WORDS),
            Self::App(kind) => (DefaultApps, kind.texts().0, t::APPS_WORDS),
        }
    }

    pub fn page(self) -> Page {
        self.entry().0
    }

    /// The settings file that holds it; none for the sound and the default applications,
    /// which go through `pactl` and `mime-apps`.
    pub fn file(self) -> Option<File> {
        let file = match self.page() {
            Page::Appearance => match self {
                Self::Theme | Self::LightFrom | Self::DarkFrom => File::Appearance,
                Self::Wallpaper | Self::FollowTheme => File::Wallpaper,
                _ => File::Dock,
            },
            Page::Touchpad => match self {
                Self::ShowClicks => File::Pointer,
                _ => File::Touchpad,
            },
            Page::Keyboard => match self {
                Self::Shortcuts => return None,
                _ => File::Keyboard,
            },
            Page::Lock => File::Lock,
            Page::Clipboard => File::Clipboard,
            Page::Sound | Page::Network | Page::DefaultApps => return None,
        };
        Some(file)
    }
}

/// A setting found, and the byte ranges of its name that the query matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub setting: Setting,
    pub marks: Vec<Range<usize>>,
}

/// The settings that hold every word of `query`, in their name, their page's name or their
/// other words: those whose name starts with the query first, then those whose name holds
/// it, then the others, each group in the order of the pages. An empty query finds nothing.
pub fn search(query: &str) -> Vec<Hit> {
    let words: Vec<String> = query.split_whitespace().map(fold).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let whole = fold(query.trim());
    let mut found: Vec<(u8, Hit)> = Setting::ALL
        .into_iter()
        .filter_map(|setting| {
            let (page, name, others) = setting.entry();
            let haystack = fold(&format!("{name} {} {others}", page.name()));
            if !words.iter().all(|word| haystack.contains(word.as_str())) {
                return None;
            }
            let marks: Vec<Range<usize>> = words.iter().filter_map(|w| find(name, w)).collect();
            let rank = match fold(name) {
                n if n.starts_with(&whole) => 0,
                n if n.contains(&whole) || !marks.is_empty() => 1,
                _ => 2,
            };
            Some((rank, Hit { setting, marks }))
        })
        .collect();
    found.sort_by_key(|(rank, _)| *rank);
    found.into_iter().map(|(_, hit)| hit).collect()
}

/// One character without its case nor its accent: each character gives exactly one, so that
/// a position in the folded text is a position in the text.
fn fold_char(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'à' | 'â' | 'ä' => 'a',
        'ç' => 'c',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' => 'i',
        'ô' | 'ö' => 'o',
        'ù' | 'û' | 'ü' => 'u',
        'ÿ' => 'y',
        '’' => '\'',
        other => other,
    }
}

pub fn fold(text: &str) -> String {
    text.chars().map(fold_char).collect()
}

/// Byte range in `text` of the first place where `word` (folded) matches it, folded.
fn find(text: &str, word: &str) -> Option<Range<usize>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let wanted: Vec<char> = word.chars().collect();
    let start = (0..=chars.len().checked_sub(wanted.len())?).find(|&i| {
        chars[i..i + wanted.len()]
            .iter()
            .zip(&wanted)
            .all(|((_, c), w)| fold_char(*c) == *w)
    })?;
    let end = chars
        .get(start + wanted.len())
        .map_or(text.len(), |(at, _)| *at);
    Some(chars[start].0..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(query: &str) -> Vec<&'static str> {
        search(query)
            .into_iter()
            .map(|h| h.setting.entry().1)
            .collect()
    }

    #[test]
    fn a_setting_is_found_by_its_name_without_case_nor_accents() {
        assert_eq!(names("defilement"), [t::NATURAL_SCROLL]);
        assert_eq!(names("DÉFILEMENT"), [t::NATURAL_SCROLL]);
        let hit = &search("naturel")[0];
        assert_eq!(hit.setting, Setting::NaturalScroll);
        // The marks fall on the name as it is shown, accents included.
        let name = t::NATURAL_SCROLL;
        assert_eq!(&name[hit.marks[0].clone()], "naturel");
        let hit = &search("defil")[0];
        assert_eq!(&name[hit.marks[0].clone()], "Défil");
    }

    #[test]
    fn other_words_and_the_page_find_a_setting_too() {
        // « sombre » is in the name of a setting, and a word of the theme.
        let found = names("sombre");
        assert_eq!(found[0], t::DARK_FROM, "its name holds the query");
        assert!(found.contains(&t::THEME));
        // A word that is not in any name: the setting is found, nothing is marked.
        let hit = search("trackpad");
        assert!(hit.iter().any(|h| h.setting == Setting::TapToClick));
        assert!(hit.iter().all(|h| h.marks.is_empty()));
        // Every word must be found.
        assert_eq!(names("dock position"), [t::POSITION_SEARCH]);
        assert!(names("dock pavé").is_empty());
        assert!(names("   ").is_empty());
        assert!(names("introuvable").is_empty());
    }

    #[test]
    fn names_that_start_with_the_query_come_first() {
        let found = names("clic");
        assert_eq!(found[0], t::RIGHT_CLICK);
        assert!(found.contains(&t::TAP_TO_CLICK));
    }

    #[test]
    fn every_setting_is_on_a_page_with_a_name() {
        for setting in Setting::ALL {
            let (page, name, others) = setting.entry();
            assert!(Page::ALL.contains(&page));
            assert!(!name.is_empty() && !others.is_empty());
            assert_eq!(search(name)[0].setting, setting, "{name}");
        }
    }
}
