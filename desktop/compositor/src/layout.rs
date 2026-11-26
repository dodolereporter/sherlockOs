//! Window layout: where to place windows, how to move, resize and snap them.
//!
//! Pure computation on rectangles, without Wayland or rendering, so that it can be tested alone.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Edges pulled during a resize.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    /// Reads the bits of `xdg_toplevel.resize_edge`: top 1, bottom 2, left 4, right 8.
    pub fn from_xdg_bits(bits: u32) -> Self {
        Self {
            top: bits & 1 != 0,
            bottom: bits & 2 != 0,
            left: bits & 4 != 0,
            right: bits & 8 != 0,
        }
    }

    /// Edges nearest to the pointer: the corner of the window quarter it is in gets pulled.
    pub fn nearest(window: Rectangle<i32, Logical>, pointer: Point<f64, Logical>) -> Self {
        let center = window.loc.to_f64() + window.size.to_f64().downscale(2.0).to_point();
        Self {
            top: pointer.y < center.y,
            bottom: pointer.y >= center.y,
            left: pointer.x < center.x,
            right: pointer.x >= center.x,
        }
    }
}

/// Edges grabbed in the band around a window, outside its geometry `window`.
///
/// The band is `band` pixels wide along the sides. At the corners, it widens to `corner` pixels,
/// diagonally outside the window as well as along the sides: a press near a corner pulls the
/// corner.
pub fn border_edges(
    window: Rectangle<i32, Logical>,
    pointer: Point<f64, Logical>,
    band: i32,
    corner: i32,
) -> Option<Edges> {
    let (x0, y0) = (f64::from(window.loc.x), f64::from(window.loc.y));
    let (w, h) = (f64::from(window.size.w), f64::from(window.size.h));
    let (x1, y1) = (x0 + w, y0 + h);
    // Distance outside the window on each axis, zero in front of it.
    let dx = (x0 - pointer.x).max(pointer.x - x1).max(0.0);
    let dy = (y0 - pointer.y).max(pointer.y - y1).max(0.0);
    let inside = (x0..x1).contains(&pointer.x) && (y0..y1).contains(&pointer.y);
    let reach = if dx > 0.0 && dy > 0.0 { corner } else { band };
    if inside || dx.max(dy) > f64::from(reach) {
        return None;
    }
    // A narrow window shares its sides between the two corners.
    let (kx, ky) = (
        f64::from(corner).min(w / 2.0),
        f64::from(corner).min(h / 2.0),
    );
    let left = pointer.x < x0 + kx;
    let top = pointer.y < y0 + ky;
    Some(Edges {
        top,
        bottom: !top && pointer.y >= y1 - ky,
        left,
        right: !left && pointer.x >= x1 - kx,
    })
}

/// Zone of the screen to snap a window into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// The whole screen: the window is maximized.
    Maximized,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Zone {
    /// Zone targeted by a window dropped with the pointer at `pointer`: against the top, left or
    /// right edge of the screen (within `edge` pixels), or in a corner (within `corner` pixels
    /// of that corner, along an edge). The bottom edge alone snaps nothing.
    pub fn under(
        area: Rectangle<i32, Logical>,
        pointer: Point<f64, Logical>,
        edge: i32,
        corner: i32,
    ) -> Option<Self> {
        let x = pointer.x - f64::from(area.loc.x);
        let y = pointer.y - f64::from(area.loc.y);
        let (w, h) = (f64::from(area.size.w), f64::from(area.size.h));
        let near = |limit: i32| {
            let limit = f64::from(limit);
            (x < limit, x >= w - limit, y < limit, y >= h - limit)
        };
        let (left, right, top, bottom) = near(edge);
        let (c_left, c_right, c_top, c_bottom) = near(corner);
        Some(if (left && c_top) || (top && c_left) {
            Self::TopLeft
        } else if (right && c_top) || (top && c_right) {
            Self::TopRight
        } else if (left && c_bottom) || (bottom && c_left) {
            Self::BottomLeft
        } else if (right && c_bottom) || (bottom && c_right) {
            Self::BottomRight
        } else if top {
            Self::Maximized
        } else if left {
            Self::Left
        } else if right {
            Self::Right
        } else {
            return None;
        })
    }

    /// Rectangle taken by a window snapped into this zone of `area`, `gap` away from the edges
    /// of the area and from the window in the next zone. A maximized window takes the whole
    /// area, without a gap. An odd pixel goes to the right or to the bottom.
    pub fn geometry(self, area: Rectangle<i32, Logical>, gap: i32) -> Rectangle<i32, Logical> {
        // Start and length of the first and of the second half along an axis.
        let halves = |start: i32, len: i32| {
            let free = (len - 3 * gap).max(0);
            let first = free / 2;
            (
                (start + gap, first),
                (start + 2 * gap + first, free - first),
            )
        };
        let Point { x, y, .. } = area.loc;
        let Size { w, h, .. } = area.size;
        let (left, right) = halves(x, w);
        let (top, bottom) = halves(y, h);
        let whole = (y + gap, (h - 2 * gap).max(0));
        let ((x, w), (y, h)) = match self {
            Self::Maximized => return area,
            Self::Left => (left, whole),
            Self::Right => (right, whole),
            Self::TopLeft => (left, top),
            Self::TopRight => (right, top),
            Self::BottomLeft => (left, bottom),
            Self::BottomRight => (right, bottom),
        };
        Rectangle::new((x, y).into(), (w, h).into())
    }
}

