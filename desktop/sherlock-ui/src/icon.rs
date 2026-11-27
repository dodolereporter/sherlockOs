//! Line icons: 24 grid, 1.7 stroke, round caps and joins.
//!
//! An icon is only an SVG path. The frame (grid, stroke, rounding) is shared and comes from
//! the tokens; the color is given at display time. To add one: a variant, its path drawn on
//! the 24 grid, and an entry in `Icon::ALL`.

use crate::metrics;

/// The icons of the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    /// Opens a submenu.
    Chevron,
    /// Checked choice in a menu.
    Check,
    /// Search.
    Search,
    /// Close.
    Close,
    /// Network connected.
    Network,
    /// Sound on.
    Sound,
    /// Go back: previous folder.
    Back,
    /// Ascending and descending sort.
    SortUp,
    SortDown,
    /// Minimize and maximize a window.
    Minimize,
    Maximize,
    /// Places and file kinds (Evidence mock-up).
    Home,
    Desktop,
    Document,
    Download,
    Folder,
    Image,
    Trash,
    /// Container: Dockerfile, compose file.
    Cube,
    /// Devices: system disk (from the mock-up), internal disk, USB key, external disk,
    /// memory card, optical disc; eject and lock.
    SystemDisk,
    Drive,
    UsbKey,
    ExternalDrive,
    Card,
    Optical,
    Eject,
    Lock,
    /// Network disconnected.
    NetworkOff,
    /// Sound muted.
    SoundMuted,
    /// Battery.
    Battery,
    /// Battery charging.
    BatteryCharging,
    /// Microphone listening, and muted (on-screen display of the media keys, lot D-35).
    Microphone,
    MicrophoneMuted,
    /// Screen brightness: a sun.
    Brightness,
    /// Keyboard backlight: a keyboard under three rays (lot D-35d).
    KeyboardBrightness,
    /// The password field of the lock screen (lot D-25a): show the password, hide it, send it.
    Eye,
    EyeOff,
    ArrowRight,
    /// The machine buttons of the lock screen: sleep (a moon), restart, power off.
    Sleep,
    Restart,
    Power,
    /// An open arc, turned while the password is checked.
    Spinner,
    Bluetooth,
    Touchpad,
    Grid,
    Sidebar,
    /// A person: « Autre… » on the login screen (lot D-24).
    Person,
}

impl Icon {
    /// All the icons, for the tests and the gallery.
    pub const ALL: [Self; 47] = [
        Self::Chevron,
        Self::Check,
        Self::Search,
        Self::Close,
        Self::Network,
        Self::Sound,
        Self::Back,
        Self::SortUp,
        Self::SortDown,
        Self::Minimize,
        Self::Maximize,
        Self::Home,
        Self::Desktop,
        Self::Document,
        Self::Download,
        Self::Folder,
        Self::Image,
        Self::Trash,
        Self::Cube,
        Self::SystemDisk,
        Self::Drive,
        Self::UsbKey,
        Self::ExternalDrive,
        Self::Card,
        Self::Optical,
        Self::Eject,
        Self::Lock,
        Self::NetworkOff,
        Self::SoundMuted,
        Self::Battery,
        Self::BatteryCharging,
        Self::Microphone,
        Self::MicrophoneMuted,
        Self::Brightness,
        Self::KeyboardBrightness,
        Self::Eye,
        Self::EyeOff,
        Self::ArrowRight,
        Self::Sleep,
        Self::Restart,
        Self::Power,
        Self::Spinner,
        Self::Bluetooth,
        Self::Touchpad,
        Self::Grid,
        Self::Sidebar,
        Self::Person,
    ];

