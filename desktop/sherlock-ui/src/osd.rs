//! The on-screen display of the media keys (lot D-35): a pill of glass in the bottom middle of
//! the screen, above the dock, with an icon and a gauge. The compositor draws it. No board draws
//! it: the measures below are choices of the lot, taken from the panel and dock tokens, to
//! review with the design. See `docs/media-keys.md`.

use std::time::Duration;

use crate::color::{Color, Palette};
use crate::icon::Icon;
use crate::motion::{self, Ease, Spring};
use crate::{dock, metrics};

/// Size of the pill on a screen 1920 logical pixels wide, and its radius: fully round ends.
/// Elsewhere the pill and its content take the same share of the screen's width, within
/// `WIDTH_MIN` and `WIDTH_MAX` (`fit`, lot D-35d).
pub const WIDTH: f32 = 232.0;
pub const HEIGHT: f32 = 48.0;
pub const RADIUS: f32 = HEIGHT / 2.0;
/// Share of the screen's width the pill takes, and its bounds, in logical pixels.
pub const WIDTH_SHARE: f32 = WIDTH / 1920.0;
pub const WIDTH_MIN: f32 = 200.0;
pub const WIDTH_MAX: f32 = 300.0;
/// Room the dock takes at the bottom of the screen when it reserves none (it hides, or it is
/// on a side): its margin and its height.
pub const DOCK_BAND: f32 = dock::MARGIN_BOTTOM + metrics::DOCK_HEIGHT;
/// Gap between the top of the dock and the bottom of the pill, at the pill's scale.
pub const DOCK_GAP: f32 = 40.0;
/// From the bottom edge of the screen to the bottom of the pill on a screen 1920 wide.
pub const BOTTOM: f32 = DOCK_BAND + DOCK_GAP;
/// The pill never goes higher than this share of the screen's height.
pub const BOTTOM_MAX_SHARE: f32 = 0.4;
/// Space between the edges of the pill and its content, and between the icon and the gauge.
pub const PADDING: f32 = 18.0;
pub const GAP: f32 = 14.0;
/// Size of the icon.
pub const ICON_SIZE: f32 = 22.0;
/// Height of the gauge; its ends are round.
pub const GAUGE_HEIGHT: f32 = 6.0;
/// How long the pill stays after the last key press, before it goes.
pub const VISIBLE: Duration = Duration::from_millis(1500);
/// Coming: it grows from `SCALE` to 1 around its center and fades in, as a window opens on
/// macOS. Going: the other way, on the curve of a window that closes.
pub const SCALE: f64 = motion::WINDOW_OPEN_SCALE;
pub const ENTER: Duration = motion::WINDOW_OPEN;
pub const ENTER_EASE: Ease = motion::WINDOW_OPEN_EASE;
pub const LEAVE: Duration = motion::WINDOW_CLOSE;
pub const LEAVE_EASE: Ease = motion::WINDOW_CLOSE_EASE;
/// The gauge slides to a new level on a spring, quicker than a window.
pub const GAUGE_SPRING: Spring = motion::PREVIEW_SPRING;
/// A key that insists at the end (« plus » at the top, « moins » at the bottom) stretches the
/// pill towards its side by this share of its length, as on macOS and iOS; it gets thinner by
/// `STRETCH_THIN` of that share meanwhile. The stretch comes on `STRETCH_SPRING`, is let go
/// after `STRETCH_HOLD` and goes back on the same spring, a little past its rest. Each press
/// aims at the same stretch again: presses that follow each other never add up.
pub const STRETCH: f64 = 0.05;
pub const STRETCH_THIN: f64 = 0.4;
pub const STRETCH_SPRING: Spring = motion::PREVIEW_SPRING;
pub const STRETCH_HOLD: Duration = Duration::from_millis(120);

/// The scale of the pill and of its content on a screen `width` logical pixels wide: 1 on a
/// screen 1920 wide.
pub fn fit(width: f32) -> f32 {
    let room = (width - 2.0 * PADDING).max(1.0);
    (width * WIDTH_SHARE)
        .clamp(WIDTH_MIN, WIDTH_MAX)
        .min(room)
        / WIDTH
}

/// From the bottom edge of a screen `height` logical pixels high to the bottom of the pill at
/// scale `k`, above a dock that reserves `reserved` pixels at the bottom (0 when it reserves
/// none).
pub fn bottom(height: f32, reserved: f32, k: f32) -> f32 {
    (DOCK_BAND.max(reserved) + DOCK_GAP * k).min(height * BOTTOM_MAX_SHARE)
}

// The content fits the pill, which stays above the dock.
const _: () = {
    assert!(WIDTH - 2.0 * PADDING - ICON_SIZE - GAP > WIDTH / 2.0);
    assert!(ICON_SIZE < HEIGHT - 2.0 * GAUGE_HEIGHT);
    assert!(BOTTOM > metrics::DOCK_HEIGHT);
    assert!(WIDTH_MIN < WIDTH && WIDTH < WIDTH_MAX);
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
    /// The keyboard backlight.
    Keyboard,
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
            (Self::Keyboard, _) => Icon::KeyboardBrightness,
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
        assert_eq!(Kind::Keyboard.icon(false), Icon::KeyboardBrightness);
    }

    #[test]
    fn the_pill_follows_the_screen_within_bounds() {
        assert!((fit(1920.0) - 1.0).abs() < 1e-6);
        // A small laptop: the lower bound; a wide screen: the upper bound.
        assert!((fit(1366.0) * WIDTH - WIDTH_MIN).abs() < 1e-3);
        assert!((fit(3840.0) * WIDTH - WIDTH_MAX).abs() < 1e-3);
        // A screen narrower than the pill: it still fits.
        assert!(fit(150.0) * WIDTH < 150.0);
        // Above the dock, never higher than its share of the screen.
        assert!((bottom(1080.0, 0.0, 1.0) - BOTTOM).abs() < 1e-6);
        assert!((bottom(1080.0, 120.0, 1.0) - 160.0).abs() < 1e-6);
        assert!((bottom(200.0, 0.0, 1.0) - 80.0).abs() < 1e-6);
    }
}