/// Work area of `screen` from the zone that anchored surfaces leave (`zone`, in the same
/// coordinates). Kept inside the screen, and never smaller than `min` on each axis (or the
/// screen, if smaller): exclusive zones that cover the screen leave a usable area at their
/// edge, never a negative or empty one. Saturating: the zone comes from clients.
pub fn work_area(
    screen: Rectangle<i32, Logical>,
    zone: Rectangle<i32, Logical>,
    min: (i32, i32),
) -> Rectangle<i32, Logical> {
    let axis = |start: i32, len: i32, z_start: i32, z_len: i32, min: i32| {
        let end = start.saturating_add(len.max(0));
        let min = min.clamp(0, end - start);
        let z_start = z_start.clamp(start, end);
        let z_end = z_start.saturating_add(z_len).clamp(z_start, end);
        if z_end - z_start >= min {
            (z_start, z_end - z_start)
        } else {
            (z_start.min(end - min), min)
        }
    };
    let (x, w) = axis(screen.loc.x, screen.size.w, zone.loc.x, zone.size.w, min.0);
    let (y, h) = axis(screen.loc.y, screen.size.h, zone.loc.y, zone.size.h, min.1);
    Rectangle::new((x, y).into(), (w, h).into())
}

/// Position of a window pulled out of its zone by moving it: it gets its size `size` back under
/// the pointer, grabbed at the same proportion of its width and at the same distance from its
/// top edge (within its new height).
pub fn unsnapped(
    current: Rectangle<i32, Logical>,
    size: Size<i32, Logical>,
    pointer: Point<f64, Logical>,
) -> Point<i32, Logical> {
    let ratio = if current.size.w > 0 {
        ((pointer.x - f64::from(current.loc.x)) / f64::from(current.size.w)).clamp(0.0, 1.0)
    } else {
        0.5
    };
    let from_top = (pointer.y - f64::from(current.loc.y)).clamp(0.0, f64::from(size.h.max(0)));
    Point::<f64, Logical>::from((pointer.x - ratio * f64::from(size.w), pointer.y - from_top))
        .to_i32_round()
}

/// Position that centers a window in the area, without its top left corner leaving it.
pub fn centered(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    let x = area.loc.x + ((area.size.w - size.w) / 2).max(0);
    let y = area.loc.y + ((area.size.h - size.h) / 2).max(0);
    (x, y).into()
}

/// Position of a moved window: it follows the pointer from the start of the gesture.
pub fn moved(
    initial: Point<i32, Logical>,
    grab_start: Point<f64, Logical>,
    pointer: Point<f64, Logical>,
) -> Point<i32, Logical> {
    (initial.to_f64() + (pointer - grab_start)).to_i32_round()
}

/// Size requested during a resize, bounded by those the window accepts.
///
/// `min` and `max` come from the client; 0 means no bound.
pub fn resized(
    initial: Rectangle<i32, Logical>,
    edges: Edges,
    delta: Point<f64, Logical>,
    min: Size<i32, Logical>,
    max: Size<i32, Logical>,
) -> Size<i32, Logical> {
    let grow = |size: i32, d: f64, start: bool, end: bool| -> i32 {
        match (start, end) {
            (true, _) => size - d.round() as i32,
            (false, true) => size + d.round() as i32,
            (false, false) => size,
        }
    };
    let bound = |value: i32, min: i32, max: i32| -> i32 {
        let max = if max <= 0 { i32::MAX } else { max };
        value.min(max).max(min.max(1))
    };
    let w = grow(initial.size.w, delta.x, edges.left, edges.right);
    let h = grow(initial.size.h, delta.y, edges.top, edges.bottom);
    (bound(w, min.w, max.w), bound(h, min.h, max.h)).into()
}

