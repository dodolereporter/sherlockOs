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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_background_is_the_one_of_the_system_screens() {
        let colors = Colors::of(&Palette::DARK);
        assert_eq!(colors.background, Palette::DARK.system_screen);
        assert_eq!(hex(Palette::DARK.system_screen), "0a1424");
    }
}
