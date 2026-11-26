//! The two forms of the bar, and the spring between them.
//!
//! Floating, the pill of the Main board: margins of 10 on top and 12 on the sides, radius 12,
//! glass. Attached, while a maximized or fullscreen window is on its screen: a band along the
//! top edge, the whole width, square corners, the bar's glass made opaque.
//!
//! The surface keeps an exclusive zone of its height in both forms. The compositor reserves
//! that zone plus the top margin (`docs/compositor.md`): 46 floating, 36 attached. Only the
//! margins move during the transition, each change of a whole pixel being one commit, so the
//! room left to a maximized window follows the bar. Changing the zone as well would take a
//! second commit per frame, and the compositor would see a reserved height that is neither
//! the old one nor the new one between the two.

use std::time::{Duration, Instant};

use sherlock_ui::metrics;
use sherlock_ui::motion::{self, Animation};

/// Settled within half a pixel of the widest margin.
const PRECISION: f64 = motion::GEOMETRY_PRECISION / metrics::TOP_BAR_MARGIN_SIDE as f64;

/// The bar at one instant of its shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub margin_top: i32,
    pub margin_side: i32,
    pub radius: f32,
    /// 0 for the glass, 1 for the opaque band.
    pub solid: f32,
}

impl Geometry {
    /// At `progress` from floating (0) to attached (1).
    pub fn at(progress: f32) -> Self {
        let floating = 1.0 - progress.clamp(0.0, 1.0);
        Self {
            margin_top: (metrics::TOP_BAR_MARGIN_TOP * floating).round() as i32,
            margin_side: (metrics::TOP_BAR_MARGIN_SIDE * floating).round() as i32,
            radius: metrics::TOP_BAR_RADIUS * floating,
            solid: 1.0 - floating,
        }
    }

    /// What the compositor keeps clear below the top edge of the screen.
    pub fn reserved(&self) -> i32 {
        exclusive_zone() + self.margin_top
    }
}

impl Default for Geometry {
    /// Floating.
    fn default() -> Self {
        Self::at(0.0)
    }
}

/// Exclusive zone of the surface, in both forms: its height.
pub fn exclusive_zone() -> i32 {
    metrics::TOP_BAR_HEIGHT as i32
}

/// The shape of the bar, from floating (0) to attached (1).
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    animation: Animation,
    origin: Instant,
    /// Last frame, from `origin`.
    now: Duration,
}

impl Default for Shape {
    /// Floating, at rest.
    fn default() -> Self {
        Self {
            animation: Animation::new(motion::TOP_BAR_SPRING, 0.0, 0.0, Duration::ZERO),
            origin: Instant::now(),
            now: Duration::ZERO,
        }
    }
}

impl Shape {
    /// Whether the bar is attached, or on its way there.
    pub fn attached(&self) -> bool {
        self.animation.target() > 0.5
    }

    /// Heads for the attached form or the floating one, from where the bar is at `now`.
    /// `true` when the aim changed.
    pub fn set(&mut self, attached: bool, now: Instant) -> bool {
        if attached == self.attached() {
            return false;
        }
        self.now = now.saturating_duration_since(self.origin);
        let target = if attached { 1.0 } else { 0.0 };
        self.animation.retarget(self.now, target);
        true
    }

    /// A new frame.
    pub fn tick(&mut self, now: Instant) {
        self.now = now.saturating_duration_since(self.origin);
    }

    /// The spring still moves: frames are needed.
    pub fn running(&self) -> bool {
        !self.animation.is_done(self.now, PRECISION)
    }

    pub fn geometry(&self) -> Geometry {
        let progress = if self.running() {
            self.animation.value(self.now)
        } else {
            self.animation.target()
        };
        Geometry::at(progress as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_then_attached_geometry() {
        let floating = Geometry::at(0.0);
        assert_eq!((floating.margin_top, floating.margin_side), (10, 12));
        assert_eq!((floating.radius, floating.solid), (12.0, 0.0));
        assert_eq!((exclusive_zone(), floating.reserved()), (36, 46));
        let attached = Geometry::at(1.0);
        assert_eq!((attached.margin_top, attached.margin_side), (0, 0));
        assert_eq!((attached.radius, attached.solid), (0.0, 1.0));
        assert_eq!(attached.reserved(), 36);
        // The spring's overshoot never pushes the bar off the screen nor below the pill.
        assert_eq!(Geometry::at(1.03), attached);
        assert_eq!(Geometry::at(-0.03), floating);
    }

    #[test]
    fn the_bar_springs_to_the_top_edge_and_back() {
        let mut shape = Shape::default();
        let start = shape.origin;
        assert!(!shape.running() && !shape.attached());
        assert!(shape.set(true, start));
        assert!(!shape.set(true, start), "already heading there");
        let mut last = shape.geometry();
        assert_eq!(last, Geometry::at(0.0), "nothing jumps");
        for frame in 1..=10 {
            shape.tick(start + Duration::from_millis(16 * frame));
            let now = shape.geometry();
            assert!(now.margin_top <= last.margin_top && now.radius <= last.radius);
            assert!((36..=46).contains(&now.reserved()));
            last = now;
        }
        assert!(
            last.margin_top < 10 && shape.running(),
            "on its way: {last:?}"
        );
        shape.tick(start + Duration::from_secs(1));
        assert!(!shape.running());
        assert_eq!(shape.geometry(), Geometry::at(1.0));

        // Back to floating, interrupted halfway: it turns back from where it is.
        let back = start + Duration::from_secs(2);
        shape.set(false, back);
        shape.tick(back + Duration::from_millis(40));
        let halfway = shape.geometry();
        assert!(shape.set(true, back + Duration::from_millis(40)));
        assert_eq!(shape.geometry(), halfway);
        shape.tick(back + Duration::from_secs(1));
        assert_eq!(shape.geometry(), Geometry::at(1.0));
    }
}
