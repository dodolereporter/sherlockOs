//! The notification area of the top bar (lot D-45): the icons applications put there with
//! `StatusNotifierItem` (Discord, Steam, a VPN), behind a chevron before the indicators, in a
//! glass popup that opens under it, as the hidden icons of Windows do. No board draws it: the
//! measures below are choices of the lot, taken from the bar and the menus, to review with the
//! design. See `docs/bar.md`, « Notification area ».

use std::time::Duration;

use crate::color::{Color, Palette};
use crate::{metrics, motion, panels};

/// Side of an icon in the bar: the indicators'. It follows the bar: a bar taller or shorter
/// would carry icons of the same share of its height.
pub const ICON: f32 = metrics::STATUS_ICON;
/// Side of an icon in the popup, a little larger than in the bar, and of its cell: the hit area,
/// with the hover pill of the bar's buttons.
pub const POPUP_ICON: f32 = 22.0;
pub const CELL: f32 = 40.0;
pub const CELL_RADIUS: f32 = metrics::SHERLOCK_BUTTON_RADIUS;
/// Gap between two cells.
pub const GAP: f32 = 2.0;
/// Columns of the grid at most; fewer icons make a narrower popup.
pub const COLUMNS: usize = 5;
/// Glass of the popup: padding, radius (a menu's), and the line of the tooltip under the
/// grid.
pub const PADDING: f32 = metrics::MENU_PADDING;
pub const RADIUS: f32 = metrics::MENU_RADIUS;
pub const TIP_HEIGHT: f32 = 26.0;
/// Gap between the bottom of the bar and the popup: a menu's.
pub const BELOW_BAR: f32 = metrics::TOP_BAR_MENU_GAP;
/// The popup holds at least the empty notice.
pub const MIN_WIDTH: f32 = 200.0;

/// The chevron: its glyph, which points down while the popup is closed and turns half a turn
/// on the spring of a panel as it opens.
pub const CHEVRON: f32 = 14.0;
pub const CHEVRON_SPRING: motion::Spring = panels::OPEN_SPRING;
/// Positions of the half turn: one every 10°.
pub const CHEVRON_STEPS: u32 = 18;
/// The overlay of an icon (a badge drawn by the application), at its bottom right corner, as a
/// share of the icon's side.
pub const OVERLAY_SHARE: f32 = 0.55;

/// An icon that needs attention breathes: it grows by `PULSE_SCALE` and back, one breath every
/// `PULSE`, on a sine. Never a blink. The chevron carries a dot of `DOT` meanwhile.
pub const PULSE: Duration = Duration::from_millis(1600);
pub const PULSE_SCALE: f32 = 0.12;
pub const DOT: f32 = 5.0;

/// The growth of an icon that needs attention, `at` into the pulse: 0 at the start of a breath,
/// `PULSE_SCALE` at its middle.
pub fn breath(at: Duration) -> f32 {
    let period = PULSE.as_secs_f32();
    let phase = (at.as_secs_f32() % period) / period;
    PULSE_SCALE * (1.0 - (phase * std::f32::consts::TAU).cos()) / 2.0
}

/// Width of the popup for `count` icons: the columns they fill, at least `MIN_WIDTH`.
pub fn width(count: usize) -> f32 {
    let columns = count.clamp(1, COLUMNS) as f32;
    let grid = columns * CELL + (columns - 1.0) * GAP;
    (grid + 2.0 * (PADDING + metrics::BORDER_WIDTH)).max(MIN_WIDTH)
}

/// Height of the popup for `count` icons: their rows and the tooltip line under them.
pub fn height(count: usize) -> f32 {
    let rows = count.div_ceil(COLUMNS).max(1) as f32;
    let grid = rows * CELL + (rows - 1.0) * GAP;
    grid + TIP_HEIGHT + 2.0 * (PADDING + metrics::BORDER_WIDTH)
}