    /// Path on the 24 grid. All come from the mock-ups, except the cross and the two sort
    /// arrows. The device icons are drawn for Evidence, on the same grid and stroke, except
    /// the system disk, which is the mock-up's. The boards draw neither the battery, nor the
    /// network off, nor the muted sound, nor the microphone, the sun and the lit keyboard of
    /// the media keys: those follow the stroke of the others.
    pub const fn path(self) -> &'static str {
        match self {
            Self::Chevron => "M9 6l6 6-6 6",
            Self::Check => "M5 12.5l4.5 4.5L19 7.5",
            Self::Search => "M10.5 4a6.5 6.5 0 1 0 0 13a6.5 6.5 0 0 0 0-13zM15.3 15.3L20 20",
            Self::Close => "M6 6l12 12M18 6L6 18",
            Self::Network => {
                "M3 9.5a13 13 0 0 1 18 0M6.2 13a8.5 8.5 0 0 1 11.6 0M9.4 16.4a4 4 0 0 1 5.2 0M12 19.5h.01"
            }
            Self::Sound => {
                "M4 10v4h3.5L12 18V6L7.5 10zM15.5 9.5a3.5 3.5 0 0 1 0 5M18 7a7 7 0 0 1 0 10"
            }
            Self::Back => "M15 6l-6 6 6 6",
            Self::SortUp => "M6 15l6-6 6 6",
            Self::SortDown => "M6 9l6 6 6-6",
            Self::Minimize => "M6 12h12",
            Self::Maximize => "M7 7h10v10H7z",
            Self::Home => "M4 11l8-7 8 7v8a1 1 0 0 1-1 1h-4v-6h-6v6H5a1 1 0 0 1-1-1z",
            Self::Desktop => "M3 5h18v11H3zM9 20h6M12 16v4",
            Self::Document => {
                "M7 3h7l5 5v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM14 3v5h5"
            }
            Self::Download => "M12 4v11M7 11l5 5 5-5M5 20h14",
            Self::Folder => {
                "M3 7.5A1.5 1.5 0 0 1 4.5 6h4.2l2 2.2h8.8A1.5 1.5 0 0 1 21 9.7v7.8A1.5 1.5 0 0 1 19.5 19h-15A1.5 1.5 0 0 1 3 17.5z"
            }
            Self::Image => "M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4",
            Self::Trash => "M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13",
            Self::Cube => "M12 3l8 4.2v9.6L12 21l-8-4.2V7.2zM4 7.2l8 4.2 8-4.2M12 11.4V21",
            Self::SystemDisk => {
                "M4 7h9M17 7h3M4 12h3M11 12h9M4 17h11M19 17h1M15 5v4M9 10v4M17 15v4"
            }
            Self::Drive => {
                "M4 6h16a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1zM7 14h.01M11 14h6"
            }
            Self::UsbKey => {
                "M9 3h6v5H9zM8 8h8v11a2 2 0 0 1-2 2h-4a2 2 0 0 1-2-2zM11 5.5h.01M13 5.5h.01"
            }
            Self::ExternalDrive => {
                "M7 3h10a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1zM12 17h.01M9.5 7h5"
            }
            Self::Card => {
                "M9 3h8a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V7zM10 7v3M13 7v3M16 7v3"
            }
            Self::Optical => {
                "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18zM12 9.5a2.5 2.5 0 1 0 0 5a2.5 2.5 0 1 0 0-5z"
            }
            Self::Eject => "M12 5l7 8H5zM5 18h14",
            Self::Lock => "M7 11h10v9H7zM9 11V8a3 3 0 0 1 6 0v3",
            Self::NetworkOff => {
                "M3 9.5a13 13 0 0 1 18 0M9.4 16.4a4 4 0 0 1 5.2 0M12 19.5h.01M4 4l16 16"
            }
            Self::SoundMuted => "M4 10v4h3.5L12 18V6L7.5 10zM16 9.5l5 5M21 9.5l-5 5",
            Self::Battery => {
                "M4.5 7h13A1.5 1.5 0 0 1 19 8.5v7a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 15.5v-7A1.5 1.5 0 0 1 4.5 7zM21.5 10.5v3"
            }
            Self::BatteryCharging => {
                "M4.5 7h13A1.5 1.5 0 0 1 19 8.5v7a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 15.5v-7A1.5 1.5 0 0 1 4.5 7zM21.5 10.5v3M12 9l-2.5 3.5h3L10 16"
            }
            Self::Microphone => {
                "M9 7a3 3 0 0 1 6 0v4a3 3 0 0 1-6 0zM6 11a6 6 0 0 0 12 0M12 17v3M9 20h6"
            }
            Self::MicrophoneMuted => {
                "M9 7a3 3 0 0 1 6 0v4a3 3 0 0 1-6 0zM6 11a6 6 0 0 0 12 0M12 17v3M9 20h6M4 4l16 16"
            }
            Self::Brightness => {
                "M12 8.5a3.5 3.5 0 1 0 0 7a3.5 3.5 0 1 0 0-7zM12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6L7 7M17 17l1.4 1.4M5.6 18.4L7 17M17 7l1.4-1.4"
            }
            Self::KeyboardBrightness => {
                "M4.5 11h15a1.5 1.5 0 0 1 1.5 1.5v6a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5v-6A1.5 1.5 0 0 1 4.5 11zM7 14h.01M10 14h.01M14 14h.01M17 14h.01M8.5 17h7M12 3.5v3M6.8 5.2l1.4 1.8M17.2 5.2l-1.4 1.8"
            }
            Self::Eye => {
                "M2.5 12s3.5-6.5 9.5-6.5S21.5 12 21.5 12s-3.5 6.5-9.5 6.5S2.5 12 2.5 12zM12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6z"
            }
            Self::EyeOff => {
                "M2.5 12s3.5-6.5 9.5-6.5S21.5 12 21.5 12s-3.5 6.5-9.5 6.5S2.5 12 2.5 12zM12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6zM4 4l16 16"
            }
            Self::ArrowRight => "M5 12h14M13 6l6 6-6 6",
            Self::Sleep => "M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z",
            Self::Restart => "M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4v4h4",
            Self::Power => "M12 3v8M7.2 6.2a7.5 7.5 0 1 0 9.6 0",
            Self::Spinner => "M12 4a8 8 0 1 1-8 8",
            // The Reglages board's.
            Self::Bluetooth => "M7 7l10 10-5 4V3l5 4L7 17",
            // A touchpad with its button line, drawn for Control Room.
            Self::Touchpad => {
                "M6 4h12a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2zM4 15h16M12 15v5"
            }
            // « Dock et barre » in the Reglages board.
            Self::Grid => "M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z",
            // A window with its sidebar: shows or hides a folded sidebar.
            Self::Sidebar => "M4 5h16v14H4zM10 5v14",
            Self::Person => "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM5 20a7 7 0 0 1 14 0",
        }
    }

    /// Full SVG document, stroked in black: the display recolors it.
    pub fn svg(self) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\" fill=\"none\" \
             stroke=\"#000\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>",
            grid = metrics::ICON_GRID,
            stroke = metrics::ICON_STROKE,
            path = self.path(),
        )
    }
}

