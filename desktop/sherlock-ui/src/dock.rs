//! Dock (batch D-21): the tokens, styles and texts the dock needs beyond `metrics::DOCK_*`.
//! Values from the Dock design board, `docs/design/planches/Dock.dc.html`, and from the dock
//! menu of the Menus board.

use crate::color::{self, Color};
use crate::motion::Spring;

// Geometry, in logical pixels.

/// Inner padding of the dock bar: sides, bottom. The icons sit on the bottom padding and grow
/// upwards, past the top of the bar.
pub const PADDING: (f32, f32) = (12.0, 12.0);
/// Gap between the bottom of the screen and the bar (Main board: the dock frame has a bottom
/// padding of 8).
pub const MARGIN_BOTTOM: f32 = 8.0;
/// Dot below an open application: diameter, and distance from the icon bottom to the dot
/// bottom (`bottom: -9px` on the board).
pub const DOT: f32 = 5.0;
pub const DOT_OFFSET: f32 = 9.0;
/// Label above the hovered icon: distance from the bar bottom to the label bottom, vertical and
/// horizontal padding, radius, text size.
pub const LABEL_BOTTOM: f32 = 108.0;
pub const LABEL_PADDING: (f32, f32) = (3.0, 10.0);
pub const LABEL_RADIUS: f32 = 8.0;
pub const LABEL_SIZE: f32 = 12.0;
/// Room left on each side of the widest magnified dock, so that the label of an icon at the
/// edge still fits in the surface.
pub const SURFACE_SLACK: f32 = 52.0;
/// The Sherlock logo in its tile, as a share of the tile: the ring of the logo in its square.
pub const SHERLOCK_GLYPH_RATIO: f32 = 0.62;
/// Initial of an application without icon, as a share of the tile.
pub const LETTER_RATIO: f32 = 0.42;

/// Dock menu (Menus board): width, gap above the bar, height of the header with the
/// application name.
pub const MENU_WIDTH: f32 = 240.0;
pub const MENU_GAP: f32 = 12.0;
pub const MENU_HEADER_HEIGHT: f32 = 28.0;

// Motion.

/// Magnification of the icons. The board uses `cubic-bezier(0.2, 0.9, 0.3, 1.25)` over 150 ms:
/// a peak of about 8 % past the target near 97 ms. This spring (damping ratio 0.63) peaks at
/// the same time with the same overshoot and is within 2 % of its target at 150 ms.
pub const SPRING: Spring = Spring::new(1740.0, 52.6);
/// Fade of the label (`opacity 120ms` on the board).
pub const LABEL_FADE_MS: u64 = 120;

// Colors.

/// Label above the hovered icon, and its text.
pub const LABEL: Color = color::CREAM;
pub const LABEL_TEXT: Color = color::TEXT_ON_CREAM;
/// Dot below an open application. The board draws no active application: the active one takes
/// the green of the active state.
pub const DOT_OPEN: Color = color::CREAM;
pub const DOT_ACTIVE: Color = color::ACCENT;
/// Tile of the Sherlock icon: the night blue square of the color logo.
pub const SHERLOCK_TILE: Color = color::DESKTOP;
/// Tile of an application whose icon is not found, and its initial (Fichiers tile of the board).
pub const FALLBACK_TILE: Color = color::BLUE_DARK;
pub const FALLBACK_LETTER: Color = color::CREAM;

/// Dock texts, in French.
pub mod text {
    /// Name of the Sherlock icon, in its label and menu.
    pub const SHERLOCK: &str = "Sherlock";
    pub const OPEN: &str = "Ouvrir";
    pub const NEW_WINDOW: &str = "Nouvelle fenêtre";
    pub const UNPIN: &str = "Retirer du dock";
    pub const QUIT: &str = "Quitter";

    /// Number of open windows, at the top right of the menu, like « 3 en marche » on the board.
    pub fn windows(count: usize) -> String {
        match count {
            1 => String::from("1 fenêtre"),
            n => format!("{n} fenêtres"),
        }
    }
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::container;
    use iced::{Background, Border, Theme, border};

    use super::*;
    use crate::metrics;

    fn fill(c: Color, edge: Border) -> container::Style {
        container::Style {
            background: Some(Background::Color(c.into())),
            border: edge,
            ..container::Style::default()
        }
    }

    /// Dock bar: dock glass, its border, radius 22. Blur and shadow are the compositor's job.
    pub fn bar(_: &Theme) -> container::Style {
        let edge = border::width(metrics::BORDER_WIDTH)
            .color(color::DOCK_BORDER)
            .rounded(metrics::DOCK_RADIUS);
        fill(color::DOCK_GLASS, edge)
    }

    /// Tile behind an icon drawn by the dock, at the icon radius (27 % of its side).
    pub fn tile(tint: Color, side: f32) -> impl Fn(&Theme) -> container::Style {
        move |_| {
            fill(
                tint,
                border::rounded(side * metrics::DOCK_ICON_RADIUS_RATIO),
            )
        }
    }

    /// Label above the hovered icon, faded by `opacity`.
    pub fn label(opacity: f32) -> impl Fn(&Theme) -> container::Style {
        move |_| {
            let tint = Color {
                a: LABEL.a * opacity,
                ..LABEL
            };
            container::Style {
                text_color: Some(
                    Color {
                        a: opacity,
                        ..LABEL_TEXT
                    }
                    .into(),
                ),
                ..fill(tint, border::rounded(LABEL_RADIUS))
            }
        }
    }

    /// Dot below an open application.
    pub fn dot(active: bool) -> impl Fn(&Theme) -> container::Style {
        let tint = if active { DOT_ACTIVE } else { DOT_OPEN };
        move |_| fill(tint, border::rounded(DOT / 2.0))
    }

    /// Keyboard focus ring around an icon of `side`.
    pub fn ring(side: f32) -> impl Fn(&Theme) -> container::Style {
        move |_| container::Style {
            border: border::width(metrics::FOCUS_RING_WIDTH)
                .color(color::FOCUS_RING)
                .rounded(side * metrics::DOCK_ICON_RADIUS_RATIO + metrics::FOCUS_RING_GAP),
            ..container::Style::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::Animation;
    use std::time::Duration;

    #[test]
    fn the_spring_follows_the_board_transition() {
        let animation = Animation::new(SPRING, 0.0, 1.0, Duration::ZERO);
        let values: Vec<f64> = (0..=400)
            .map(|t| animation.value(Duration::from_millis(t)))
            .collect();
        let (peak_at, peak) = values
            .iter()
            .copied()
            .enumerate()
            .fold(
                (0, 0.0),
                |best, (t, v)| if v > best.1 { (t, v) } else { best },
            );
        // The board's cubic-bezier peaks near 97 ms, about 7.8 % past the target.
        assert!((90..=105).contains(&peak_at), "peak at {peak_at} ms");
        assert!(peak > 1.06 && peak < 1.09, "peak of {peak}");
        // Within 2 % at the 150 ms of the transition, settled to 1 % soon after.
        assert!((values[150] - 1.0).abs() < 0.02, "{}", values[150]);
        assert!(values[200..].iter().all(|v| (v - 1.0).abs() < 0.01));
    }

    #[test]
    fn the_window_count_agrees_in_number() {
        assert_eq!(text::windows(1), "1 fenêtre");
        assert_eq!(text::windows(3), "3 fenêtres");
    }

    #[test]
    fn the_label_reads_on_its_cream() {
        assert!(LABEL_TEXT.contrast(LABEL) >= 4.5);
        assert!(FALLBACK_LETTER.contrast(FALLBACK_TILE) >= 4.5);
    }
}