/// The dot of the chevron while an icon needs attention: amber, the color of what waits.
pub fn dot(palette: &Palette) -> Color {
    palette.amber
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::{Stack, container, svg};
    use iced::{Element, Length, border};

    use super::*;
    use crate::color;
    use crate::component::Control;
    use crate::icon::Icon;
    use crate::theme;

    /// A cell of the popup around an icon: the hover pill and the press of the bar's buttons.
    pub fn cell<'a, Message: Clone + 'a>(
        content: impl Into<Element<'a, Message>>,
        on_press: Option<Message>,
    ) -> Control<'a, Message> {
        let content = container(content).center(Length::Fill);
        let mut control = Control::new(
            content,
            theme::sherlock_button(false),
            on_press,
            CELL_RADIUS,
        );
        control.button = control.button.width(CELL).height(CELL).padding(0);
        control.press_scale = panels::BAR_PRESS_SCALE;
        control
    }

    /// The chevron, `turn` of its half turn (0 pointing down, 1 up), with the dot of an icon
    /// that needs attention at its top right.
    pub fn chevron<'a, Message: 'a>(turn: f32, dot: bool) -> Element<'a, Message> {
        let p = color::palette();
        let tint = p.text_secondary;
        let glyph = svg(turned(turn))
            .width(CHEVRON)
            .height(CHEVRON)
            .opacity(color::icon_opacity(tint))
            .style(move |_, _| svg::Style {
                color: Some(tint.into()),
            });
        let glyph = container(glyph).center(ICON);
        if !dot {
            return glyph.into();
        }
        let tint = super::dot(p);
        let mark = container(iced::widget::space())
            .width(DOT)
            .height(DOT)
            .style(move |_| container::Style {
                background: Some(iced::Background::Color(tint.into())),
                border: border::rounded(DOT / 2.0),
                ..container::Style::default()
            });
        let mark = container(mark).align_right(ICON).align_top(ICON);
        Stack::with_children([glyph.into(), mark.into()]).into()
    }

    /// The chevron `turn` of its half turn, in `CHEVRON_STEPS` steps: turned inside the
    /// document rather than by the renderer, whose software version turns around the wrong
    /// point (as the spinner), each step built once.
    fn turned(turn: f32) -> svg::Handle {
        static STEPS: std::sync::OnceLock<Vec<svg::Handle>> = std::sync::OnceLock::new();
        let steps = STEPS.get_or_init(|| {
            (0..=CHEVRON_STEPS)
                .map(|step| {
                    // The glyph of a submenu points right: a quarter turn makes it point down.
                    let degrees = 90.0 + 180.0 * step as f32 / CHEVRON_STEPS as f32;
                    let path = format!("{}\" transform=\"rotate({degrees} 12 12)", Icon::Chevron.path());
                    let document = format!(
                        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\" \
                         fill=\"none\" stroke=\"#000\" stroke-width=\"{stroke}\" \
                         stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>",
                        grid = metrics::ICON_GRID,
                        stroke = metrics::ICON_STROKE,
                    );
                    svg::Handle::from_memory(document.into_bytes())
                })
                .collect()
        });
        let step = (turn.clamp(0.0, 1.0) * CHEVRON_STEPS as f32).round() as usize;
        steps[step.min(steps.len() - 1)].clone()
    }
}

/// Texts of the notification area, in French.
pub mod text {
    /// Label of the chevron, for the screen readers to come.
    pub const CHEVRON: &str = "Icônes de la zone de notification";
    /// The popup without an icon.
    pub const EMPTY: &str = "Aucune icône pour l'instant";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_popup_grows_with_its_icons() {
        assert_eq!(width(0), MIN_WIDTH);
        assert!(width(COLUMNS) > MIN_WIDTH);
        assert_eq!(width(COLUMNS), width(COLUMNS * 3));
        assert!(height(COLUMNS + 1) > height(COLUMNS));
        assert_eq!(height(0), height(1));
    }

    #[test]
    fn an_icon_breathes_without_blinking() {
        assert_eq!(breath(Duration::ZERO), 0.0);
        let middle = breath(PULSE / 2);
        assert!((middle - PULSE_SCALE).abs() < 1e-4);
        // Smooth: a frame later it has barely moved.
        let step = (breath(PULSE / 4) - breath(PULSE / 4 + Duration::from_millis(16))).abs();
        assert!(step < PULSE_SCALE / 20.0);
    }
}