/// What the battery gauge shows over its level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryMark {
    None,
    /// A bolt: it charges.
    Charging,
    /// A plug: plugged in, full or not charging.
    Plugged,
}

/// The inside of the battery outline, which its level fills from the left: x, y, width,
/// height on the 24 grid.
pub const BATTERY_INSIDE: (f32, f32, f32, f32) = (5.0, 9.0, 12.0, 6.0);

/// The battery of the bar, drawn rather than chosen: the outline of `Icon::Battery`, filled
/// from the left to `percent`, its mark cut out of the level and filled outside it (even-odd
/// rule), so that it shows at every level in the single color of the icon.
pub fn battery_svg(percent: u8, mark: BatteryMark) -> String {
    let (x, y, width, height) = BATTERY_INSIDE;
    let level = width * f32::from(percent.min(100)) / 100.0;
    let mut fill = if level > 0.0 {
        format!("M{x} {y}h{level:.1}v{height}H{x}z")
    } else {
        String::new()
    };
    fill.push_str(match mark {
        BatteryMark::None => "",
        BatteryMark::Charging => "M11.8 8.4L8.9 12.3h2.5l-1 3.3 3.1-4.1H11z",
        BatteryMark::Plugged => "M8 11.3h1.5v-1.3h2.6a1.3 1.3 0 0 1 1.3 1.3v.2h1.6v.9h-1.6v.2a1.3 1.3 0 0 1-1.3 1.3H9.5v-1.3H8z",
    });
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\">\
         <path d=\"{outline}\" fill=\"none\" stroke=\"#000\" stroke-width=\"{stroke}\" \
         stroke-linecap=\"round\" stroke-linejoin=\"round\"/>\
         <path d=\"{fill}\" fill=\"#000\" fill-rule=\"evenodd\"/></svg>",
        grid = metrics::ICON_GRID,
        stroke = metrics::ICON_STROKE,
        outline = Icon::Battery.path(),
    )
}

