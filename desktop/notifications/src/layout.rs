//! Pure geometry of the stack of bubbles, in the coordinates of the surface, which the
//! compositor places below the area the top bar reserves (see `crate::surface`).

use sherlock_ui::notifications::{
    CLOSE_ON_CURVE, CLOSE_OUTSET, CLOSE_SIZE, GAP, PILE_DEPTH, PILE_STEP, SCREEN_EDGE, WIDTH,
};

/// Width of the surface: the room of « × » on the left (lot D-23f), a bubble and the room to
/// the screen edge, so that a bubble sliding out leaves the screen at its edge. The bubbles are
/// drawn `CLOSE_OUTSET` from the left and from the top of the surface.
pub const SURFACE_WIDTH: f32 = CLOSE_OUTSET + WIDTH + SCREEN_EDGE;
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

/// Where a bubble goes in the piles of its application (D-23d).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Place {
    /// Behind the newest bubble of its application, how deep (1, 2…).
    pub depth: usize,
    pub behind: bool,
    /// On the newest of a closed pile: how many bubbles are behind it.
    pub others: usize,
    /// Behind: the index of the newest bubble of its application.
    pub leader: usize,
}

/// The places of bubbles of these applications, from the newest down: the bubbles of an
/// application whose pile is not `open` go behind its newest one.
pub fn piles(apps: &[String], open: impl Fn(&str) -> bool) -> Vec<Place> {
    let mut places = vec![Place::default(); apps.len()];
    for (i, app) in apps.iter().enumerate() {
        if open(app) {
            continue;
        }
        let Some(first) = apps.iter().position(|a| a == app) else {
            continue;
        };
        if first < i {
            places[first].others += 1;
            let depth = places[first].others;
            places[i] = Place {
                depth,
                behind: true,
                others: 0,
                leader: first,
            };
        }
    }
    places
}

/// The top of each bubble, as `stack` does, the piles laid together: a bubble behind another
/// goes `PILE_STEP` lower per level (`PILE_DEPTH` at most), and the pile takes that room.
pub fn stack_piled(heights: &[f32], places: &[Place]) -> Vec<f32> {
    let mut top = 0.0;
    let mut tops = vec![0.0; heights.len()];
    let mut front: Vec<Option<f32>> = vec![None; heights.len()];
    for (i, height) in heights.iter().enumerate() {
        let place = places.get(i).copied().unwrap_or_default();
        if place.behind {
            // The newest of its application is above it in the list.
            if let Some(Some(leader)) = front.get(place.leader) {
                let depth = place.depth.min(PILE_DEPTH) as f32;
                tops[i] = leader + depth * PILE_STEP;
                continue;
            }
        }
        tops[i] = top;
        front[i] = Some(top);
        let room = place.others.min(PILE_DEPTH) as f32 * PILE_STEP;
        top += height + room + GAP;
    }
    tops
}

/// Height of the surface holding bubbles at these `(top, height)`, in whole pixels, one at
/// least: a surface cannot be empty.
pub fn surface_height(bubbles: impl IntoIterator<Item = (f32, f32)>) -> u32 {
    let bottom = bubbles
        .into_iter()
        .map(|(top, height)| CLOSE_OUTSET + top + height)
        .fold(0.0_f32, f32::max);
    (bottom.ceil() as u32).max(1)
}

/// Where the surface takes the pointer: the rectangle of each bubble at `(top, height)`, and the
/// square of its « × » on its top left corner (lot D-23f), in whole pixels rounded out. Between
/// and beside the bubbles, clicks go to the window below.
pub fn region(bubbles: impl IntoIterator<Item = (f32, f32)>) -> Vec<(i32, i32, i32, i32)> {
    // The badge's square, rounded out: from the curve of the corner, less half the badge.
    let (inset, side) = (
        CLOSE_ON_CURVE.floor() as i32,
        (CLOSE_SIZE + 1.0).ceil() as i32,
    );
    let outset = CLOSE_OUTSET as i32;
    bubbles
        .into_iter()
        .flat_map(|(top, height)| {
            let y = (CLOSE_OUTSET + top).floor() as i32;
            let bottom = (CLOSE_OUTSET + top + height).ceil() as i32;
            [
                (outset, y, WIDTH.ceil() as i32, bottom - y),
                (inset, y - outset + inset, side, side),
            ]
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
        assert_eq!(surface_height(spans), 277 + CLOSE_OUTSET as u32);
        assert_eq!(surface_height([]), 1);
    }

    #[test]
    fn the_bubbles_of_one_application_pile_up_unless_its_pile_is_open() {
        let mixed: Vec<String> = ["Mail", "Chat", "Chat", "Mail"].map(String::from).to_vec();
        let places = piles(&mixed, |_| false);
        let tops = stack_piled(&[80.0, 60.0, 60.0, 80.0], &places);
        assert_eq!(tops[3], PILE_STEP, "behind its own application's bubble");
        let apps: Vec<String> = ["Mail", "Chat", "Mail", "Mail", "Mail"]
            .map(String::from)
            .to_vec();
        let places = piles(&apps, |_| false);
        assert_eq!(places[0].others, 3);
        assert!(!places[1].behind && places[1].others == 0);
        assert_eq!(
            places[2..]
                .iter()
                .map(|p| (p.behind, p.depth))
                .collect::<Vec<_>>(),
            [(true, 1), (true, 2), (true, 3)]
        );
        let tops = stack_piled(&[80.0, 60.0, 70.0, 70.0, 70.0], &places);
        let pile = PILE_DEPTH as f32 * PILE_STEP;
        // « Chat » under the pile of « Mail » and its room.
        assert_eq!(tops[1], 80.0 + pile + GAP);
        assert_eq!(tops[2], PILE_STEP);
        assert_eq!(tops[4], pile, "deeper ones stay at the last level");
        // Opened, the pile is a plain stack.
        let open = piles(&apps, |app| app == "Mail");
        assert!(open.iter().all(|p| !p.behind && p.others == 0));
        assert_eq!(stack_piled(&[80.0, 60.0], &open[..2]), stack(&[80.0, 60.0]));
    }

    #[test]
    fn the_pointer_goes_through_the_gaps() {
        let region = region([(0.0, 80.0), (88.0, 120.5)]);
        // Each bubble, then the square of its « × », half outside its top left corner.
        assert_eq!(
            region,
            [
                (8, 8, 320, 80),
                (4, 4, 17, 17),
                (8, 96, 320, 121),
                (4, 92, 17, 17)
            ]
        );
        // The room on the right, towards the screen edge, takes no click either.
        assert!(
            region
                .iter()
                .all(|(x, _, w, _)| x + w < SURFACE_WIDTH as i32)
        );
    }
}
