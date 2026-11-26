//! Colors and motion of the cursor, from the cursor boards of the author
//! (`docs/design/cursors-dark.html` and `docs/design/cursors-light.html`). The drawings are
//! the SVGs of `assets/cursors/`; the generator paints them with these colors.

use std::time::Duration;

use crate::color::{self, Color};

/// Colors of the drawings in a desktop theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorColors {
    /// Inside of the shapes, and the lines of the line drawings.
    pub body: Color,
    /// The outline that detaches the cursor from any background.
    pub edge: Color,
    /// The ring of the waiting cursor, under its green arc.
    pub track: Color,
}

/// Dark theme: cream cursors outlined in the darkest blue.
pub const DARK: CursorColors = CursorColors {
    body: color::CREAM,
    edge: color::DEEPEST,
    track: Color::rgba(0xF4, 0xEB, 0xE3, 0.30),
};

/// Light theme: night blue cursors outlined in white.
pub const LIGHT: CursorColors = CursorColors {
    body: color::TEXT_ON_CREAM,
    edge: Color::rgb(0xFF, 0xFF, 0xFF),
    track: Color::rgba(0x14, 0x23, 0x3B, 0.22),
};

/// Names and descriptions of the two cursor themes, as a cursor setting lists them.
pub const DARK_NAME: &str = "SherlockOS";
pub const DARK_COMMENT: &str = "Curseurs de SherlockOS pour le bureau sombre";
pub const LIGHT_NAME: &str = "SherlockOS clair";
pub const LIGHT_COMMENT: &str = "Curseurs de SherlockOS pour le bureau clair";

/// The arc of the waiting cursors makes a turn in this time.
pub const SPIN_TURN: Duration = Duration::from_secs(1);

/// Shape change: the new shape fades in from this opacity and scale, with a light bounce.
pub const SHAPE_CHANGE: Duration = Duration::from_millis(120);
pub const SHAPE_FROM_OPACITY: f32 = 0.3;
pub const SHAPE_FROM_SCALE: f32 = 0.8;

/// Shake to find: so many changes of direction within `SHAKE_WINDOW`, each move longer than
/// `SHAKE_STEP` logical pixels, make the cursor `SHAKE_SCALE` times larger. It stays so until
/// `SHAKE_HOLD` after the last shake, then comes back with a bounce. Both ways take
/// `SHAKE_TRANSITION`.
pub const SHAKE_WINDOW: Duration = Duration::from_millis(700);
pub const SHAKE_STEP: f64 = 6.0;
pub const SHAKE_TURNS: usize = 4;
pub const SHAKE_SCALE: f32 = 2.6;
pub const SHAKE_HOLD: Duration = Duration::from_secs(1);
pub const SHAKE_TRANSITION: Duration = Duration::from_millis(240);

/// The curve of the cursor's transitions, `cubic-bezier(0.2, 0.9, 0.3, 1.35)` in the boards:
/// fast, past the end, then back. `t` from 0 to 1.
pub fn bounce(t: f32) -> f32 {
    const P: [f32; 4] = [0.2, 0.9, 0.3, 1.35];
    let t = t.clamp(0.0, 1.0);
    let curve = |a: f32, b: f32, s: f32| {
        let r = 1.0 - s;
        3.0 * r * r * s * a + 3.0 * r * s * s * b + s * s * s
    };
    // x grows with s: the parameter whose x is `t`, by bisection.
    let (mut low, mut high) = (0.0_f32, 1.0_f32);
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        if curve(P[0], P[2], mid) < t {
            low = mid;
        } else {
            high = mid;
        }
    }
    curve(P[1], P[3], (low + high) / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bounce_starts_and_ends_in_place_and_overshoots_between() {
        assert!(bounce(0.0).abs() < 1e-3);
        assert!((bounce(1.0) - 1.0).abs() < 1e-3);
        let peak = (1..100)
            .map(|i| bounce(i as f32 / 100.0))
            .fold(0.0, f32::max);
        assert!(peak > 1.02 && peak < 1.2, "{peak}");
    }

    #[test]
    fn the_outline_contrasts_with_the_body() {
        for colors in [DARK, LIGHT] {
            assert!(colors.body.contrast(colors.edge) > 4.5);
        }
    }
}
