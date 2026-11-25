//! Motion of the windows and of the snap preview: the springs and fades of
//! `sherlock_ui::motion` applied to rectangles. Pure computation, without rendering or Wayland.

use std::time::Duration;

use sherlock_ui::motion::{self, Animation, Fade, Spring};
use smithay::utils::{Logical, Rectangle};

pub type RectF = Rectangle<f64, Logical>;

/// What to draw of a window or of the preview at a given time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visual {
    pub rect: RectF,
    pub opacity: f32,
    /// The geometry is still sliding towards its real place.
    pub moving: bool,
}

/// `rect` scaled by `scale` around its center.
pub fn scaled(rect: RectF, scale: f64) -> RectF {
    let size = rect.size.upscale(scale);
    let loc = rect.loc + (rect.size.to_point() - size.to_point()).downscale(2.0);
    Rectangle::new(loc, size)
}

fn parts(rect: RectF) -> [f64; 4] {
    [rect.loc.x, rect.loc.y, rect.size.w, rect.size.h]
}

/// Rectangle driven by four springs: position and size.
#[derive(Debug, Clone, Copy)]
pub struct AnimatedRect([Animation; 4]);

impl AnimatedRect {
    pub fn new(spring: Spring, from: RectF, to: RectF, now: Duration) -> Self {
        let (from, to) = (parts(from), parts(to));
        Self(std::array::from_fn(|i| {
            Animation::new(spring, from[i], to[i], now)
        }))
    }

    /// Rectangle at time `now`. A settled side is at its exact target, with no floating-point
    /// leftover.
    pub fn value(&self, now: Duration) -> RectF {
        let [x, y, w, h] = self.0.map(|a| {
            if a.is_done(now, motion::GEOMETRY_PRECISION) {
                a.target()
            } else {
                a.value(now)
            }
        });
        Rectangle::new((x, y).into(), (w, h).into())
    }

    /// Changes target without a jolt.
    pub fn retarget(&mut self, now: Duration, to: RectF) {
        for (animation, to) in self.0.iter_mut().zip(parts(to)) {
            if animation.target() != to {
                animation.retarget(now, to);
            }
        }
    }

    pub fn is_done(&self, now: Duration) -> bool {
        self.0
            .iter()
            .all(|a| a.is_done(now, motion::GEOMETRY_PRECISION))
    }
}

/// Motion of a window: opening (scale and opacity), then geometry changes announced by
/// `animate_next_change`. Other changes, like those of a drag or a resize, are followed without
/// delay.
#[derive(Debug, Default)]
pub struct WindowMotion {
    /// Opening: scale and opacity. `None` once finished.
    open: Option<(Animation, Fade)>,
    opened: bool,
    /// Animated geometry. `None` at rest: the window is drawn at its real place.
    geometry: Option<AnimatedRect>,
    /// Last geometry drawn, before the opening scale.
    last: Option<RectF>,
    animate_next: bool,
}

impl WindowMotion {
    /// The next geometry change will be animated: snap, maximize, restore. The flag only holds
    /// for the next frame: the geometry has already changed there, or will not change.
    pub fn animate_next_change(&mut self) {
        self.animate_next = true;
    }

    /// Stops the geometry animation: a gesture starts, the window follows the pointer.
    pub fn follow(&mut self) {
        self.geometry = None;
        self.animate_next = false;
    }

    /// What to draw at time `now` for a window whose real geometry is `target`. The first call
    /// starts the opening.
    pub fn frame(&mut self, now: Duration, target: Rectangle<i32, Logical>) -> Visual {
        let target = target.to_f64();
        if !self.opened {
            self.opened = true;
            self.open = Some((
                Animation::new(motion::OPEN_SPRING, motion::OPEN_SCALE, 1.0, now),
                Fade::new(0.0, 1.0, now, motion::FADE),
            ));
        }
        // The flag is consumed on every frame, whether an animation starts or not: otherwise the
        // next drag or resize would be animated.
        let animate = std::mem::take(&mut self.animate_next);
        match (&mut self.geometry, self.last) {
            (Some(geometry), _) => geometry.retarget(now, target),
            (None, Some(last)) if animate && last != target => {
                self.geometry = Some(AnimatedRect::new(motion::SNAP_SPRING, last, target, now));
            }
            // Never drawn: nothing to start from, it will appear at its place.
            _ => {}
        }
        let rect = match self.geometry {
            Some(geometry) if !geometry.is_done(now) => geometry.value(now),
            _ => {
                self.geometry = None;
                target
            }
        };
        self.last = Some(rect);

        let (scale, opacity) = match self.open {
            Some((scale, fade))
                if !(scale.is_done(now, motion::SCALE_PRECISION) && fade.is_done(now)) =>
            {
                (scale.value(now), fade.value(now))
            }
            _ => {
                self.open = None;
                (1.0, 1.0)
            }
        };
        Visual {
            rect: scaled(rect, scale),
            opacity,
            moving: self.geometry.is_some(),
        }
    }

    /// An animation is running: the drawn geometry is not the real geometry.
    pub fn is_animating(&self) -> bool {
        self.open.is_some() || self.geometry.is_some()
    }
}

/// Motion of the snap preview: it grows as it appears, slides from one zone to another and
/// fades out at its last place.
#[derive(Debug, Default)]
pub struct PreviewMotion {
    shown: Option<(AnimatedRect, Fade)>,
}

