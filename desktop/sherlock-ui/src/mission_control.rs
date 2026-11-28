//! Mission Control (lot D-33): the compositor shows every window of the active workspace side by
//! side, under the strip of the workspaces. No board draws it: the measures below are choices of
//! the lot, taken from the dock label and the window tokens, to review with the design. See
//! "Mission Control" in `docs/compositor.md`.

use crate::{
    color::{Color, Palette},
    dock, metrics,
    motion::{self, Spring},
};

/// Opening, closing, and the end of a three finger swipe: the windows reach their place in the
/// grid, or their own, on the spring of a snap (ratio 0.75, overshoot of 3 %).
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
/// Strip of the workspaces, at the top of the work area: height of a thumbnail (its width
/// follows the screen), margin above and below, gap between two thumbnails.
pub const STRIP_HEIGHT: f32 = 84.0;
pub const STRIP_MARGIN: f32 = 16.0;
pub const THUMBNAIL_GAP: f32 = 16.0;
pub const THUMBNAIL_RADIUS: f32 = 8.0;
/// Border of a thumbnail: that of a window, and twice as thick in the accent color for the
/// active workspace and for the one a dragged window is over.
pub const THUMBNAIL_BORDER: f32 = metrics::BORDER_WIDTH;
pub const THUMBNAIL_ACTIVE_BORDER: f32 = 2.0;
/// The « × » that closes the window under the pointer (lot D-33c): its diameter, its center
/// this far inside the top left corner of the window (most of it outside, as on macOS), and
/// how long the title and it take to fade in or out.
pub const CLOSE_SIZE: f32 = 24.0;
pub const CLOSE_INSET: f32 = 2.0;
pub const HOVER_FADE: std::time::Duration = std::time::Duration::from_millis(150);

/// The colors of the view in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Laid over the wallpaper, fully once the view is open.
    pub dim: Color,
    pub highlight: Color,
    pub label: Color,
    pub label_text: Color,
    /// Behind a thumbnail, when no wallpaper is loaded.
    pub thumbnail: Color,
    pub thumbnail_border: Color,
    pub active: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            dim: palette.scrim,
            highlight: palette.accent,
            label: palette.menu_glass,
            label_text: palette.text_primary,
            thumbnail: palette.desktop,
            thumbnail_border: palette.border,
            active: palette.accent,
        }
    }
}
