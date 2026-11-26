//! The on-screen display of the media keys (lot D-35): a pill of glass in the bottom middle of
//! the screen, above the dock, with an icon and a gauge. The compositor draws it. No board draws
//! it: the measures below are choices of the lot, taken from the panel and dock tokens, to
//! review with the design. See `docs/media-keys.md`.

use std::time::Duration;

use crate::color::{Color, Palette};
use crate::icon::Icon;
use crate::motion::{self, Spring};
use crate::{dock, metrics};

/// Size of the pill, in logical pixels, and its radius: fully round ends.
pub const WIDTH: f32 = 232.0;
pub const HEIGHT: f32 = 48.0;
pub const RADIUS: f32 = HEIGHT / 2.0;
/// From the bottom edge of the screen to the bottom of the pill: the dock and its margin, then
/// a gap of 40.
pub const BOTTOM: f32 = dock::MARGIN_BOTTOM + metrics::DOCK_HEIGHT + 40.0;
/// Space between the edges of the pill and its content, and between the icon and the gauge.
pub const PADDING: f32 = 18.0;
pub const GAP: f32 = 14.0;
/// Size of the icon.
pub const ICON_SIZE: f32 = 22.0;
/// Height of the gauge; its ends are round.
pub const GAUGE_HEIGHT: f32 = 6.0;
/// How long the pill stays after the last key press, before it goes.
pub const VISIBLE: Duration = Duration::from_millis(1500);
/// Coming and going: opacity and a small rise, on the spring of a window that opens.
pub const SPRING: Spring = motion::OPEN_SPRING;
/// How far below its place the pill comes from, and goes to.
pub const RISE: f32 = 10.0;
/// The gauge follows the level on a spring too, quicker.
pub const GAUGE_SPRING: Spring = motion::PREVIEW_SPRING;

// The content fits the pill, which stays above the dock.
const _: () = {
    assert!(WIDTH - 2.0 * PADDING - ICON_SIZE - GAP > WIDTH / 2.0);
    assert!(ICON_SIZE < HEIGHT - 2.0 * GAUGE_HEIGHT);
    assert!(BOTTOM > metrics::DOCK_HEIGHT);
};

/// What the pill of the microphone says, in place of a gauge.
pub const MICROPHONE_ON: &str = "Micro activé";
pub const MICROPHONE_OFF: &str = "Micro coupé";

/// What the pill shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Volume,
    Microphone,
    Brightness,
}

impl Kind {
    /// The icon of the level: muted shows the muted icon.
    pub const fn icon(self, muted: bool) -> Icon {
        match (self, muted) {
            (Self::Volume, false) => Icon::Sound,
            (Self::Volume, true) => Icon::SoundMuted,
            (Self::Microphone, false) => Icon::Microphone,
            (Self::Microphone, true) => Icon::MicrophoneMuted,
            (Self::Brightness, _) => Icon::Brightness,
        }
    }

    /// The microphone has no gauge: only whether it listens.
    pub const fn has_gauge(self) -> bool {
        !matches!(self, Self::Microphone)
    }
}

/// The colors of the pill in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub glass: Color,
    pub border: Color,
    pub icon: Color,
    /// The empty part of the gauge.
    pub track: Color,
    /// The level.
    pub fill: Color,
    /// The level of a muted output.
    pub fill_muted: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            glass: palette.menu_glass,
            border: palette.menu_border,
            icon: palette.text_primary,
            track: palette.toggle_off,
            fill: palette.accent,
            fill_muted: palette.text_tertiary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_follows_the_level() {
        assert_eq!(Kind::Volume.icon(true), Icon::SoundMuted);
        assert_eq!(Kind::Microphone.icon(false), Icon::Microphone);
        assert!(!Kind::Microphone.has_gauge());
        assert!(Kind::Brightness.has_gauge());
    }
}