impl PreviewMotion {
    /// What to draw at time `now`, `zone` being the targeted zone if there is one.
    pub fn frame(
        &mut self,
        now: Duration,
        zone: Option<Rectangle<i32, Logical>>,
    ) -> Option<Visual> {
        match (&mut self.shown, zone) {
            (Some((rect, fade)), Some(zone)) => {
                rect.retarget(now, zone.to_f64());
                if fade.target() < 1.0 {
                    fade.retarget(now, 1.0, motion::FADE);
                }
            }
            (None, Some(zone)) => {
                let zone = zone.to_f64();
                let from = scaled(zone, motion::OPEN_SCALE);
                self.shown = Some((
                    AnimatedRect::new(motion::PREVIEW_SPRING, from, zone, now),
                    Fade::new(0.0, 1.0, now, motion::FADE),
                ));
            }
            (Some((_, fade)), None) if fade.target() > 0.0 => {
                fade.retarget(now, 0.0, motion::FADE);
            }
            _ => {}
        }
        let (rect, fade) = self.shown?;
        if fade.target() <= 0.0 && fade.is_done(now) {
            self.shown = None;
            return None;
        }
        Some(Visual {
            rect: rect.value(now),
            opacity: fade.value(now),
            moving: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn a_window_opens_by_growing_and_appearing() {
        let mut motion = WindowMotion::default();
        let place = rect(100, 100, 400, 300);
        let first = motion.frame(ms(0), place);
        assert_eq!(first.opacity, 0.0);
        let expected = scaled(place.to_f64(), motion::OPEN_SCALE);
        assert!((first.rect.size.w - expected.size.w).abs() < 1e-9);
        assert!((first.rect.loc.y - expected.loc.y).abs() < 1e-9);
        // Centered on its place during the whole opening.
        let midway = motion.frame(ms(60), place);
        assert!(midway.opacity > 0.0 && midway.opacity < 1.0);
        let center = |r: RectF| r.loc + r.size.to_point().downscale(2.0);
        assert!((center(midway.rect) - center(place.to_f64())).x.abs() < 1e-9);
        assert!(motion.is_animating());
        let end = motion.frame(ms(1000), place);
        assert_eq!(end.rect, place.to_f64());
        assert_eq!(end.opacity, 1.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn a_snap_is_animated_and_a_drag_follows_without_delay() {
        let mut motion = WindowMotion::default();
        let (floating, left) = (rect(300, 200, 400, 300), rect(0, 0, 720, 900));
        motion.frame(ms(0), floating);
        motion.frame(ms(1000), floating);
        // Drag: the window follows the pointer.
        let dragged = rect(310, 205, 400, 300);
        assert_eq!(motion.frame(ms(1010), dragged).rect, dragged.to_f64());
        // Snap: it leaves its place and reaches the zone, first the position, then the size
        // when the client answers.
        motion.animate_next_change();
        let start = motion.frame(ms(1020), rect(0, 0, 400, 300)).rect;
        assert_eq!(start, dragged.to_f64());
        motion.frame(ms(1060), left);
        let moving = motion.frame(ms(1100), left).rect;
        assert!(moving.loc.x > 0.0 && moving.loc.x < 310.0);
        assert!(moving.size.w > 400.0 && moving.size.w < 720.0);
        assert_eq!(motion.frame(ms(2000), left).rect, left.to_f64());
        assert!(!motion.is_animating());
        // Once the snap is over, the next drag is no longer animated.
        assert_eq!(motion.frame(ms(2010), floating).rect, floating.to_f64());
    }

    #[test]
    fn the_flag_only_holds_for_the_next_frame() {
        let (left, right) = (rect(0, 0, 400, 600), rect(400, 0, 400, 600));
        // Snap without change: the window is already on the left.
        let mut motion = WindowMotion::default();
        motion.frame(ms(0), left);
        motion.frame(ms(1000), left);
        motion.animate_next_change();
        motion.frame(ms(1010), left);
        assert_eq!(motion.frame(ms(1020), right).rect, right.to_f64());
        // Snap during a snap: the target changes, the flag drops too.
        let mut motion = WindowMotion::default();
        motion.frame(ms(0), left);
        motion.frame(ms(1000), left);
        motion.animate_next_change();
        motion.frame(ms(1010), right);
        motion.animate_next_change();
        motion.frame(ms(1050), left);
        motion.frame(ms(2000), left);
        assert_eq!(motion.frame(ms(2010), right).rect, right.to_f64());
        // A gesture stops the running animation.
        motion.animate_next_change();
        motion.frame(ms(2020), left);
        assert!(motion.frame(ms(2030), left).moving);
        motion.follow();
        assert_eq!(motion.frame(ms(2040), right).rect, right.to_f64());
    }

    #[test]
    fn the_preview_appears_slides_and_fades_out() {
        let mut preview = PreviewMotion::default();
        let (left, top) = (rect(0, 0, 720, 900), rect(0, 0, 1440, 900));
        assert_eq!(preview.frame(ms(0), None), None);
        let first = preview.frame(ms(0), Some(left)).unwrap();
        assert_eq!(first.opacity, 0.0);
        assert!(first.rect.size.w < 720.0);
        assert_eq!(
            preview.frame(ms(500), Some(left)).unwrap().rect,
            left.to_f64()
        );
        let sliding = preview.frame(ms(550), Some(top)).unwrap();
        assert!(sliding.rect.size.w > 720.0 && sliding.rect.size.w < 1440.0);
        assert_eq!(sliding.opacity, 1.0);
        let fading = preview.frame(ms(600), None).unwrap();
        assert_eq!(fading.opacity, 1.0);
        assert!(preview.frame(ms(650), None).unwrap().opacity < 1.0);
        assert_eq!(preview.frame(ms(600) + motion::FADE, None), None);
    }
}
