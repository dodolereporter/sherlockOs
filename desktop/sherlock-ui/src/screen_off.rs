//! The screen that goes off and comes back like an old television (lot D-38): the picture is
//! squashed from the top and the bottom into a bright line in the middle, the line shrinks to a
//! dot, and the dot fades out. The compositor plays it before the sleep, when the lid closes,
//! when the screen goes off for inactivity or by its shortcut, and backwards when it comes
//! back. No board draws it: the measures below are choices of the lot.

use std::time::Duration;

use crate::color::{Color, Palette};

/// The whole animation.
pub const DURATION: Duration = Duration::from_millis(350);
/// Share of `DURATION` the picture takes to become a line, then the line to become a dot; the
/// rest, the dot fades out.
pub const SQUASH: f32 = 0.55;
pub const SHRINK: f32 = 0.85;
/// Thickness of the line and side of the dot, in logical pixels.
pub const LINE: f32 = 3.0;
/// The halo around the line and the dot: its size relative to them, and its opacity.
pub const HALO: f32 = 4.0;
pub const HALO_OPACITY: f32 = 0.28;
/// How bright the picture gets while it is squashed: the opacity of the light laid over it at
/// the end of the squash.
pub const GLOW: f32 = 0.55;

/// The light of the line, of the dot and of the glow: the cream of Sherlock, almost white.
pub const fn light(palette: &Palette) -> Color {
    palette.cream
}

/// The veil before the screens go off or the session locks for inactivity, as on macOS: it
/// shows `DIM_LEAD` before, darkens the screens to `DIM` (the opacity of its black) over
/// `DIM_FADE`, and the slightest input lifts it at once. A veil of the compositor, not the
/// backlight.
pub const DIM: f32 = 0.35;
pub const DIM_LEAD: Duration = Duration::from_secs(5);
pub const DIM_FADE: Duration = Duration::from_millis(1500);

/// What fills the screen around the picture: black, as a television off.
pub const BLACK: Color = Color::rgb(0, 0, 0);
