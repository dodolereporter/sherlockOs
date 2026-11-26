//! Mission Control (lot D-33): the compositor shows every window of the active workspace side by
//! side. No board draws it: the measures below are choices of
//! the lot, taken from the dock label and the window tokens, to review with the design. See
//! "Mission Control" in `docs/compositor.md`.

use crate::{
    color::{Color, Palette},
    dock,
    motion::{self, Spring},
};

/// Opening and closing: the windows reach their place in the grid, or their own, on the spring
/// of a snap (ratio 0.75, overshoot of 3 %).
pub const SPRING: Spring = motion::SNAP_SPRING;
/// Space left between the work area and the grid of the windows.
pub const MARGIN: f32 = 32.0;
/// Gap between two windows of the grid, side by side and from one row to the next.
pub const GAP: f32 = 24.0;
/// Largest scale of a window in the grid: even alone, it shrinks a little, so that the view
/// reads as a view.
pub const MAX_SCALE: f64 = 0.8;
/// Border of the accent color around the window under the pointer, outside its frame.
pub const HIGHLIGHT_WIDTH: f32 = 3.0;
/// Title under each window: its gap to the window, its height, its side padding and radius.
pub const LABEL_GAP: f32 = 8.0;
pub const LABEL_HEIGHT: f32 = 24.0;
pub const LABEL_PADDING: f32 = dock::LABEL_PADDING.1;
pub const LABEL_RADIUS: f32 = dock::LABEL_RADIUS;

/// The colors of the view in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Laid over the wallpaper, fully once the view is open.
    pub dim: Color,
    pub highlight: Color,
    pub label: Color,
    pub label_text: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            dim: palette.scrim,
            highlight: palette.accent,
            label: palette.menu_glass,
            label_text: palette.text_primary,
        }
    }
}