// The live icons of the bar's indicators (UI-01): drawn from their level, like the battery,
// rather than chosen among pictures.

/// Opacity of the arcs of a Wi-Fi signal or the Bluetooth rune that are not lit: the outline
/// of what could be, as on macOS.
pub const UNLIT: f32 = 0.3;

/// Levels of a Wi-Fi signal and of the sound: 0 to 3 arcs or waves.
pub const LEVELS: u8 = 3;

/// The three arcs of the Wi-Fi signal, inner first, and its dot (`Icon::Network`).
const WIFI_ARCS: [&str; 3] = [
    "M9.4 16.4a4 4 0 0 1 5.2 0",
    "M6.2 13a8.5 8.5 0 0 1 11.6 0",
    "M3 9.5a13 13 0 0 1 18 0",
];
const WIFI_DOT: &str = "M12 19.5h.01";

/// What the Wi-Fi icon shows besides its level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiMark {
    None,
    /// Connected, but NetworkManager finds no way to the Internet: an exclamation mark.
    NoInternet,
    /// No network: every arc faded and a slash across.
    Off,
}

/// A stroked SVG document with its lit paths, then its unlit ones at `UNLIT`, and plain
/// filled shapes.
fn layered(lit: &str, unlit: &str, filled: &str) -> String {
    let stroke = |d: &str, opacity: f32| {
        if d.is_empty() {
            String::new()
        } else {
            format!("<path d=\"{d}\" stroke-opacity=\"{opacity}\"/>")
        }
    };
    let filled = if filled.is_empty() {
        String::new()
    } else {
        format!("<path d=\"{filled}\" fill=\"#000\" stroke=\"none\"/>")
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\" fill=\"none\" \
         stroke=\"#000\" stroke-width=\"{width}\" stroke-linecap=\"round\" \
         stroke-linejoin=\"round\">{lit}{unlit}{filled}</svg>",
        grid = metrics::ICON_GRID,
        width = metrics::ICON_STROKE,
        lit = stroke(lit, 1.0),
        unlit = stroke(unlit, UNLIT),
    )
}

/// The level of a Wi-Fi signal from its strength (0 to 100, NetworkManager's `Strength`):
/// 0 to 3 arcs, as the bars of a phone.
pub fn wifi_level(strength: u8) -> u8 {
    match strength {
        0..=24 => 0,
        25..=49 => 1,
        50..=74 => 2,
        _ => 3,
    }
}

