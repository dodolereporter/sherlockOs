//! Notifications (batch D-23): the tokens, styles and texts of the notification bubbles. Values
//! from the notification centre of the Panels design board, `docs/design/planches/Panneaux.dc.html`:
//! a bubble is one of its cards, standing alone on the panel glass. The board draws no bubble
//! of its own: the choices are listed in `docs/notifications.md`.

use std::time::Duration;

use crate::{metrics, motion, typography};

// Geometry, in logical pixels.

/// Width of a bubble: the width of the board's notification centre.
pub const WIDTH: f32 = 320.0;
/// Inner padding of a bubble, gap between its rows (the board's cards).
pub const PADDING: f32 = 12.0;
pub const ROW_GAP: f32 = 6.0;
pub const RADIUS: f32 = metrics::CARD_RADIUS;
/// Gap between two bubbles of the stack (the board's gap between cards).
pub const GAP: f32 = 8.0;
/// Distance between the area the top bar reserves and the first bubble: the board's centre
/// starts at 54, the bar ends at 46.
pub const BELOW_BAR: f32 = 8.0;
/// Distance between a bubble and the right edge of the screen: the bar's side margin.
pub const SCREEN_EDGE: f32 = metrics::TOP_BAR_MARGIN_SIDE;
/// Header row: application tile, its radius and initial, gap, text size.
pub const TILE: f32 = 20.0;
pub const TILE_RADIUS: f32 = 6.0;
pub const TILE_LETTER_SIZE: f32 = 11.0;
pub const HEADER_GAP: f32 = 8.0;
pub const HEADER_SIZE: f32 = typography::SMALL_SIZE;
/// Action buttons: the board's compact buttons, their side padding, the gap between them and
/// the room above them.
pub const BUTTON_HEIGHT: f32 = metrics::BUTTON_COMPACT_HEIGHT;
pub const BUTTON_RADIUS: f32 = metrics::BUTTON_COMPACT_RADIUS;
pub const BUTTON_PADDING: f32 = 12.0;
pub const BUTTON_GAP: f32 = 6.0;
pub const BUTTONS_TOP: f32 = 2.0;

/// Room the surface keeps below the stack, so that a spring passing its target never squeezes
/// the last bubble; and room it gives a new bubble to be laid out in, out of sight, before its
/// height is known: more than the tallest bubble (title, 300 characters of body, buttons).
pub const STACK_SLACK: f32 = 2.0 * GAP;
pub const MEASURE_ROOM: f32 = 480.0;

// Behavior.

/// Bubbles shown at once; the others wait their turn. A choice of D-23: four bubbles of three
/// lines fit above the dock of the 900 high reference screen.
pub const MAX_VISIBLE: usize = 4;
/// Time a bubble stays when its application gives none. A choice of D-23.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);
/// Characters kept of the body, the title, the application name and an action label: a bubble
/// stays a bubble, and a client cannot make the daemon hold or lay out more.
pub const MAX_BODY_CHARS: usize = 300;
pub const MAX_SUMMARY_CHARS: usize = 120;
pub const MAX_APP_NAME_CHARS: usize = 48;
pub const MAX_LABEL_CHARS: usize = 24;
/// Action buttons shown on a bubble: three compact buttons fit its width.
pub const MAX_BUTTONS: usize = 3;
/// Largest side of the raw pixels of a notification: the tile is 20 px.
pub const MAX_PIXELS_SIDE: u32 = 256;
/// Notifications waiting for room at most; beyond, the oldest ones close, kept in the history.
pub const MAX_WAITING: usize = 50;
/// Notifications the history keeps, the newest ones.
pub const HISTORY: usize = 100;
/// The spring of a bubble sliding in and out, and of the stack closing up: the spring of an
/// opening window.
pub const SPRING: motion::Spring = motion::OPEN_SPRING;

/// Notification texts, in French.
pub mod text {
    /// Shown when an application does not give its name.
    pub const UNKNOWN_APP: &str = "Application";
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::{button, container};
    use iced::{Background, Theme, border};

    use super::*;
    use crate::theme;

    /// A bubble: the panel glass and border, the card radius. A critical notification takes a
    /// salmon border. Blur is the compositor's job.
    pub fn bubble(critical: bool) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let p = theme::palette(theme);
            let edge = if critical { p.salmon } else { p.border };
            container::Style {
                background: Some(Background::Color(p.glass.into())),
                border: border::width(metrics::BORDER_WIDTH)
                    .color(edge)
                    .rounded(RADIUS),
                text_color: Some(p.text_primary.into()),
                ..container::Style::default()
            }
        }
    }

    /// Tile of an application without an icon, with its initial: the same in both themes,
    /// as the launcher's tiles.
    pub fn tile(_: &Theme) -> container::Style {
        container::Style {
            background: Some(Background::Color(crate::color::BLUE_BRIGHT.into())),
            border: border::rounded(TILE_RADIUS),
            text_color: Some(crate::color::DEEPEST.into()),
            ..container::Style::default()
        }
    }

    /// Header (application name) and body: the secondary text of the board.
    pub fn secondary_text(theme: &Theme) -> iced::widget::text::Style {
        iced::widget::text::Style {
            color: Some(theme::palette(theme).text_secondary.into()),
        }
    }

    /// An action: the first one is the primary button of the board, the others secondary.
    pub fn action(primary: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |theme, status| {
            let mut style = if primary {
                theme::primary(theme, status)
            } else {
                theme::secondary(theme, status)
            };
            style.border.radius = BUTTON_RADIUS.into();
            style
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Palette;

    #[test]
    fn texts_read_on_a_bubble_in_both_themes() {
        for palette in [&Palette::DARK, &Palette::LIGHT] {
            // The glass laid over each disc of the desktop, the lightest being the worst.
            for below in palette.desktop_discs {
                let glass = palette.glass.over(below);
                for text in [palette.text_primary, palette.text_secondary] {
                    assert!(text.contrast(glass) >= 4.5, "{text:?} on {glass:?}");
                }
                assert!(palette.text_on_accent.contrast(palette.accent) >= 4.5);
            }
        }
        assert!(crate::color::DEEPEST.contrast(crate::color::BLUE_BRIGHT) >= 4.5);
    }

    #[test]
    fn the_first_bubble_sits_where_the_board_puts_the_centre() {
        let bar = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT;
        assert_eq!(bar + BELOW_BAR, 54.0);
        assert_eq!(metrics::REFERENCE_SCREEN.0 - SCREEN_EDGE, 1428.0);
    }
}
