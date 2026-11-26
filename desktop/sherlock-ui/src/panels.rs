//! Panels of the top bar (batch D-20b): the notification centre and the quick settings of the
//! Panels design board, `docs/design/planches/Panneaux.dc.html` (and its light version), and
//! the calendar under the clock, which no board draws: it follows the same panel glass and
//! the rules of the visual identity. Tokens, colors, styles and the French texts.

use crate::color::{Color, Palette};
use crate::{metrics, typography};

// Geometry, in logical pixels.

/// Distance between the area the top bar reserves and the panels: the board puts them at 54,
/// the bar ends at 46.
pub const BELOW_BAR: f32 = 8.0;
/// Inner padding and radius of a panel (the board's `padding: 14px`, `border-radius: 20px`).
pub const PADDING: f32 = metrics::PANEL_PADDING;
pub const RADIUS: f32 = metrics::PANEL_RADIUS;
/// Gap between the notification centre and the quick settings on the board (756 + 320 + 16 =
/// 1092).
pub const PANEL_GAP: f32 = 16.0;

/// Notification centre: width, gap between its rows, title size, « Tout effacer » button.
pub const CENTER_WIDTH: f32 = 320.0;
pub const CENTER_GAP: f32 = 8.0;
pub const TITLE_SIZE: f32 = typography::TITLE_SMALL_SIZE;
pub const CLEAR_HEIGHT: f32 = 26.0;
pub const CLEAR_RADIUS: f32 = 8.0;
pub const CLEAR_PADDING: f32 = 10.0;
/// Card of a group of notifications: padding, gap between its rows, radius, the application
/// tile and its initial, gap in the header, and the gap between two notifications of a group.
pub const CARD_PADDING: f32 = 12.0;
pub const CARD_GAP: f32 = 6.0;
pub const CARD_RADIUS: f32 = metrics::CARD_RADIUS;
pub const TILE: f32 = 20.0;
pub const TILE_RADIUS: f32 = 6.0;
pub const TILE_LETTER_SIZE: f32 = 11.0;
pub const HEADER_GAP: f32 = 8.0;
pub const ENTRY_GAP: f32 = 10.0;
/// Action buttons of an open notification: compact, as on the board.
pub const ACTION_HEIGHT: f32 = metrics::BUTTON_COMPACT_HEIGHT;
pub const ACTION_RADIUS: f32 = metrics::BUTTON_COMPACT_RADIUS;
pub const ACTION_PADDING: f32 = 12.0;
pub const ACTION_GAP: f32 = 6.0;
/// Notifications of one application shown in its card, the newest ones; the others are
/// counted below them.
pub const GROUP_MAX: usize = 3;

/// Quick settings: width. Its height sets the height of the whole control centre, the
/// notification centre included (see `CONTROL_HEIGHT`).
pub const QUICK_WIDTH: f32 = 336.0;
/// Height of the control centre panels: the board's quick settings (a grid of two rows of 56
/// and 8, the volume and brightness rows, the two segmented groups, the user row, 14 between
/// each), plus the line under the volume that names the output.
pub const CONTROL_HEIGHT: f32 = 2.0 * (PADDING + metrics::BORDER_WIDTH)
    + (2.0 * 56.0 + 8.0)
    + 5.0 * metrics::GAP_SECTION
    + (28.0 + 4.0 + 17.0)
    + 28.0
    + 2.0 * (17.0 + 6.0 + 36.0)
    + (metrics::BORDER_WIDTH + 12.0 + 34.0);

/// Calendar: one day cell, the radius of the mark of today, gap below the month row and
/// below the weekday row, the agenda slot.
pub const DAY_CELL: f32 = 36.0;
pub const DAY_MARK: f32 = 30.0;
pub const MONTH_ROW: f32 = 32.0;
pub const WEEKDAY_ROW: f32 = 22.0;
pub const CALENDAR_GAP: f32 = 8.0;
pub const AGENDA_HEIGHT: f32 = 56.0;
/// Weeks shown: always six, so that the panel keeps its height from one month to the next.
pub const WEEKS: usize = 6;
pub const CALENDAR_WIDTH: f32 = 2.0 * (PADDING + metrics::BORDER_WIDTH) + 7.0 * DAY_CELL;
pub const CALENDAR_HEIGHT: f32 = 2.0 * (PADDING + metrics::BORDER_WIDTH)
    + MONTH_ROW
    + CALENDAR_GAP
    + WEEKDAY_ROW
    + WEEKS as f32 * DAY_CELL
    + metrics::GAP_SECTION
    + metrics::BORDER_WIDTH
    + AGENDA_HEIGHT;

