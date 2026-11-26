//! The session lock (lot D-25a, ADR 0023). The lock screen itself is a program of its own,
//! `sherlockos-lock`; what lives here is what the compositor draws when no lock program shows
//! anything: the plain background of the system screens, and, once every lock program failed,
//! the rescue screen, two pills of glass in the middle of each screen. No board draws it: the
//! measures below are choices of the lot, taken from the Mission Control labels. See "Session
//! lock" in `docs/compositor.md`.

use crate::color::{Color, Palette};
use crate::mission_control;

/// What the rescue screen says.
pub const RESCUE_TITLE: &str = "Verrouillé";
pub const RESCUE_HINT: &str = "Appuie sur Entrée pour réessayer";

/// Height of a pill of the rescue screen, and the gap between the two.
pub const RESCUE_LABEL_HEIGHT: f32 = mission_control::LABEL_HEIGHT;
pub const RESCUE_GAP: f32 = 12.0;

/// The colors of the lock in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Behind the lock surfaces, and alone on a screen without one.
    pub background: Color,
    pub label: Color,
    pub label_text: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            background: palette.system_screen,
            label: palette.menu_glass,
            label_text: palette.text_primary,
        }
    }
}

/// `#RRGGBB` of a color without its `#`, as `swaylock --color` takes it.
pub fn hex(color: Color) -> String {
    format!("{:02x}{:02x}{:02x}", color.r, color.g, color.b)
}

/// The lock screen, `sherlockos-lock` (lot D-25a): macOS-like, as the author asked (no board
/// draws it, the Connexion board inspires it). The time and the date at the top, then at the
/// bottom middle the bubble of Sherlock, the name of the user and the password pill, and the
/// machine buttons under them. Every size is in logical pixels and follows the screen: the
/// `fit` helpers keep them between a minimum and a maximum, from a 1366 × 768 laptop to a 4K
/// screen at scale 2. See `docs/lock.md`.
pub mod screen {
    use std::time::Duration;

    use crate::color::{Color, Palette};
    use crate::motion::{self, Spring};

    /// A length proportional to the screen's height, kept between `min` and `max`.
    pub fn fit(height: f32, ratio: f32, min: f32, max: f32) -> f32 {
        (height * ratio).clamp(min, max)
    }

    /// The time: size and room above it, from the screen's height.
    pub fn clock_size(height: f32) -> f32 {
        fit(height, 0.11, 64.0, 132.0)
    }
    pub fn top(height: f32) -> f32 {
        fit(height, 0.08, 28.0, 112.0)
    }
    /// The date, above the time.
    pub const DATE_SIZE: f32 = 20.0;
    /// Weight of the time: thin, as on macOS.
    pub const CLOCK_WEIGHT: u16 = 250;
    pub const DATE_WEIGHT: u16 = 500;

    /// The bubble of Sherlock: its diameter, the gap under it, the name under that.
    pub fn bubble(height: f32) -> f32 {
        fit(height, 0.1, 64.0, 112.0)
    }
    pub const BUBBLE_GAP: f32 = 14.0;
    pub const NAME_SIZE: f32 = 17.0;
    pub const NAME_GAP: f32 = 14.0;

    /// The password pill: width, height (fully round ends), the room of each icon button
    /// inside it, and the size of its icons.
    pub const FIELD_WIDTH: f32 = 260.0;
    pub const FIELD_HEIGHT: f32 = 36.0;
    pub const FIELD_RADIUS: f32 = FIELD_HEIGHT / 2.0;
    pub const FIELD_BUTTON: f32 = 28.0;
    pub const FIELD_ICON: f32 = 16.0;
    pub const FIELD_TEXT_SIZE: f32 = 14.0;
    /// The line under the pill: the error, or the caps lock.
    pub const HINT_SIZE: f32 = 13.0;
    pub const HINT_GAP: f32 = 10.0;

    /// The machine buttons: a round glass button and its label, side by side.
    pub const MACHINE_BUTTON: f32 = 40.0;
    pub const MACHINE_ICON: f32 = 18.0;
    pub const MACHINE_LABEL_SIZE: f32 = 12.0;
    pub const MACHINE_GAP: f32 = 28.0;
    /// Room under the machine buttons, and between the pill and them.
    pub fn bottom(height: f32) -> f32 {
        fit(height, 0.05, 20.0, 72.0)
    }
    pub fn machine_gap(height: f32) -> f32 {
        fit(height, 0.07, 24.0, 96.0)
    }

