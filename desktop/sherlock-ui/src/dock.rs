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
/// The dock fits its screen (lot D-21l), as on macOS: its icons shrink, down to `FIT_MIN_ICON`,
/// until the surface leaves `SCREEN_ROOM` at each end of the screen.
pub const FIT_MIN_ICON: f32 = 20.0;
pub const SCREEN_ROOM: f32 = 8.0;
/// Initial of an application without icon, as a share of the tile.
pub const LETTER_RATIO: f32 = 0.42;
/// Scale the icon files are looked up for, as in the launcher: a scalable icon is drawn at the
/// screen's own size, a fixed one is sharp on a scale 2 screen and scaled down on the others.
pub const ICON_SCALE: u32 = 2;

/// Dock menu (Menus board): width, gap above the bar, height of the header with the
/// application name.
pub const MENU_WIDTH: f32 = 240.0;
/// The dock's own menu, wider for « Afficher Sherlock dans le Dock » and its check mark.
pub const OWN_MENU_WIDTH: f32 = 290.0;
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

// Motion of lot D-21c, after macOS: the dot of an application that opens or closes its last
// window fades, and the menu of an icon unfolds from the icon like the menus of the bar.

/// A swipe down over an icon opens App Expose once the scroll it sends goes this far down
/// (logical pixels; a notch of a mouse wheel is enough), each scroll coming within
/// `EXPOSE_SWIPE_GAP` of the previous one. A choice of D-21c2, to review on the machine.
pub const EXPOSE_SWIPE: f32 = 40.0;
pub const EXPOSE_SWIPE_GAP: Duration = Duration::from_millis(250);

/// The dot under an icon appearing or going: the fade of an icon arriving.
pub const DOT_FADE: Duration = ICON_FADE;
/// The menu of an icon opening and closing: the spring, fade and scale of the bar's panels.
pub const MENU_SPRING: Spring = crate::panels::OPEN_SPRING;
pub const MENU_FADE: Duration = crate::panels::OPEN_FADE;
pub const MENU_SCALE: f32 = crate::panels::OPEN_SCALE;

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
/// Hops of an icon whose window asks for attention, refused the keyboard (lot Q-52): a few,
/// then it rests, as on macOS for a request that is not critical.
pub const ATTENTION_HOPS: u32 = 3;

/// Distance the pointer travels with the button held before the icon follows it: under it, a
/// press and a release make a click.
pub const DRAG_THRESHOLD: f32 = 4.0;
/// Opacity of an icon dragged out of the dock, where letting it go removes it.
pub const DRAG_OUT_OPACITY: f32 = 0.5;

/// A pinned application let go out of the dock goes up in a puff of smoke, as on macOS (lot
/// D-21d): its duration, its puffs set around the icon, how far they drift and how big they
/// grow, as shares of the icon side. Choices of D-21d, to review with the design.
pub const POOF_DURATION: Duration = Duration::from_millis(400);
pub const POOF_PUFFS: usize = 5;
pub const POOF_DRIFT: (f32, f32) = (0.10, 0.42);
pub const POOF_RADIUS: (f32, f32) = (0.20, 0.34);
/// An open application dragged across the separator changes side once the pointer is this
/// share of an icon past it: it does not flap back and forth on the line.
pub const CROSS_MARGIN: f32 = 0.25;

// Hiding (batch D-18c2): `AutoHide`, `docs/dock.md`.

/// Band along the screen edge where a hidden dock still takes the pointer, to come back.
pub const HIDE_STRIP: f32 = 2.0;
/// How long the pointer rests against that band before the dock comes back (lot D-21d): as on
/// macOS, a short wait, so that a pointer passing by the edge, or aiming at the bottom of a
/// fullscreen window, does not bring it out by mistake.
pub const REVEAL_DELAY: Duration = Duration::from_millis(150);
/// A dock that came back over a maximized window hides again this long after the pointer left
/// it.
pub const HIDE_DELAY: Duration = Duration::from_millis(300);
/// The dock sliding off the screen and back: it answers a window being maximized or restored,
/// like the top bar, so it moves with the same spring.
pub const HIDE_SPRING: Spring = motion::TOP_BAR_SPRING;