/// Position of a resized window: the edge opposite the pulled one stays in place.
pub fn anchored(
    initial: Rectangle<i32, Logical>,
    edges: Edges,
    size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let mut loc = initial.loc;
    if edges.left {
        loc.x += initial.size.w - size.w;
    }
    if edges.top {
        loc.y += initial.size.h - size.h;
    }
    loc
}

/// Resize of a window, from the first motion to the client's last commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resize {
    pub edges: Edges,
    /// Geometry of the window at the start of the gesture.
    pub initial: Rectangle<i32, Logical>,
    /// The button is released: the client's next commit is the last of the gesture.
    pub done: bool,
}

impl Resize {
    /// On each new content from the client: position to give the window, and what to keep for
    /// the next commit.
    pub fn on_commit(self, size: Size<i32, Logical>) -> (Point<i32, Logical>, Option<Self>) {
        let next = (!self.done).then_some(self);
        (anchored(self.initial, self.edges, size), next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn the_work_area_stays_in_the_screen_and_is_never_empty() {
        let screen = rect(800, 0, 800, 600);
        let min = (320, 240);
        // smithay builds the zone field by field: it can be negative, which `Size::new`
        // refuses.
        let raw = |x, y, w, h| {
            let mut zone = rect(0, 0, 0, 0);
            (zone.loc.x, zone.loc.y, zone.size.w, zone.size.h) = (x, y, w, h);
            zone
        };
        // A bar of 46 at the top: the rest of the screen.
        assert_eq!(
            work_area(screen, rect(800, 46, 800, 554), min),
            rect(800, 46, 800, 554)
        );
        // Zones larger than the screen: a negative zone becomes the minimum, at its edge,
        // inside the screen.
        assert_eq!(
            work_area(screen, raw(800, 100_000, 800, -99_400), min),
            rect(800, 360, 800, 240)
        );
        assert_eq!(
            work_area(screen, rect(800, 0, 800, 0), min),
            rect(800, 0, 800, 240)
        );
        assert_eq!(
            work_area(screen, raw(i32::MAX, i32::MIN, i32::MAX, i32::MIN), min),
            rect(1280, 0, 320, 240)
        );
        // A screen smaller than the minimum: the whole screen.
        assert_eq!(
            work_area(rect(0, 0, 200, 100), rect(0, 50, 200, 0), min),
            rect(0, 0, 200, 100)
        );
    }

    #[test]
    fn centers_the_window_in_the_area() {
        assert_eq!(
            centered(rect(0, 0, 1000, 800), (400, 200).into()),
            (300, 300).into()
        );
        assert_eq!(
            centered(rect(100, 50, 1000, 800), (400, 200).into()),
            (400, 350).into()
        );
    }

    #[test]
    fn a_window_too_large_stays_in_the_area() {
        assert_eq!(
            centered(rect(10, 20, 300, 200), (600, 400).into()),
            (10, 20).into()
        );
    }

    #[test]
    fn moving_follows_the_pointer() {
        let start = Point::from((50.0, 50.0));
        assert_eq!(
            moved((100, 100).into(), start, (80.4, 20.6).into()),
            (130, 71).into()
        );
        assert_eq!(moved((100, 100).into(), start, start), (100, 100).into());
    }

    #[test]
    fn reads_the_xdg_edges() {
        let bottom_right = Edges::from_xdg_bits(10);
        assert!(bottom_right.bottom && bottom_right.right);
        assert!(!bottom_right.top && !bottom_right.left);
        assert_eq!(Edges::from_xdg_bits(0), Edges::default());
    }

    #[test]
    fn picks_the_nearest_corner() {
        let window = rect(0, 0, 200, 100);
        let top_left = Edges::nearest(window, (10.0, 10.0).into());
        assert!(top_left.top && top_left.left && !top_left.bottom && !top_left.right);
        let bottom_right = Edges::nearest(window, (190.0, 90.0).into());
        assert!(bottom_right.bottom && bottom_right.right);
    }

    #[test]
    fn pulling_right_and_down_enlarges() {
        let edges = Edges::from_xdg_bits(10);
        let size = resized(
            rect(0, 0, 200, 100),
            edges,
            (30.0, -20.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(size, (230, 80).into());
    }

    #[test]
    fn pulling_left_and_up_keeps_the_opposite_corner() {
        let initial = rect(100, 100, 200, 100);
        let edges = Edges::from_xdg_bits(5);
        let size = resized(
            initial,
            edges,
            (-50.0, 10.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(size, (250, 90).into());
        assert_eq!(anchored(initial, edges, size), (50, 110).into());
        // The bottom right edge has not moved.
        assert_eq!(
            anchored(initial, edges, size) + size.to_point(),
            (300, 200).into()
        );
    }

    #[test]
    fn respects_the_client_sizes() {
        let edges = Edges::from_xdg_bits(10);
        let initial = rect(0, 0, 200, 100);
        let small = resized(
            initial,
            edges,
            (-500.0, -500.0).into(),
            (120, 60).into(),
            (0, 0).into(),
        );
        assert_eq!(small, (120, 60).into());
        let big = resized(
            initial,
            edges,
            (500.0, 500.0).into(),
            (0, 0).into(),
            (300, 150).into(),
        );
        assert_eq!(big, (300, 150).into());
        // Without a minimum, a window keeps at least one pixel.
        let tiny = resized(
            initial,
            edges,
            (-500.0, -500.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(tiny, (1, 1).into());
    }

    #[test]
    fn an_edge_not_pulled_does_not_change() {
        let edges = Edges {
            right: true,
            ..Edges::default()
        };
        let size = resized(
            rect(0, 0, 200, 100),
            edges,
            (10.0, 40.0).into(),
            (0, 0).into(),
            (0, 0).into(),
        );
        assert_eq!(size, (210, 100).into());
    }

    fn edges(top: bool, bottom: bool, left: bool, right: bool) -> Option<Edges> {
        Some(Edges {
            top,
            bottom,
            left,
            right,
        })
    }

    #[test]
    fn the_band_around_the_window_pulls_each_edge() {
        let window = rect(100, 100, 200, 100);
        let at = |x: f64, y: f64| border_edges(window, (x, y).into(), 8, 20);
        assert_eq!(at(95.0, 150.0), edges(false, false, true, false), "left");
        assert_eq!(at(305.0, 150.0), edges(false, false, false, true), "right");
        assert_eq!(at(200.0, 93.0), edges(true, false, false, false), "top");
        assert_eq!(at(200.0, 200.0), edges(false, true, false, false), "bottom");
        // Along a side, near a corner: it is the corner.
        assert_eq!(at(95.0, 110.0), edges(true, false, true, false), "top left");
        assert_eq!(
            at(290.0, 95.0),
            edges(true, false, false, true),
            "top right"
        );
        assert_eq!(
            at(95.0, 190.0),
            edges(false, true, true, false),
            "bottom left"
        );
        assert_eq!(
            at(304.0, 199.0),
            edges(false, true, false, true),
            "bottom right"
        );
    }

    #[test]
    fn the_band_is_wider_at_the_corners() {
        let window = rect(100, 100, 200, 100);
        let at = |x: f64, y: f64| border_edges(window, (x, y).into(), 8, 20);
        // 15 pixels outside the window: too far on a side, still caught diagonally.
        assert_eq!(at(85.0, 150.0), None);
        assert_eq!(at(85.0, 85.0), edges(true, false, true, false));
        assert_eq!(at(315.0, 215.0), edges(false, true, false, true));
        assert_eq!(at(79.0, 79.0), None);
    }

    #[test]
    fn the_band_does_not_cover_the_inside() {
        let window = rect(100, 100, 200, 100);
        let at = |x: f64, y: f64| border_edges(window, (x, y).into(), 8, 20);
        assert_eq!(at(100.0, 100.0), None);
        assert_eq!(at(299.5, 199.5), None);
        assert_eq!(at(200.0, 150.0), None);
        assert_eq!(at(400.0, 150.0), None);
    }

    #[test]
    fn a_narrow_window_shares_its_sides_between_the_corners() {
        let window = rect(0, 0, 30, 30);
        let right = border_edges(window, (20.0, -3.0).into(), 8, 20);
        assert_eq!(right, edges(true, false, false, true));
    }

    fn zone(x: f64, y: f64) -> Option<Zone> {
        Zone::under(rect(0, 0, 1440, 900), (x, y).into(), 8, 64)
    }

    #[test]
    fn dropped_against_an_edge_the_window_snaps() {
        assert_eq!(zone(700.0, 0.0), Some(Zone::Maximized));
        assert_eq!(zone(700.0, 7.5), Some(Zone::Maximized));
        assert_eq!(zone(0.0, 450.0), Some(Zone::Left));
        assert_eq!(zone(1439.0, 450.0), Some(Zone::Right));
        // The pointer may leave the screen during the gesture: that counts as the edge.
        assert_eq!(zone(-20.0, 450.0), Some(Zone::Left));
        assert_eq!(zone(700.0, 450.0), None);
        assert_eq!(zone(700.0, 8.0), None);
        assert_eq!(
            zone(700.0, 899.0),
            None,
            "the bottom edge alone snaps nothing"
        );
    }

    #[test]
    fn dropped_in_a_corner_the_window_takes_a_quarter() {
        assert_eq!(zone(0.0, 0.0), Some(Zone::TopLeft));
        assert_eq!(zone(3.0, 60.0), Some(Zone::TopLeft), "along the left edge");
        assert_eq!(zone(60.0, 3.0), Some(Zone::TopLeft), "along the top edge");
        assert_eq!(zone(1439.0, 10.0), Some(Zone::TopRight));
        assert_eq!(zone(2.0, 899.0), Some(Zone::BottomLeft));
        assert_eq!(zone(1400.0, 899.0), Some(Zone::BottomRight));
        assert_eq!(zone(3.0, 70.0), Some(Zone::Left), "outside the corner");
    }

    #[test]
    fn geometry_of_each_zone() {
        let area = rect(10, 20, 1001, 801);
        let cases = [
            (Zone::Maximized, rect(10, 20, 1001, 801)),
            (Zone::Left, rect(10, 20, 500, 801)),
            (Zone::Right, rect(510, 20, 501, 801)),
            (Zone::TopLeft, rect(10, 20, 500, 400)),
            (Zone::TopRight, rect(510, 20, 501, 400)),
            (Zone::BottomLeft, rect(10, 420, 500, 401)),
            (Zone::BottomRight, rect(510, 420, 501, 401)),
        ];
        for (zone, expected) in cases {
            assert_eq!(zone.geometry(area, 0), expected, "{zone:?}");
        }
    }

    #[test]
    fn snapped_windows_keep_their_gap() {
        // 8 from the edges of the work area and between two windows; maximized, none.
        let area = rect(0, 46, 1440, 778);
        let cases = [
            (Zone::Maximized, rect(0, 46, 1440, 778)),
            (Zone::Left, rect(8, 54, 708, 762)),
            (Zone::Right, rect(724, 54, 708, 762)),
            (Zone::TopLeft, rect(8, 54, 708, 377)),
            (Zone::TopRight, rect(724, 54, 708, 377)),
            (Zone::BottomLeft, rect(8, 439, 708, 377)),
            (Zone::BottomRight, rect(724, 439, 708, 377)),
        ];
        for (zone, expected) in cases {
            assert_eq!(zone.geometry(area, 8), expected, "{zone:?}");
        }
        // Side by side, the two halves leave the gap between them, and the area's edges.
        let (left, right) = (Zone::Left.geometry(area, 8), Zone::Right.geometry(area, 8));
        assert_eq!(right.loc.x - (left.loc.x + left.size.w), 8);
        assert_eq!(area.loc.x + area.size.w - (right.loc.x + right.size.w), 8);
        // An area smaller than its gaps gives empty windows, never negative ones.
        assert!(Zone::BottomRight.geometry(rect(0, 0, 10, 10), 8).size.w >= 0);
    }

    #[test]
    fn pulled_out_the_window_gets_its_size_back_under_the_pointer() {
        let maximized = rect(0, 0, 1440, 900);
        // Grabbed at three quarters of the width, 12 pixels from the top.
        let loc = unsnapped(maximized, (400, 300).into(), (1080.0, 12.0).into());
        assert_eq!(loc, (780, 0).into());
        // The pointer stays at three quarters of the window restored to its size.
        assert_eq!(f64::from(1080 - loc.x) / 400.0, 0.75);
        // Grabbed lower than the new height: the pointer stays in the window.
        let half = rect(720, 0, 720, 900);
        let loc = unsnapped(half, (400, 300).into(), (900.0, 800.0).into());
        assert_eq!(loc, (800, 500).into());
    }

    #[test]
    fn the_opposite_corner_stays_in_place_during_the_gesture() {
        let initial = rect(100, 100, 200, 100);
        let top_left = Edges::from_xdg_bits(5);
        let during = Resize {
            edges: top_left,
            initial,
            done: false,
        };
        // During the gesture, each commit realigns the window and the state stays.
        let (loc, next) = during.on_commit((260, 140).into());
        assert_eq!(loc, (40, 60).into());
        assert_eq!(next, Some(during));
        // After the release, the last commit realigns once more, then the state goes away.
        let (loc, next) = Resize {
            done: true,
            ..during
        }
        .on_commit((250, 130).into());
        assert_eq!(loc, (50, 70).into());
        assert_eq!(next, None);
    }
}
