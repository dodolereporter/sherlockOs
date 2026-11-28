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

/// Bubbles shown at once at most; the others wait their turn. A choice of D-23: four bubbles of
/// three lines fit above the dock of the 900 high reference screen.
pub const MAX_VISIBLE: usize = 4;
/// The room a bubble of three lines takes in the stack, its gap included, and the room kept
/// for the dock under the stack: what `visible_for` counts with (D-23c).
pub const BUBBLE_ROOM: f32 = 128.0;
pub const DOCK_ROOM: f32 = 100.0;

/// Bubbles shown at once on a screen `height` logical pixels high: as many as fit between the
/// bar and the dock, one at least, `MAX_VISIBLE` at most. 1280 × 800 at 1.5 (533) shows two,
/// 1366 × 768 at 1.25 (614) three, 1920 × 1080 at 1.5 (720) and taller four.
pub fn visible_for(height: f32) -> usize {
    let top = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT + BELOW_BAR;
    let room = (height - top - DOCK_ROOM).max(0.0);
    ((room / BUBBLE_ROOM) as usize).clamp(1, MAX_VISIBLE)
}
/// Time a bubble stays when its application gives none. A choice of D-23.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(6);
/// Characters kept of the body, the title, the application name and an action label: a bubble
/// stays a bubble, and a client cannot make the daemon hold or lay out more.
pub const MAX_BODY_CHARS: usize = 300;
pub const MAX_SUMMARY_CHARS: usize = 120;
pub const MAX_APP_NAME_CHARS: usize = 48;
pub const MAX_LABEL_CHARS: usize = 24;
/// Action buttons on one row of a bubble: three compact buttons share its width. With more
/// actions, the first `MAX_BUTTONS - 1` stay and « Options » lists the others, as on macOS.
pub const MAX_BUTTONS: usize = 3;
/// Actions an « Options » list shows at most.
pub const MAX_OPTIONS: usize = 8;
/// Lines of body a bubble shows at most; a word longer than its width breaks.
pub const BODY_LINES: f32 = 4.0;
/// Line height of the body, relative to its size.
pub const BODY_LINE_HEIGHT: f32 = 1.3;
/// Largest side of the raw pixels of a notification: the tile is 20 px.
pub const MAX_PIXELS_SIDE: u32 = 256;
/// Notifications waiting for room at most; beyond, the oldest ones close, kept in the history.
pub const MAX_WAITING: usize = 50;
/// Notifications the history keeps, the newest ones.
pub const HISTORY: usize = 100;
/// The bubbles of one application pile up behind its newest one (D-23d): each level shows
/// `PILE_STEP` of the one behind, `PILE_DEPTH` levels at most, as on macOS.
pub const PILE_STEP: f32 = 6.0;
pub const PILE_DEPTH: usize = 2;
/// Swiped to the right past this share of its width, a bubble goes; the fingers rest
/// `SWIPE_END` before the swipe is over. Pulled the other way, it moves this share only.
pub const SWIPE_DISMISS: f32 = 0.35;
pub const SWIPE_END: Duration = Duration::from_millis(120);
pub const SWIPE_RESISTANCE: f32 = 0.25;
/// A wheel line turned sideways moves a bubble this much.
pub const SWIPE_LINE: f32 = 40.0;
/// The cross of « × », inside its button the size of the tile.
pub const CLOSE_GLYPH: f32 = 12.0;
/// « × » of a bubble, as on macOS (lot D-23f, the author's choice): a round badge on the top
/// left corner, half outside the bubble, its side, its cross; the surface keeps `CLOSE_OUTSET`
/// of room for it above and on the left. It fades in over `CLOSE_FADE` under the pointer.
pub const CLOSE_SIZE: f32 = 16.0;
pub const CLOSE_BADGE_GLYPH: f32 = 9.0;
pub const CLOSE_OUTSET: f32 = CLOSE_SIZE / 2.0;
pub const CLOSE_FADE: Duration = Duration::from_millis(120);
/// How far inside the corner of the bubble's box the badge's centre sits: on the curve of its
/// rounded corner, as on macOS, not beside it.
pub const CLOSE_ON_CURVE: f32 = RADIUS * (1.0 - std::f32::consts::FRAC_1_SQRT_2);
// The room above the badge is taken from the gap under the bar: the bubbles stay in place.
const _: () = assert!(CLOSE_OUTSET <= BELOW_BAR);
/// The theme icon of a notification is looked up at twice the tile (screens at scale 2).
pub const ICON_LOOKUP_SIZE: u32 = 40;

/// The spring of a bubble sliding in and out, and of the stack closing up: the spring of an
/// opening window.
pub const SPRING: motion::Spring = motion::OPEN_SPRING;