// Colors. The dock bar, its label, dots and separator take the palette shown
// (`theme::palette`); the tiles keep their colors in both themes (`docs/design/THEME-CLAIR.md`).

/// Tile of an application whose icon is not found, and its initial (the « Fichiers » tile of the board).
pub const FALLBACK_TILE: Color = Palette::DARK.blue_dark;
pub const FALLBACK_LETTER: Color = Palette::DARK.cream;

/// Dock texts, in French.
pub mod text {
    /// Name of the Sherlock icon, in its label and menu.
    pub const SHERLOCK: &str = "Sherlock";
    pub const OPEN: &str = "Ouvrir";
    pub const NEW_WINDOW: &str = "Nouvelle fenêtre";
    pub const KEEP: &str = "Garder dans le Dock";
    pub const UNPIN: &str = "Retirer du Dock";
    /// The button that opens the applications view (D-41), in its label and menu.
    pub const APPLICATIONS: &str = "Applications";
    pub const QUIT: &str = "Quitter";
    // The options of an icon and the end of its menu (D-21i).
    pub const OPTIONS: &str = "Options";
    pub const OPEN_AT_LOGIN: &str = "Ouvrir à l’ouverture de session";
    pub const SHOW_IN_EVIDENCE: &str = "Afficher dans Evidence";
    pub const HIDE: &str = "Masquer";
    pub const FORCE_QUIT: &str = "Forcer à quitter";
    /// App Expose (lot D-21c2): Mission Control on the windows of one application.
    pub const SHOW_ALL: &str = "Afficher toutes les fenêtres";
    /// Title of the dock's own menu: a right click on its separator or between its icons.
    pub const DOCK: &str = "Dock";
    /// The entry of the dock's own menu that puts Sherlock back, or takes it out (checked).
    pub const SHOW_SHERLOCK: &str = "Afficher Sherlock dans le Dock";

    /// Number of open windows, at the top right of the menu, like « 3 en marche » on the board.
    pub fn windows(count: usize) -> String {
        match count {
            1 => String::from("1 fenêtre"),
            n => format!("{n} fenêtres"),
        }
    }
}

/// The « Applications » button (D-41): nine squares on a slate tile, in the style of the
/// application icons of `app_icon` (24 grid, tile radius 5.28, faint white border).
pub fn applications_icon() -> String {
    const PLACES: [f32; 3] = [5.3, 10.3, 15.3];
    let squares: String = (0..9)
        .map(|n| {
            // The middle one in the accent green, the others cream.
            let fill = if n == 4 {
                hex(crate::color::ACCENT)
            } else {
                hex(crate::color::CREAM)
            };
            let (x, y) = (PLACES[n % 3], PLACES[n / 3]);
            format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"3.4\" height=\"3.4\" rx=\"1\" fill=\"{fill}\"/>"
            )
        })
        .collect();
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\"><defs><clipPath id=\"tile\">\
         <rect width=\"24\" height=\"24\" rx=\"5.28\"/></clipPath></defs><g clip-path=\"url(#tile)\">\
         <rect width=\"24\" height=\"24\" fill=\"{tile}\"/>{squares}</g><rect x=\".09\" y=\".09\" \
         width=\"23.82\" height=\"23.82\" rx=\"5.19\" fill=\"none\" stroke=\"white\" \
         stroke-opacity=\".1\" stroke-width=\".18\"/></svg>",
        tile = hex(crate::color::TILE_APPLICATIONS),
    )
}

fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
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
    /// Dot of an open application, green when it is in front, at `opacity` while it fades.
    pub fn dot(active: bool, opacity: f32) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let p = palette(theme);
            let tint = if active { p.accent } else { p.logo_dot };
            let tint = Color {
                a: tint.a * opacity.clamp(0.0, 1.0),
                ..tint
            };
            fill(tint, border::rounded(DOT / 2.0))
        }
    }

    /// A puff of the smoke of an unpinned application, at `opacity`: the secondary text color,
    /// light in the dark theme and dark in the light one, like a cloud over the desktop.
    pub fn poof(radius: f32, opacity: f32) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let tint = palette(theme).text_secondary;
            let tint = Color {
                a: tint.a * opacity.clamp(0.0, 1.0),
                ..tint
            };
            fill(tint, border::rounded(radius))
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
