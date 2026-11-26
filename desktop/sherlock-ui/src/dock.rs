//! Dock (batch D-21): the tokens, styles and texts the dock needs beyond `metrics::DOCK_*`.
//! Values from the Dock design board, `docs/design/planches/Dock.dc.html`, and from the dock
//! menu of the Menus board.

use std::time::Duration;

use crate::color::{Color, Palette};
use crate::motion::{self, Spring};

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
/// Label next to the hovered icon: gap past the tallest magnified icon (the board puts it 108
/// above the bar bottom, 7 past an icon of 52 × 1.7 on the padding of 13), vertical and
/// horizontal padding, radius, text size.
pub const LABEL_GAP: f32 = 7.0;
pub const LABEL_PADDING: (f32, f32) = (3.0, 10.0);
pub const LABEL_RADIUS: f32 = 8.0;
pub const LABEL_SIZE: f32 = 12.0;
/// Room for the label beside the icons of a dock on the left or right edge.
pub const LABEL_SIDE_ROOM: f32 = 200.0;
/// Separator before the open applications that are not pinned (board, before « Réglages »): its
/// thickness, its length across the bar, its margin on each side.
pub const SEPARATOR: (f32, f32) = (1.0, 40.0);
pub const SEPARATOR_MARGIN: f32 = 2.0;
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

// Settings (`docs/dock.md`): bounds of what the configuration may ask, defaults in `metrics`.

/// Side of an icon at rest. A choice of D-21: from a small 32 to a large 96 on the 1440 × 900
/// reference screen.
pub const ICON_SIZE_MIN: f32 = 32.0;
pub const ICON_SIZE_MAX: f32 = 96.0;
/// Magnification factor, 1 for none.
pub const MAGNIFICATION_MIN: f32 = 1.0;
pub const MAGNIFICATION_MAX: f32 = 2.0;

const _: () = {
    assert!(
        ICON_SIZE_MIN <= crate::metrics::DOCK_ICON && crate::metrics::DOCK_ICON <= ICON_SIZE_MAX
    );
    assert!(MAGNIFICATION_MIN <= crate::metrics::DOCK_MAGNIFICATION);
    assert!(crate::metrics::DOCK_MAGNIFICATION <= MAGNIFICATION_MAX);
};

// Motion.

/// Magnification of the icons. The board uses `cubic-bezier(0.2, 0.9, 0.3, 1.25)` over 150 ms:
/// a peak of about 8 % past the target near 97 ms. This spring (damping ratio 0.63) peaks at
/// the same time with the same overshoot and is within 2 % of its target at 150 ms.
pub const SPRING: Spring = Spring::new(1740.0, 52.6);
/// Fade of the label (`opacity 120ms` on the board).
pub const LABEL_FADE_MS: u64 = 120;

// Motion of batch D-21b. The board shows none of it: these follow macOS, with the springs and
// the fade of `motion`.

/// An icon arriving or leaving: its room in the bar and its scale, from 0 to 1 or back (the
/// spring of a window opening, overshoot of 2 %).
pub const ARRIVAL_SPRING: Spring = motion::OPEN_SPRING;
/// Fade of an icon arriving or leaving: the fade of a window opening.
pub const ICON_FADE: Duration = motion::FADE;
/// Icons moving aside or closing up, and a dropped icon flying to its place (the spring of a
/// window snapping, overshoot of 3 %).
pub const SHIFT_SPRING: Spring = motion::SNAP_SPRING;

/// Bounce of an icon while its application starts: height of a hop, as a share of the icon
/// side, and its duration. A choice of D-21b, close to macOS.
pub const BOUNCE_HEIGHT: f32 = 0.5;
pub const BOUNCE_PERIOD: Duration = Duration::from_millis(600);
/// The icon stops bouncing after this long even when no window of the application came.
pub const BOUNCE_TIMEOUT: Duration = Duration::from_secs(10);

/// Distance the pointer travels with the button held before the icon follows it: under it, a
/// press and a release make a click.
pub const DRAG_THRESHOLD: f32 = 4.0;
/// Opacity of an icon dragged out of the dock, where letting it go removes it.
pub const DRAG_OUT_OPACITY: f32 = 0.5;

// Colors. The dock bar, its label, dots and separator take the palette shown
// (`theme::palette`); the tiles keep their colors in both themes (`docs/design/THEME-CLAIR.md`).

/// Tile of the Sherlock icon: the night blue square of the color logo.
pub const SHERLOCK_TILE: Color = Palette::DARK.desktop;
/// Tile of an application whose icon is not found, and its initial (the « Fichiers » tile of the board).
pub const FALLBACK_TILE: Color = Palette::DARK.blue_dark;
pub const FALLBACK_LETTER: Color = Palette::DARK.cream;

/// Dock texts, in French.
pub mod text {
    /// Name of the Sherlock icon, in its label and menu.
    pub const SHERLOCK: &str = "Sherlock";
    pub const OPEN: &str = "Ouvrir";
    pub const NEW_WINDOW: &str = "Nouvelle fenêtre";
    pub const KEEP: &str = "Garder dans le dock";
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
    use crate::theme::palette;

    fn fill(c: Color, edge: Border) -> container::Style {
        container::Style {
            background: Some(Background::Color(c.into())),
            border: edge,
            ..container::Style::default()
        }
    }

    /// Dock bar: dock glass, its border, radius 22. Blur and shadow are the compositor's job.
    pub fn bar(theme: &Theme) -> container::Style {
        let p = palette(theme);
        let edge = border::width(metrics::BORDER_WIDTH)
            .color(p.dock_border)
            .rounded(metrics::DOCK_RADIUS);
        fill(p.dock_glass, edge)
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

    /// Label above the hovered icon, cream like Sherlock's bubbles, faded by `opacity`.
    pub fn label(opacity: f32) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let p = palette(theme);
            let tint = Color {
                a: p.cream.a * opacity,
                ..p.cream
            };
            container::Style {
                text_color: Some(
                    Color {
                        a: opacity,
                        ..p.text_on_cream
                    }
                    .into(),
                ),
                ..fill(tint, border::rounded(LABEL_RADIUS))
            }
        }
    }

    /// Separator before the open applications that are not pinned.
    pub fn separator(theme: &Theme) -> container::Style {
        fill(palette(theme).dock_separator, Border::default())
    }

    /// Dot below an open application. The board draws no active application: the active one
    /// takes the green of the active state.
    pub fn dot(active: bool) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let p = palette(theme);
            let tint = if active { p.accent } else { p.logo_dot };
            fill(tint, border::rounded(DOT / 2.0))
        }
    }

    /// Keyboard focus ring around an icon of `side`.
    pub fn ring(side: f32) -> impl Fn(&Theme) -> container::Style {
        move |theme| container::Style {
            border: border::width(metrics::FOCUS_RING_WIDTH)
                .color(palette(theme).focus_ring)
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
    fn the_label_and_the_dots_read_in_both_themes() {
        for p in [&Palette::DARK, &Palette::LIGHT] {
            assert!(p.text_on_cream.contrast(p.cream) >= 4.5);
            // The open dot, a non-text element, on the dock glass over the desktop.
            let glass = p.dock_glass.over(p.desktop);
            assert!(p.logo_dot.contrast(glass) >= 3.0, "{:?}", p.logo_dot);
        }
        assert!(FALLBACK_LETTER.contrast(FALLBACK_TILE) >= 4.5);
    }
}
