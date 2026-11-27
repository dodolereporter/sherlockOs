//! The blur behind glass (lot D-37, ADR 0027): light, as the "vibrancy" of macOS. A glass
//! surface asks the compositor for it with `ext_background_effect_v1`, over its rounded shape
//! (`rounded_region`); the compositor blurs what is behind with a dual Kawase filter and draws
//! it under the surface. Where it cannot blur, it lays a veil of the desktop color instead, so
//! that the glass still reads as denser. See "Blur behind glass" in `docs/compositor.md`.

/// Halvings of the picture behind, after the first one: the blur grows with each.
pub const PASSES: usize = 2;
/// Distance between the samples of each pass, in pixels of the reduced picture.
pub const OFFSET: f32 = 2.0;
/// Saturation of the blurred picture: 1 leaves it as is, more makes its colors livelier.
pub const SATURATION: f32 = 1.3;
/// Opacity of the veil of the desktop color laid behind glass when there is no blur: the
/// software renderer, or a graphics chip that cannot do it.
pub const VEIL_OPACITY: f32 = 0.7;

/// The rectangles, `[x, y, width, height]` in logical pixels, that cover a rectangle of
/// `width` × `height` at (`x`, `y`) rounded by `radius`: a band per row in the corners, where
/// the curve covers at least half of the pixel, and one block between them. A Wayland region
/// is made of rectangles only; the glass drawn over it smooths the edge.
pub fn rounded_region(x: i32, y: i32, width: i32, height: i32, radius: f32) -> Vec<[i32; 4]> {
    if width <= 0 || height <= 0 {
        return Vec::new();
    }
    let r = radius.max(0.0).min(width.min(height) as f32 / 2.0);
    let rows = r.ceil() as i32;
    // Room left of the curve on the row `row` from the top of a corner.
    let inset = |row: i32| {
        let dy = r - (row as f32 + 0.5);
        (r - (r * r - dy * dy).max(0.0).sqrt()).round() as i32
    };
    let mut rects: Vec<[i32; 4]> = Vec::new();
    let mut push = |top: i32, rows: i32, inset: i32| {
        // Rows of the same inset make one rectangle.
        match rects.last_mut() {
            Some(last) if last[0] == x + inset && last[1] + last[3] == top => last[3] += rows,
            _ => rects.push([x + inset, top, width - 2 * inset, rows]),
        }
    };
    for row in 0..rows {
        push(y + row, 1, inset(row));
    }
    push(y + rows, height - 2 * rows, 0);
    for row in (0..rows).rev() {
        push(y + height - 1 - row, 1, inset(row));
    }
    rects.retain(|r| r[2] > 0 && r[3] > 0);
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covers(rects: &[[i32; 4]], px: i32, py: i32) -> bool {
        rects
            .iter()
            .any(|r| (r[0]..r[0] + r[2]).contains(&px) && (r[1]..r[1] + r[3]).contains(&py))
    }

    #[test]
    fn a_square_rectangle_is_one_block() {
        assert_eq!(
            rounded_region(10, 20, 100, 50, 0.0),
            vec![[10, 20, 100, 50]]
        );
        assert!(rounded_region(0, 0, 0, 50, 8.0).is_empty());
    }

    #[test]
    fn the_corners_follow_the_curve() {
        let rects = rounded_region(0, 0, 200, 76, 22.0);
        // Inside, and the middle of each edge: covered.
        for (px, py) in [(100, 38), (0, 38), (199, 38), (100, 0), (100, 75)] {
            assert!(covers(&rects, px, py), "({px}, {py})");
        }
        // The very corners: outside the curve.
        for (px, py) in [(0, 0), (199, 0), (0, 75), (199, 75), (3, 3)] {
            assert!(!covers(&rects, px, py), "({px}, {py})");
        }
        // On the diagonal, past the curve by a pixel: covered.
        let d = (22.0 - 22.0 / 2f32.sqrt()).ceil() as i32 + 1;
        assert!(covers(&rects, d, d));
        // No pixel twice, nothing outside: the area is that of the rounded rectangle.
        let area: i32 = rects.iter().map(|r| r[2] * r[3]).sum();
        let exact = 200.0 * 76.0 - (4.0 - std::f32::consts::PI) * 22.0 * 22.0;
        assert!((area as f32 - exact).abs() < 30.0, "{area} {exact}");
        // Rows of the same inset are merged: far fewer rectangles than rows.
        assert!(rects.len() < 2 * 22 + 1, "{}", rects.len());
    }

    #[test]
    fn a_radius_larger_than_the_side_makes_a_pill() {
        let rects = rounded_region(0, 0, 100, 20, 50.0);
        assert!(covers(&rects, 50, 0) && covers(&rects, 50, 19));
        assert!(!covers(&rects, 0, 0));
        assert!(covers(&rects, 0, 10) || covers(&rects, 1, 10));
    }
}
