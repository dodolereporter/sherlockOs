//! Pure geometry of the stack of bubbles, in the coordinates of the surface, which the
//! compositor places below the area the top bar reserves (see `crate::surface`).

use sherlock_ui::notifications::{GAP, SCREEN_EDGE, WIDTH};

/// Width of the surface: a bubble and the room to the screen edge, so that a bubble sliding
/// out leaves the screen at its edge.
pub const SURFACE_WIDTH: f32 = WIDTH + SCREEN_EDGE;
/// Offset of a bubble out of sight, to the right: where it slides in from and out to.
pub const OUT: f32 = SURFACE_WIDTH;

/// The top of each bubble, for bubbles of these heights from the top of the stack down.
pub fn stack(heights: &[f32]) -> Vec<f32> {
    let mut top = 0.0;
    heights
        .iter()
        .map(|height| {
            let at = top;
            top += height + GAP;
            at
        })
        .collect()
}

/// Height of the surface holding bubbles at these `(top, height)`, in whole pixels, one at
/// least: a surface cannot be empty.
pub fn surface_height(bubbles: impl IntoIterator<Item = (f32, f32)>) -> u32 {
    let bottom = bubbles
        .into_iter()
        .map(|(top, height)| top + height)
        .fold(0.0_f32, f32::max);
    (bottom.ceil() as u32).max(1)
}

/// Where the surface takes the pointer: the rectangle of each bubble at `(top, height)`, in
/// whole pixels rounded out. Between and beside the bubbles, clicks go to the window below.
pub fn region(bubbles: impl IntoIterator<Item = (f32, f32)>) -> Vec<(i32, i32, i32, i32)> {
    bubbles
        .into_iter()
        .map(|(top, height)| {
            let y = top.floor() as i32;
            let bottom = (top + height).ceil() as i32;
            (0, y, WIDTH.ceil() as i32, bottom - y)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubbles_stack_downwards_with_the_board_gap() {
        assert_eq!(stack(&[]), Vec::<f32>::new());
        assert_eq!(stack(&[80.0, 120.5, 60.0]), [0.0, 88.0, 216.5]);
        let tops = stack(&[80.0, 120.5, 60.0]);
        let spans = tops.iter().copied().zip([80.0, 120.5, 60.0]);
        assert_eq!(surface_height(spans), 277);
        assert_eq!(surface_height([]), 1);
    }

    #[test]
    fn the_pointer_goes_through_the_gaps() {
        let region = region([(0.0, 80.0), (88.0, 120.5)]);
        assert_eq!(region, [(0, 0, 320, 80), (0, 88, 320, 121)]);
        // The room on the right, towards the screen edge, takes no click either.
        assert!(
            region
                .iter()
                .all(|(x, _, w, _)| x + w < SURFACE_WIDTH as i32)
        );
    }
}
