//! Title bar and buttons that the compositor draws for the applications that leave their
//! decoration to it (server-side decorations, lot D-08). The measures are those of the title
//! bar of the Main board and of the files board, which Evidence draws itself
//! (`window_controls`): a bar of 40 under the window border, a 1 px divider, the title on the
//! left, the three buttons in their pill on the right. See "Server-side decorations" in
//! `docs/compositor.md`.

use std::time::Duration;

use crate::{
    color::{Color, Palette},
    icon::Icon,
    metrics,
    motion::{self, Spring},
    typography,
};

/// Height of the bar, between the window border and the divider.
pub const BAR_HEIGHT: f32 = metrics::TITLE_BAR_HEIGHT;
/// Line under the bar: `border-bottom` of the boards.
pub const DIVIDER_WIDTH: f32 = 1.0;
/// What the decoration adds above the content of the client: the window border, the bar and
/// its divider. The content starts 42 below the top of the window on the Main board.
pub const INSET: f32 = metrics::BORDER_WIDTH + BAR_HEIGHT + DIVIDER_WIDTH;
/// Padding before the title, and after the buttons.
pub const PADDING: (f32, f32) = metrics::TITLE_BAR_PADDING;
/// Smallest gap between the end of the title and the buttons.
pub const TITLE_GAP: f32 = metrics::GAP_ICON;
/// The title: Inter, interface size, semibold, like the application name of the boards.
pub const TITLE_SIZE: f32 = typography::UI_SIZE;
pub const TITLE_WEIGHT: u16 = typography::WEIGHT_SEMIBOLD;
/// What ends a title too long for its bar.
pub const ELLIPSIS: char = '…';
/// A button, the gap between two, the padding and radius of their pill, the radius of a button.
/// Larger than the 28 × 24 of the boards (lot C-01): the author found them too small to hit.
/// 32 × 28 is the smallest target asked for; the pill (104 × 32) still fits the bar of 40.
pub const BUTTON: (f32, f32) = (32.0, 28.0);
pub const BUTTON_GAP: f32 = metrics::GAP_TIGHT;
pub const CONTROLS_PADDING: f32 = metrics::WINDOW_CONTROLS_PADDING;
/// The radii of the boards, grown with the buttons.
pub const CONTROLS_RADIUS: f32 = 10.0;
pub const BUTTON_RADIUS: f32 = 8.0;
/// Size of the icon of a button, drawn on the grid of `icon`: 16 instead of the 14 of the
/// boards, for the larger buttons.
pub const ICON_SIZE: f32 = metrics::ICON_SIZE;
/// Stroke of the icons, in units of the grid of `icon`: the 2 of the boards' window buttons,
/// a little more than the 1.7 of the other icons. 1.3 pixels at 16 px.
pub const ICON_STROKE: f32 = 2.0;
/// Scale of a button under the pointer, then held down: it grows a little, and sinks when
/// pressed. Both reached on `BUTTON_SPRING`.
pub const HOVER_SCALE: f32 = 1.06;
pub const PRESS_SCALE: f32 = 0.92;
/// The spring of the scale of a button: a slight bounce (ratio 0.79).
pub const BUTTON_SPRING: Spring = motion::PREVIEW_SPRING;
/// Fade of the color of a button, and of the icons when the pointer enters the pill: a hover.
pub const HOVER_FADE: Duration = motion::FADE;
/// Icon of the button of a maximized window, which gives it back its size: a square in front
/// of another one, on the grid of `icon`.
pub const RESTORE_ICON: &str = "M7 10h7v7H7zM10 10V7h7v7h-3";
/// Two presses on the same bar within this delay are a double click, which maximizes. The
/// usual value of GTK and KDE: `docs/contexte.md` gives none.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The three buttons, from left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Minimize,
    /// Maximize, or restore a maximized window.
    Maximize,
    Close,
}

impl Button {
    pub const ALL: [Self; 3] = [Self::Minimize, Self::Maximize, Self::Close];

    /// The path of its icon on the grid of `icon`. `maximized`: the window is maximized.
    pub const fn icon(self, maximized: bool) -> &'static str {
        match (self, maximized) {
            (Self::Minimize, _) => Icon::Minimize.path(),
            (Self::Maximize, false) => Icon::Maximize.path(),
            (Self::Maximize, true) => RESTORE_ICON,
            (Self::Close, _) => Icon::Close.path(),
        }
    }
}

/// The colors of the decoration in a palette: those of the title bar of Evidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub bar: Color,
    pub divider: Color,
    pub title: Color,
    /// The pill behind the buttons.
    pub controls: Color,
    /// A button under the pointer, then held down.
    pub hover: Color,
    pub pressed: Color,
    /// The icons at rest, quieter than the title.
    pub icon: Color,
    /// The icons once the pointer is on the pill: all three show.
    pub icon_shown: Color,
    /// The close button under the pointer: the red of destructive actions, and its icon.
    pub close_hover: Color,
    pub close_icon: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            bar: palette.title_bar,
            divider: palette.divider,
            title: palette.text_primary,
            controls: palette.controls,
            hover: palette.hover,
            pressed: palette.pressed,
            icon: palette.text_secondary,
            icon_shown: palette.text_primary,
            close_hover: palette.salmon,
            close_icon: palette.text_on_cream,
        }
    }
}

const _: () = {
    assert!(INSET == 42.0);
    // The pill fits in the bar.
    assert!(BUTTON.1 + 2.0 * CONTROLS_PADDING < BAR_HEIGHT);
    // A target of at least 32 × 28.
    assert!(BUTTON.0 >= 32.0 && BUTTON.1 >= 28.0);
    assert!(PRESS_SCALE < 1.0 && HOVER_SCALE > 1.0);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_reads_on_its_bar_in_both_themes() {
        for palette in [Palette::DARK, Palette::LIGHT] {
            let colors = Colors::of(&palette);
            assert_eq!(colors.bar.a, 1.0, "an opaque bar");
            assert!(colors.title.contrast(colors.bar) >= 4.5);
            assert!(colors.icon.contrast(colors.bar) >= 4.5);
            assert!(colors.icon_shown.contrast(colors.bar) >= colors.icon.contrast(colors.bar));
            // The icon of the close button reads on its red.
            assert!(colors.close_icon.contrast(colors.close_hover) >= 4.5);
        }
    }
}