    /// Entrance: each element rises by `RISE` and fades in, `STAGGER` after the one before,
    /// on the spring of a window that opens. The first key ends every animation at once.
    pub const RISE: f32 = 16.0;
    pub const STAGGER: Duration = Duration::from_millis(70);
    pub const SPRING: Spring = motion::OPEN_SPRING;
    /// Exit, once unlocked: everything fades out, in reverse order.
    pub const EXIT: Duration = Duration::from_millis(220);
    /// A wrong password: the pill shakes three times, then empties.
    pub const SHAKE: Duration = Duration::from_millis(450);
    pub const SHAKES: f32 = 3.0;
    pub const SHAKE_AMPLITUDE: f32 = 12.0;
    /// One turn of the spinner.
    pub const SPIN: Duration = motion::SPIN;

    /// The colors of the screen in a palette.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Colors {
        pub background: Color,
        pub text: Color,
        pub secondary: Color,
        pub field: Color,
        pub field_border: Color,
        pub placeholder: Color,
        pub bubble: Color,
        pub button: Color,
        pub button_hover: Color,
        pub armed: Color,
        pub error: Color,
    }

    impl Colors {
        pub const fn of(palette: &Palette) -> Self {
            Self {
                background: palette.system_screen,
                text: palette.text_primary,
                secondary: palette.text_secondary,
                field: palette.glass,
                field_border: palette.border,
                placeholder: palette.text_tertiary,
                bubble: palette.cream,
                button: palette.glass,
                button_hover: palette.hover,
                armed: palette.amber_soft,
                error: palette.salmon,
            }
        }
    }

    /// The texts of the screen.
    pub const PASSWORD: &str = "Mot de passe";
    pub const SHOW: &str = "Afficher le mot de passe";
    pub const HIDE: &str = "Masquer le mot de passe";
    pub const UNLOCK: &str = "Déverrouiller";
    pub const WRONG: &str = "Mot de passe incorrect";
    pub const TOO_MANY: &str = "Trop d'essais. Patiente un peu avant de réessayer.";
    pub const UNAVAILABLE: &str = "Vérification impossible. Réessaie.";
    pub const CAPS_LOCK: &str = "Verr. Maj activé";
    pub const SLEEP: &str = "Veille";
    pub const REBOOT: &str = "Redémarrer";
    pub const POWER_OFF: &str = "Éteindre";
    pub const CONFIRM: &str = "Reclique pour confirmer";

    /// Days of the week, Monday to Sunday, and months, for the date of the screen.
    const WEEKDAYS: [&str; 7] = [
        "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
    ];
    const MONTHS: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];

    /// « jeudi 26 novembre »: `weekday` from 1 (Monday) to 7, `month` from 1 to 12.
    pub fn date(weekday: u8, day: u8, month: u8) -> Option<String> {
        let weekday = WEEKDAYS.get(usize::from(weekday).checked_sub(1)?)?;
        let month = MONTHS.get(usize::from(month).checked_sub(1)?)?;
        Some(format!("{weekday} {day} {month}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_background_is_the_one_of_the_system_screens() {
        let colors = Colors::of(&Palette::DARK);
        assert_eq!(colors.background, Palette::DARK.system_screen);
        assert_eq!(hex(Palette::DARK.system_screen), "0a1424");
    }

    #[test]
    fn the_date_is_written_in_full() {
        assert_eq!(
            screen::date(4, 26, 11).as_deref(),
            Some("jeudi 26 novembre")
        );
        assert_eq!(screen::date(0, 1, 1), None);
        assert_eq!(screen::date(7, 1, 13), None);
    }

    #[test]
    fn the_sizes_follow_the_screen_within_bounds() {
        // 1366 × 768 at 1, 1920 × 1080 at 1, 2560 × 1600 at 1.5, 3840 × 2160 at 2: logical
        // heights of 768, 1080, 1067 and 1080.
        for height in [768.0, 1080.0, 1067.0, 2160.0] {
            let clock = screen::clock_size(height);
            assert!((64.0..=132.0).contains(&clock));
            let used = screen::top(height)
                + screen::DATE_SIZE
                + clock
                + screen::bubble(height)
                + screen::BUBBLE_GAP
                + screen::NAME_SIZE
                + screen::NAME_GAP
                + screen::FIELD_HEIGHT
                + screen::machine_gap(height)
                + screen::MACHINE_BUTTON
                + screen::bottom(height);
            assert!(used < height * 0.8, "{height}: {used}");
        }
    }
}