/// Ghost button around the clock and around the indicators of the bar: its side padding. The
/// bar takes it back from its gaps, so that nothing moves.
pub const BAR_BUTTON_PADDING: f32 = 6.0;
pub const BAR_BUTTON_HEIGHT: f32 = 28.0;

// Colors.

/// The panels' own colors, for one theme; the rest comes from the palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Card of a notification: the board's `rgba(255,255,255,0.07)`.
    pub card: Color,
    /// « Tout effacer » and the tiles that are off: `0.08`.
    pub control: Color,
}

impl Colors {
    pub const DARK: Self = Self {
        card: Color::rgba(255, 255, 255, 0.07),
        control: Color::rgba(255, 255, 255, 0.08),
    };

    /// The light board: the same veils, in night blue.
    pub const LIGHT: Self = Self {
        card: Color::rgba(20, 35, 59, 0.07),
        control: Color::rgba(20, 35, 59, 0.08),
    };

    /// The colors that go with `palette`.
    pub fn of(palette: &Palette) -> &'static Self {
        if *palette == Palette::LIGHT {
            &Self::LIGHT
        } else {
            &Self::DARK
        }
    }
}

/// Panel texts, in French.
pub mod text {
    /// Notification centre.
    pub const NOTIFICATIONS: &str = "Notifications";
    pub const CLEAR_ALL: &str = "Tout effacer";
    pub const NO_NOTIFICATION: &str = "Aucune notification";
    pub const CENTER_UNAVAILABLE: &str = "Le service des notifications ne répond pas.";
    /// Notifications of an application beyond those shown in its card.
    pub fn more(count: usize) -> String {
        if count == 1 {
            String::from("1 autre notification")
        } else {
            format!("{count} autres notifications")
        }
    }

    /// Calendar.
    pub const MONTHS: [&str; 12] = [
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
    /// Heads of the columns, Monday first.
    pub const WEEKDAYS: [&str; 7] = ["lu", "ma", "me", "je", "ve", "sa", "di"];
    pub const PREVIOUS_MONTH: &str = "Mois précédent";
    pub const NEXT_MONTH: &str = "Mois suivant";
    pub const AGENDA: &str = "Agenda";
    pub const AGENDA_SOON: &str = "Bientôt : tes rendez-vous du jour ici.";

    /// Title of the month shown, such as « novembre 2026 ». `month` goes from 1 to 12.
    pub fn month_title(month: u8, year: i32) -> String {
        let name = usize::from(month)
            .checked_sub(1)
            .and_then(|i| MONTHS.get(i))
            .copied()
            .unwrap_or_default();
        format!("{name} {year}")
    }
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::{button, container, text};
    use iced::{Background, Element, Length, Theme, border};

    use super::*;
    use crate::component::Control;
    use crate::theme;

    fn fill(c: Color) -> Option<Background> {
        Some(Background::Color(c.into()))
    }

    /// A panel: the glass and border of the board, radius 20. Its shadow is drawn by
    /// `surface::Shadowed`.
    pub fn panel(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: fill(p.glass),
            border: border::width(metrics::BORDER_WIDTH)
                .color(p.border)
                .rounded(RADIUS),
            text_color: Some(p.text_primary.into()),
            ..container::Style::default()
        }
    }