/// Notification texts, in French.
pub mod text {
    /// Shown when an application does not give its name.
    pub const UNKNOWN_APP: &str = "Application";
    /// The button that lists the actions beyond the row, as on macOS.
    pub const OPTIONS: &str = "Options";
    /// « × » on a bubble under the pointer.
    pub const CLOSE: &str = "Fermer";
    /// Under a pile of bubbles, and under the pile opened.
    pub fn more(count: usize) -> String {
        crate::panels::text::more(count)
    }
    pub const SHOW_LESS: &str = "Afficher moins";
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

    /// The action buttons of a notification (D-23c), for a bubble and for the notification
    /// centre: up to `MAX_BUTTONS` share one row, each label on one line ending with « … »;
    /// with more, the first `MAX_BUTTONS - 1` stay and « Options » (`toggle`) opens the others
    /// below, `MAX_OPTIONS` at most, as on macOS. `actions`: the label and the message of each,
    /// in order; the first one is the primary button.
    pub fn action_row<'a, Message: Clone + 'a>(
        actions: Vec<(&'a str, Message)>,
        options_open: bool,
        toggle: Message,
    ) -> iced::Element<'a, Message> {
        use iced::Length;
        use iced::widget::{Column, Row};
        let one = |label: &'a str, primary: bool, message: Message| {
            let font = if primary {
                typography::UI_SEMIBOLD
            } else {
                typography::UI
            };
            let label = container(crate::ellipsis::ellipsis(label).font(font))
                .center_x(Length::Fill)
                .center_y(Length::Fill);
            iced::widget::button(label)
                .width(Length::Fill)
                .height(BUTTON_HEIGHT)
                .padding([0.0, BUTTON_PADDING])
                .style(action(primary))
                .on_press(message)
        };
        let mut actions = actions.into_iter();
        let crowded = actions.len() > MAX_BUTTONS;
        let kept = if crowded {
            MAX_BUTTONS - 1
        } else {
            MAX_BUTTONS
        };
        let mut line = Row::new().spacing(BUTTON_GAP);
        for (i, (label, message)) in actions.by_ref().take(kept).enumerate() {
            line = line.push(one(label, i == 0, message));
        }
        if !crowded {
            return line.into();
        }
        line = line.push(one(text::OPTIONS, false, toggle));
        let mut all = Column::new().push(line).spacing(BUTTON_GAP);
        if options_open {
            for (label, message) in actions.take(MAX_OPTIONS) {
                all = all.push(one(label, false, message));
            }
        }
        all.into()
    }

    /// « × » of a bubble under the pointer, the macOS badge (lot D-23f): round, on the glass of
    /// a card, its border, at `opacity`.
    pub fn close_badge<'a, Message: Clone + 'a>(
        on_press: Message,
        opacity: f32,
    ) -> iced::Element<'a, Message> {
        let p = crate::color::palette();
        let glyph = crate::component::icon(
            crate::icon::Icon::Close,
            CLOSE_BADGE_GLYPH,
            p.text_secondary,
        );
        let content = container(glyph).center(iced::Length::Fill);
        let mut control = crate::component::Control::new(
            content,
            theme::secondary,
            Some(on_press),
            CLOSE_SIZE / 2.0,
        );
        control.label = Some(text::CLOSE);
        control.button = control
            .button
            .width(CLOSE_SIZE)
            .height(CLOSE_SIZE)
            .padding(0);
        crate::animated::reveal(control).opacity(opacity).into()
    }

    /// « × » of a bubble under the pointer (D-23d): the size of the application's tile, so that
    /// the header keeps its height when it shows.
    pub fn close_button<'a, Message: Clone + 'a>(on_press: Message) -> iced::Element<'a, Message> {
        let tint = crate::color::palette().text_secondary;
        let glyph = crate::component::icon(crate::icon::Icon::Close, CLOSE_GLYPH, tint);
        let content = container(glyph).center(iced::Length::Fill);
        let mut control = crate::component::Control::new(
            content,
            theme::icon_button,
            Some(on_press),
            TILE_RADIUS,
        );
        control.label = Some(text::CLOSE);
        control.button = control.button.width(TILE).height(TILE).padding(0);
        control.into()
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
    fn as_many_bubbles_show_as_the_screen_holds() {
        assert_eq!(visible_for(533.3), 2);
        assert_eq!(visible_for(614.4), 3);
        assert_eq!(visible_for(720.0), 4);
        assert_eq!(visible_for(2160.0), MAX_VISIBLE);
        assert_eq!(visible_for(200.0), 1);
        // Four bubbles of three lines fit above the dock on each screen that shows four.
        for height in [720.0, 900.0, 1080.0] {
            let top = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT + BELOW_BAR;
            let n = visible_for(height) as f32;
            assert!(top + n * BUBBLE_ROOM + DOCK_ROOM <= height, "{height}");
        }
    }

    #[test]
    fn the_first_bubble_sits_where_the_board_puts_the_centre() {
        let bar = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT;
        assert_eq!(bar + BELOW_BAR, 54.0);
        assert_eq!(metrics::REFERENCE_SCREEN.0 - SCREEN_EDGE, 1428.0);
    }
}
