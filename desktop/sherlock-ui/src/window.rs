//! Window handling by the compositor: grabbing the edges, snapping against the screen.
//!
//! Sizes are in logical pixels. `docs/contexte.md` does not set them: they are choices of lot
//! D-17, to review with the design.

use crate::color::{self, Color};

/// Width of the grab band, outside each side of a window.
pub const RESIZE_BAND: i32 = 8;

/// Size of a corner grab: along the sides from the corner, and diagonally outside the window.
/// Wider than the band, so that a corner can be found without aiming at the pixel.
pub const RESIZE_CORNER: i32 = 20;

/// Distance to the screen edge below which a dropped window snaps.
pub const SNAP_EDGE: i32 = 8;

/// Distance to a screen corner, along an edge, below which a dropped window takes that
/// quarter of the screen.
pub const SNAP_CORNER: i32 = 64;

/// Background of the snap preview: that of an active entry.
pub const SNAP_PREVIEW_FILL: Color = color::ACTIVE_ENTRY;

/// Border of the snap preview.
pub const SNAP_PREVIEW_BORDER: Color = color::ACCENT;

/// Width of the snap preview border.
pub const SNAP_PREVIEW_BORDER_WIDTH: i32 = 2;

/// Distance the pointer must travel before a move shifts the window: a plain click neither
/// moves it nor detaches it from its zone.
pub const DRAG_THRESHOLD: i32 = 4;

/// Smallest work area that anchored surfaces leave to windows on a screen, in logical pixels
/// (lot D-12): however large their exclusive zones, a maximized window gets at least this much,
/// unless the screen itself is smaller. A choice of the lot, to review with the design.
pub const MIN_WORK_AREA: (i32, i32) = (320, 240);