/// The Wi-Fi of the bar: the dot, then `level` arcs lit (0 to 3) and the others faded, and its
/// `mark`.
pub fn wifi_svg(level: u8, mark: WifiMark) -> String {
    let level = if mark == WifiMark::Off {
        0
    } else {
        usize::from(level.min(LEVELS))
    };
    let mut lit = String::from(WIFI_DOT);
    let mut unlit = String::new();
    for (i, arc) in WIFI_ARCS.iter().enumerate() {
        if i < level {
            lit.push_str(arc);
        } else {
            unlit.push_str(arc);
        }
    }
    match mark {
        WifiMark::None => {}
        // Bottom right, where no arc goes.
        WifiMark::NoInternet => lit.push_str("M20 14v3.5M20 20.5h.01"),
        WifiMark::Off => {
            lit = format!("{WIFI_DOT}M4 4l16 16");
            unlit = WIFI_ARCS.concat();
        }
    }
    layered(&lit, &unlit, "")
}

/// The speaker of `Icon::Sound` and its three waves, inner first.
const SPEAKER: &str = "M4 10v4h3.5L12 18V6L7.5 10z";
const WAVES: [&str; 3] = [
    "M15.5 9.5a3.5 3.5 0 0 1 0 5",
    "M18 7a7 7 0 0 1 0 10",
    "M20.5 4.5a10.5 10.5 0 0 1 0 15",
];

/// The level of the sound from its volume in percent: no wave at 0, then one to three.
pub fn sound_level(volume: u32) -> u8 {
    match volume {
        0 => 0,
        1..=33 => 1,
        34..=66 => 2,
        _ => 3,
    }
}

/// The sound of the bar: the speaker and `level` waves (0 to 3), or the speaker crossed out
/// when it is muted (`Icon::SoundMuted`).
pub fn sound_svg(level: u8, muted: bool) -> String {
    if muted {
        return layered(Icon::SoundMuted.path(), "", "");
    }
    let waves: String = WAVES
        .iter()
        .take(usize::from(level.min(LEVELS)))
        .copied()
        .collect();
    layered(&format!("{SPEAKER}{waves}"), "", "")
}

/// The Bluetooth rune.
const RUNE: &str = "M7 7l10 10-5 4V3l5 4L7 17";

/// What the Bluetooth radio is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radio {
    Off,
    On,
    /// On, with a device connected: a dot on the right of the rune.
    Connected,
}

