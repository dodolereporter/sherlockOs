//! The arrangement of the screens, as the « Écrans » page draws it and lets it be dragged: the
//! screens' rectangles of the space shrunk into the card, and where a screen dropped lands.
//! As on macOS, a screen always touches another by an edge: dropped anywhere, it goes to the
//! nearest place along another screen where it covers none.

/// A rectangle of the space, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        // Half a pixel of tolerance: rounded sizes of two screens side by side touch.
        let eps = 0.5;
        self.x + eps < o.x + o.w
            && o.x + eps < self.x + self.w
            && self.y + eps < o.y + o.h
            && o.y + eps < self.y + self.h
    }
}

/// How the space is drawn in an area of `width` × `height`: the scale and the offset of the
/// space's origin, so that every screen fits with `margin` around, centered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub scale: f64,
    pub origin: (f64, f64),
}

impl Fit {
    pub fn new(rects: &[Rect], width: f64, height: f64, margin: f64) -> Self {
        let Some(first) = rects.first() else {
            return Self {
                scale: 1.0,
                origin: (0.0, 0.0),
            };
        };
        let (mut x0, mut y0) = (first.x, first.y);
        let (mut x1, mut y1) = (first.x + first.w, first.y + first.h);
        for r in rects {
            x0 = x0.min(r.x);
            y0 = y0.min(r.y);
            x1 = x1.max(r.x + r.w);
            y1 = y1.max(r.y + r.h);
        }
        let (w, h) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
        let room = (
            (width - 2.0 * margin).max(1.0),
            (height - 2.0 * margin).max(1.0),
        );
        let scale = (room.0 / w).min(room.1 / h);
        let origin = (
            (width - w * scale) / 2.0 - x0 * scale,
            (height - h * scale) / 2.0 - y0 * scale,
        );
        Self { scale, origin }
    }

    /// The area's rectangle of a rectangle of the space.
    pub fn to_area(&self, r: Rect) -> Rect {
        Rect {
            x: self.origin.0 + r.x * self.scale,
            y: self.origin.1 + r.y * self.scale,
            w: r.w * self.scale,
            h: r.h * self.scale,
        }
    }

    /// The point of the space under a point of the area.
    pub fn to_space(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (
            (x - self.origin.0) / self.scale,
            (y - self.origin.1) / self.scale,
        )
    }
}

/// Where `moving` lands when dropped where it is, among `others`: along an edge of one of
/// them, covering none, the nearest place to where it was dropped. Without others, the origin.
pub fn snap(moving: Rect, others: &[Rect]) -> (i32, i32) {
    let mut best: Option<((f64, f64), f64)> = None;
    for o in others {
        // Along a side, the screens share at least a tenth of the shorter one.
        let share_y = moving.h.min(o.h) / 10.0;
        let share_x = moving.w.min(o.w) / 10.0;
        let y = moving
            .y
            .clamp(o.y - moving.h + share_y, o.y + o.h - share_y);
        let x = moving
            .x
            .clamp(o.x - moving.w + share_x, o.x + o.w - share_x);
        let candidates = [
            (o.x + o.w, y),
            (o.x - moving.w, y),
            (x, o.y + o.h),
            (x, o.y - moving.h),
        ];
        for (cx, cy) in candidates {
            let placed = Rect {
                x: cx,
                y: cy,
                ..moving
            };
            if others.iter().any(|r| placed.overlaps(r)) {
                continue;
            }
            let distance = (cx - moving.x).powi(2) + (cy - moving.y).powi(2);
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some(((cx, cy), distance));
            }
        }
    }
    let (x, y) = best.map_or((0.0, 0.0), |(p, _)| p);
    (x.round() as i32, y.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn the_screens_fit_centered() {
        let rects = [
            rect(0.0, 0.0, 1920.0, 1080.0),
            rect(1920.0, 0.0, 1280.0, 720.0),
        ];
        let fit = Fit::new(&rects, 400.0, 200.0, 20.0);
        let all = fit.to_area(rect(0.0, 0.0, 3200.0, 1080.0));
        assert!((all.w - 360.0).abs() < 1e-6, "{all:?}");
        assert!((all.x - 20.0).abs() < 1e-6);
        assert!((all.y + all.h / 2.0 - 100.0).abs() < 1e-6);
        let back = fit.to_space((all.x, all.y));
        assert!(back.0.abs() < 1e-6 && back.1.abs() < 1e-6);
    }

    #[test]
    fn a_dropped_screen_goes_to_the_nearest_edge() {
        let main = rect(0.0, 0.0, 1920.0, 1080.0);
        // Dropped a little over the right edge, a little low: right of it, at that height.
        let moved = snap(rect(1800.0, 300.0, 1280.0, 720.0), &[main]);
        assert_eq!(moved, (1920, 300));
        // Dropped above: on top of it.
        let moved = snap(rect(100.0, -500.0, 1280.0, 720.0), &[main]);
        assert_eq!(moved, (100, -720));
        // Far below and right: it keeps touching, by a tenth at least.
        let moved = snap(rect(5000.0, 5000.0, 1280.0, 720.0), &[main]);
        assert_eq!(moved, (1920, 1008));
    }

    #[test]
    fn a_dropped_screen_never_covers_another() {
        let a = rect(0.0, 0.0, 1920.0, 1080.0);
        let b = rect(1920.0, 0.0, 1920.0, 1080.0);
        let (x, y) = snap(rect(1900.0, 100.0, 1000.0, 600.0), &[a, b]);
        let placed = rect(f64::from(x), f64::from(y), 1000.0, 600.0);
        assert!(!placed.overlaps(&a) && !placed.overlaps(&b), "{placed:?}");
        assert_eq!(snap(rect(10.0, 10.0, 100.0, 100.0), &[]), (0, 0));
    }
}