    /// Card of a group of notifications.
    pub fn card(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: fill(Colors::of(p).card),
            border: border::rounded(CARD_RADIUS),
            text_color: Some(p.text_primary.into()),
            ..container::Style::default()
        }
    }

    /// Tile of an application in a card, with its initial: the bubbles' tile.
    pub fn tile(theme: &Theme) -> container::Style {
        crate::notifications::tile(theme)
    }

    /// Secondary text: application name, time, body.
    pub fn secondary_text(theme: &Theme) -> text::Style {
        text::Style {
            color: Some(theme::palette(theme).text_secondary.into()),
        }
    }

    /// « Tout effacer »: a veiled compact button with secondary text.
    fn clear_style(theme: &Theme, status: button::Status) -> button::Style {
        let p = theme::palette(theme);
        let veil = Colors::of(p).control;
        let (background, text) = match status {
            button::Status::Active => (veil, p.text_secondary),
            button::Status::Hovered => (p.chip_hover, p.text_primary),
            button::Status::Pressed => (p.pressed, p.text_primary),
            button::Status::Disabled => (p.disabled, p.text_tertiary),
        };
        button::Style {
            background: fill(background),
            text_color: text.into(),
            border: border::rounded(CLEAR_RADIUS),
            ..button::Style::default()
        }
    }

    /// The « Tout effacer » button of the notification centre.
    pub fn clear_button<'a, Message: Clone + 'a>(
        label: &'a str,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let label = text(label)
            .font(typography::UI)
            .size(typography::SMALL_SIZE);
        let content = container(label).center_y(Length::Fill);
        let mut control = Control::new(content, clear_style, on_press, CLEAR_RADIUS);
        control.button = control
            .button
            .height(CLEAR_HEIGHT)
            .padding([0.0, CLEAR_PADDING]);
        control
    }

    /// An action of an open notification: the first one primary, the others secondary.
    pub fn action_button<'a, Message: Clone + 'a>(
        label: &'a str,
        primary: bool,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let font = if primary {
            typography::UI_SEMIBOLD
        } else {
            typography::UI
        };
        let content = container(crate::component::body(label).font(font)).center_y(Length::Fill);
        let style = crate::notifications::action(primary);
        let mut control = Control::new(content, style, on_press, ACTION_RADIUS);
        control.button = control
            .button
            .height(ACTION_HEIGHT)
            .padding([0.0, ACTION_PADDING]);
        control
    }

    /// Ghost button of the top bar around the clock or the indicators: veiled on hover, and
    /// while its panel is open, like the Sherlock button.
    pub fn bar_button<'a, Message: Clone + 'a>(
        content: impl Into<Element<'a, Message>>,
        open: bool,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let content = container(content).center_y(Length::Fill);
        let mut control = Control::new(
            content,
            theme::sherlock_button(open),
            on_press,
            metrics::SHERLOCK_BUTTON_RADIUS,
        );
        control.button = control
            .button
            .height(BAR_BUTTON_HEIGHT)
            .padding([0.0, BAR_BUTTON_PADDING]);
        control
    }

    /// Mark of today in the calendar: a green disc, dark text.
    pub fn today(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: fill(p.accent),
            border: border::rounded(DAY_MARK / 2.0),
            text_color: Some(p.text_on_accent.into()),
            ..container::Style::default()
        }
    }

    /// A day of the month shown, or of a month next to it (faded).
    pub fn day_text(in_month: bool) -> impl Fn(&Theme) -> text::Style {
        move |theme| {
            let p = theme::palette(theme);
            let c = if in_month {
                p.text_primary
            } else {
                p.text_tertiary
            };
            text::Style {
                color: Some(c.into()),
            }
        }
    }

    /// Heads of the weekday columns.
    pub fn weekday_text(theme: &Theme) -> text::Style {
        text::Style {
            color: Some(theme::palette(theme).text_tertiary.into()),
        }
    }

    /// Error line of a panel: the salmon as text.
    pub fn error_text(theme: &Theme) -> text::Style {
        text::Style {
            color: Some(theme::palette(theme).salmon_text.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panels_sit_where_the_board_puts_them() {
        let bar = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT;
        assert_eq!(bar + BELOW_BAR, 54.0);
        // The notification centre at 756, the quick settings at 1092, 12 from the right edge.
        let right = metrics::REFERENCE_SCREEN.0 - metrics::TOP_BAR_MARGIN_SIDE;
        assert_eq!(right - QUICK_WIDTH, 1092.0);
        assert_eq!(right - QUICK_WIDTH - PANEL_GAP - CENTER_WIDTH, 756.0);
        // The control centre stays clear of the dock on the reference screen.
        assert!(bar + BELOW_BAR + CONTROL_HEIGHT < metrics::REFERENCE_SCREEN.1 - 150.0);
        assert!(CALENDAR_HEIGHT < CONTROL_HEIGHT);
    }

    #[test]
    fn texts_read_on_the_panels_in_both_themes() {
        for palette in [&Palette::DARK, &Palette::LIGHT] {
            let colors = Colors::of(palette);
            for below in palette.desktop_discs {
                let glass = palette.glass.over(below);
                let card = colors.card.over(glass);
                for back in [glass, card, colors.control.over(glass)] {
                    for text in [palette.text_primary, palette.text_secondary] {
                        assert!(text.contrast(back) >= 4.5, "{text:?} on {back:?}");
                    }
                    assert!(palette.salmon_text.contrast(back) >= 4.5);
                }
                assert!(palette.text_on_accent.contrast(palette.accent) >= 4.5);
            }
        }
    }

    #[test]
    fn the_month_title_is_in_french() {
        assert_eq!(text::month_title(11, 2026), "novembre 2026");
        assert_eq!(text::month_title(2, 2028), "février 2028");
        assert_eq!(text::month_title(13, 2028), " 2028");
        assert_eq!(text::more(1), "1 autre notification");
        assert_eq!(text::more(4), "4 autres notifications");
    }
}