/// The Bluetooth of the bar: the rune faded when off, lit when on, and a dot beside it when a
/// device is connected.
pub fn bluetooth_svg(radio: Radio) -> String {
    match radio {
        Radio::Off => layered("", RUNE, ""),
        Radio::On => layered(RUNE, "", ""),
        Radio::Connected => layered(
            RUNE,
            "",
            "M21.8 12a1.8 1.8 0 1 1-3.6 0a1.8 1.8 0 1 1 3.6 0z",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How many of `paths` the document draws at full strength.
    fn lit(svg: &str, paths: &[&str]) -> usize {
        let full = svg
            .split("<path d=\"")
            .skip(1)
            .filter_map(|chunk| chunk.split_once('"'))
            .find(|(_, rest)| rest.starts_with(" stroke-opacity=\"1\""))
            .map_or("", |(d, _)| d);
        paths.iter().filter(|p| full.contains(**p)).count()
    }

    #[test]
    fn the_wifi_lights_one_arc_per_level() {
        for level in 0..=LEVELS {
            let svg = wifi_svg(level, WifiMark::None);
            assert_eq!(lit(&svg, &WIFI_ARCS), usize::from(level), "level {level}");
            assert!(svg.contains(WIFI_DOT));
            // The arcs not lit are still drawn, faded.
            assert_eq!(svg.contains("stroke-opacity=\"0.3\""), level < LEVELS);
        }
        assert_eq!(wifi_svg(9, WifiMark::None), wifi_svg(3, WifiMark::None));
        let none = wifi_svg(2, WifiMark::NoInternet);
        assert!(none.contains("M20 14v3.5") && lit(&none, &WIFI_ARCS) == 2);
        let off = wifi_svg(3, WifiMark::Off);
        assert!(off.contains("M4 4l16 16") && lit(&off, &WIFI_ARCS) == 0);
        assert_eq!(
            [0, 24, 25, 49, 50, 74, 75, 100].map(wifi_level),
            [0, 0, 1, 1, 2, 2, 3, 3]
        );
    }

    #[test]
    fn the_sound_draws_one_wave_per_level_and_crosses_out_when_muted() {
        for level in 0..=LEVELS {
            let svg = sound_svg(level, false);
            let waves = WAVES.iter().filter(|w| svg.contains(**w)).count();
            assert_eq!(waves, usize::from(level), "level {level}");
            assert!(svg.contains(SPEAKER));
        }
        assert!(sound_svg(2, true).contains(Icon::SoundMuted.path()));
        assert!(!sound_svg(2, true).contains(WAVES[0]));
        assert_eq!(
            [0, 1, 33, 34, 66, 67, 150].map(sound_level),
            [0, 1, 1, 2, 2, 3, 3]
        );
    }

    #[test]
    fn the_bluetooth_is_faded_off_lit_on_and_dotted_when_connected() {
        let off = bluetooth_svg(Radio::Off);
        assert!(off.contains("stroke-opacity=\"0.3\"") && !off.contains("fill=\"#000\""));
        let on = bluetooth_svg(Radio::On);
        assert!(on.contains("stroke-opacity=\"1\"") && !on.contains("fill=\"#000\""));
        assert!(bluetooth_svg(Radio::Connected).contains("fill=\"#000\""));
    }

    #[test]
    fn the_live_icons_stay_in_the_grid() {
        let all = WIFI_ARCS.iter().chain(&WAVES).chain(&[
            RUNE,
            SPEAKER,
            WIFI_DOT,
            "M20 14v3.5M20 20.5h.01",
            "M4 4l16 16",
        ]);
        for path in all {
            let numbers = path
                .split(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == ',')
                .flat_map(|chunk| chunk.split_inclusive('-'))
                .filter_map(|n| n.trim_end_matches('-').parse::<f32>().ok());
            for n in numbers {
                assert!(n.abs() <= metrics::ICON_GRID, "{path}: {n}");
            }
        }
    }

    #[test]
    fn the_battery_fills_with_its_level_and_shows_its_mark() {
        let level = |svg: &str| {
            svg.split("M5 9h")
                .nth(1)
                .and_then(|rest| rest.split('v').next())
                .and_then(|n| n.parse::<f32>().ok())
        };
        assert_eq!(level(&battery_svg(5, BatteryMark::None)), Some(0.6));
        assert_eq!(level(&battery_svg(50, BatteryMark::None)), Some(6.0));
        assert_eq!(level(&battery_svg(100, BatteryMark::Plugged)), Some(12.0));
        assert_eq!(level(&battery_svg(0, BatteryMark::None)), None);
        let charging = battery_svg(54, BatteryMark::Charging);
        assert!(charging.contains("M11.8 8.4") && charging.contains("evenodd"));
        assert!(!battery_svg(54, BatteryMark::None).contains("M11.8"));
        assert!(battery_svg(100, BatteryMark::Plugged).contains("M8 11.3"));
    }

    #[test]
    fn every_icon_stays_in_the_grid() {
        for icon in Icon::ALL {
            let numbers = icon
                .path()
                .split(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == ',')
                .flat_map(|chunk| chunk.split_inclusive('-'))
                .filter_map(|n| n.trim_end_matches('-').parse::<f32>().ok());
            for n in numbers {
                assert!(
                    n.abs() <= metrics::ICON_GRID,
                    "{icon:?} leaves the grid: {n}"
                );
            }
        }
    }

    #[test]
    fn the_frame_carries_the_stroke_and_the_rounding() {
        let svg = Icon::Check.svg();
        assert!(svg.contains("stroke-width=\"1.7\""));
        assert!(svg.contains("stroke-linecap=\"round\""));
        assert!(svg.contains("viewBox=\"0 0 24 24\""));
    }
}
