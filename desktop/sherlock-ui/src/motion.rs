//! Springs and fades: the physics of the desktop animations, as pure computation.
//!
//! The compositor and the shell share these functions and tokens, without iced. Time is a
//! `Duration` counted from an origin the caller chooses: an animation is computed at any
//! instant, with no state that advances frame by frame.
//!
//! `docs/contexte.md` asks for springs with a slight overshoot and hovers of 110 to 150 ms.
//! The stiffnesses and dampings below are choices of lot D-11: an overshoot of 2 to 3 %,
//! settled in less than 300 ms. See "Motion" in `docs/sherlock-ui.md`, and "Animations" in
//! `docs/compositor.md` for their use.

use std::time::Duration;

/// Damped spring, of mass 1: `x'' = -stiffness × (x - target) - damping × x'`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub stiffness: f64,
    pub damping: f64,
}

impl Spring {
    pub const fn new(stiffness: f64, damping: f64) -> Self {
        Self { stiffness, damping }
    }

    /// Damping ratio: below 1, the spring overshoots its target before settling on it.
    pub fn damping_ratio(self) -> f64 {
        self.damping / (2.0 * self.stiffness.sqrt())
    }

    /// Offset from the target and velocity after `t` seconds, starting from the offset
    /// `offset` with the velocity `velocity`. Exact solution of the equation, with no time step.
    pub fn solve(self, offset: f64, velocity: f64, t: f64) -> (f64, f64) {
        let omega = self.stiffness.sqrt();
        let zeta = self.damping_ratio();
        if zeta < 1.0 {
            let decay = zeta * omega;
            let omega_d = omega * (1.0 - zeta * zeta).sqrt();
            let b = (velocity + decay * offset) / omega_d;
            let (sin, cos) = (omega_d * t).sin_cos();
            let e = (-decay * t).exp();
            let position = e * (offset * cos + b * sin);
            let speed =
                e * (velocity * cos - (decay * velocity + omega * omega * offset) / omega_d * sin);
            (position, speed)
        } else if (zeta - 1.0).abs() < 1e-9 {
            let b = velocity + omega * offset;
            let e = (-omega * t).exp();
            (e * (offset + b * t), e * (velocity - omega * b * t))
        } else {
            let root = omega * (zeta * zeta - 1.0).sqrt();
            let (r1, r2) = (-zeta * omega + root, -zeta * omega - root);
            let c2 = (velocity - r1 * offset) / (r2 - r1);
            let c1 = offset - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
        }
    }
}

/// A value driven by a spring towards its target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Animation {
    spring: Spring,
    from: f64,
    to: f64,
    velocity: f64,
    start: Duration,
}

impl Animation {
    /// Starts from `from` at rest, at the instant `start`, towards `to`.
    pub fn new(spring: Spring, from: f64, to: f64, start: Duration) -> Self {
        Self {
            spring,
            from,
            to,
            velocity: 0.0,
            start,
        }
    }

    /// Starts from `from` moving at `velocity` (units per second), at the instant `start`,
    /// towards `to`: what a gesture let go of hands over (lot D-31).
    pub fn with_velocity(
        spring: Spring,
        from: f64,
        to: f64,
        velocity: f64,
        start: Duration,
    ) -> Self {
        Self {
            velocity,
            ..Self::new(spring, from, to, start)
        }
    }

    fn solve(&self, now: Duration) -> (f64, f64) {
        let t = now.saturating_sub(self.start).as_secs_f64();
        let (offset, speed) = self.spring.solve(self.from - self.to, self.velocity, t);
        (self.to + offset, speed)
    }

    /// Value at the instant `now`.
    pub fn value(&self, now: Duration) -> f64 {
        self.solve(now).0
    }

    /// Target.
    pub fn target(&self) -> f64 {
        self.to
    }

    /// Changes target without a jolt: the value and the velocity at `now` are kept.
    pub fn retarget(&mut self, now: Duration, to: f64) {
        let (value, velocity) = self.solve(now);
        *self = Self {
            from: value,
            to,
            velocity,
            start: now,
            ..*self
        };
    }

    /// The value is within `precision` of its target and barely moves any more: it would
    /// travel less than `precision` in a tenth of a second.
    pub fn is_done(&self, now: Duration, precision: f64) -> bool {
        let (value, speed) = self.solve(now);
        (value - self.to).abs() < precision && speed.abs() * 0.1 < precision
    }
}

/// Fade of a value, most often an opacity, over a fixed duration, with a brisk start and a
/// gentle arrival (cubic).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    from: f32,
    to: f32,
    start: Duration,
    duration: Duration,
}

impl Fade {
    pub fn new(from: f32, to: f32, start: Duration, duration: Duration) -> Self {
        Self {
            from,
            to,
            start,
            duration,
        }
    }

    /// Progress from 0 to 1.
    fn progress(&self, now: Duration) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let t = now.saturating_sub(self.start).as_secs_f32() / self.duration.as_secs_f32();
        1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
    }

    /// Value at the instant `now`.
    pub fn value(&self, now: Duration) -> f32 {
        self.from + (self.to - self.from) * self.progress(now)
    }

    /// Target.
    pub fn target(&self) -> f32 {
        self.to
    }

    pub fn is_done(&self, now: Duration) -> bool {
        now >= self.start + self.duration
    }

    /// Restarts from the value at `now` towards `to`. The duration is that of the full run,
    /// reduced to the share left to travel: a fade interrupted halfway comes back in half the
    /// time, and nothing jumps.
    pub fn retarget(&mut self, now: Duration, to: f32, full: Duration) {
        let from = self.value(now);
        let span = (self.to - self.from).abs().max(f32::EPSILON);
        let share = ((to - from).abs() / span).clamp(0.0, 1.0);
        *self = Self::new(from, to, now, full.mul_f32(share));
    }
}

// Tokens. The damping ratio of each spring is noted: below 1, it overshoots its target.

/// Snap, maximize and restore of a window (ratio 0.75, overshoot of 3 %).
pub const SNAP_SPRING: Spring = Spring::new(500.0, 33.5);
/// Opening of a window: its scale (ratio 0.80, overshoot of 2 %).
pub const OPEN_SPRING: Spring = Spring::new(450.0, 34.0);
/// Top bar attaching to the top edge and floating again (D-18b): it answers a window being
/// maximized or restored, so it moves with the same spring.
pub const TOP_BAR_SPRING: Spring = SNAP_SPRING;
/// Snap preview, brisker than the window it announces (ratio 0.79).
pub const PREVIEW_SPRING: Spring = Spring::new(700.0, 42.0);
/// Corners, border and shadow of a window that becomes maximized or fullscreen, or leaves it
/// (ratio 1.0: no overshoot, so the radius never goes below 0 nor the shadow above its own).
pub const DECORATION_SPRING: Spring = Spring::new(700.0, 53.0);

/// Scale of a window at the start of its opening.
pub const OPEN_SCALE: f64 = 0.92;
/// Fade of a window opening, and of the snap preview appearing: the duration of a hover, in
/// the 110 to 150 ms range.
pub const FADE: Duration = Duration::from_millis(crate::metrics::HOVER_MS.1);

/// Minimize and restore of a window (lot D-19): the genie effect into its dock icon, or the
/// scale fallback. The duration the brief of the lot gives, about half a second.
pub const MINIMIZE: Duration = Duration::from_millis(450);
/// Scale of a window at the end of the minimize fallback, at the bottom of its screen.
pub const MINIMIZE_SCALE: f64 = 0.2;

/// Fade of the whole desktop from one theme to the other (`color::ThemeFade`). A choice of
/// D-05b: the 0.25 s of the cross-fades between two animations of Sherlock
/// (`docs/contexte.md`), longer than a hover, since every surface changes at once.
pub const THEME_FADE: Duration = Duration::from_millis(250);

/// Frame rate the desktop must hold, animations included (`docs/contexte.md`).
pub const FRAME_RATE: u32 = 60;

/// Precision below which an animated geometry is settled, in pixels.
pub const GEOMETRY_PRECISION: f64 = 0.5;
/// Precision below which an animated scale is settled.
pub const SCALE_PRECISION: f64 = 0.001;

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Spring; 3] = [SNAP_SPRING, OPEN_SPRING, PREVIEW_SPRING];

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Values of a run from 0 to 1, every millisecond for `total` ms.
    fn trajectory(spring: Spring, total: u64) -> Vec<f64> {
        let animation = Animation::new(spring, 0.0, 1.0, Duration::ZERO);
        (0..=total).map(|t| animation.value(ms(t))).collect()
    }

    #[test]
    fn every_spring_converges_to_its_target() {
        for spring in ALL {
            let animation = Animation::new(spring, 10.0, 250.0, ms(40));
            assert_eq!(animation.value(ms(40)), 10.0);
            assert!(
                (animation.value(ms(2040)) - 250.0).abs() < 1e-6,
                "{spring:?}"
            );
            assert!(animation.is_done(ms(2040), GEOMETRY_PRECISION));
            assert!(!animation.is_done(ms(60), GEOMETRY_PRECISION));
        }
    }

    #[test]
    fn the_overshoot_is_slight_and_bounded() {
        for spring in ALL {
            let peak = trajectory(spring, 2000).into_iter().fold(0.0, f64::max);
            // Slight: visible, but under 4 %.
            assert!(peak > 1.005 && peak < 1.04, "{spring:?} peaks at {peak}");
            assert!(spring.damping_ratio() > 0.7 && spring.damping_ratio() < 0.85);
        }
    }

    #[test]
    fn springs_respond_quickly() {
        for spring in ALL {
            let values = trajectory(spring, 1000);
            // 90 % of the run in less than 150 ms, settled within 1 % in less than 300 ms.
            let first_90 = values.iter().position(|v| *v >= 0.9).unwrap();
            assert!(first_90 < 150, "{spring:?}: 90 % at {first_90} ms");
            let settled = values.iter().rposition(|v| (v - 1.0).abs() > 0.01).unwrap() + 1;
            assert!(settled < 300, "{spring:?}: settled at {settled} ms");
        }
    }

    #[test]
    fn a_retarget_keeps_the_value_and_the_velocity() {
        let mut animation = Animation::new(SNAP_SPRING, 0.0, 100.0, Duration::ZERO);
        let before = (animation.value(ms(50)), animation.value(ms(51)));
        animation.retarget(ms(50), -40.0);
        assert!((animation.value(ms(50)) - before.0).abs() < 1e-9);
        // The velocity is continuous: the first millisecond still goes the same way.
        assert!(animation.value(ms(51)) - before.0 > 0.0);
        assert!((animation.value(ms(51)) - before.1).abs() < 0.1);
        assert!((animation.value(ms(3000)) + 40.0).abs() < 1e-6);
    }

    #[test]
    fn a_thrown_value_leaves_with_its_velocity() {
        // Thrown away from its target at 2 units per second: it goes on that way first.
        let animation = Animation::with_velocity(SNAP_SPRING, 0.5, 0.0, 2.0, ms(100));
        assert_eq!(animation.value(ms(100)), 0.5);
        let first = animation.value(ms(101)) - 0.5;
        // 2 × 0.001, a little less from the spring pulling back.
        assert!(first < 0.002 && first > 0.0017, "{first}");
        assert!(animation.value(ms(110)) > 0.5);
        assert!(animation.is_done(ms(1000), 1e-3));
        assert!((animation.value(ms(1000))).abs() < 1e-3);
    }

    #[test]
    fn the_decoration_spring_is_quick_without_overshoot() {
        let values = trajectory(DECORATION_SPRING, 1000);
        assert!(values.iter().all(|v| *v <= 1.0 + 1e-9));
        let first_90 = values.iter().position(|v| *v >= 0.9).unwrap();
        assert!(first_90 < 150, "90 % at {first_90} ms");
        let settled = values.iter().rposition(|v| (v - 1.0).abs() > 0.01).unwrap() + 1;
        assert!(settled < 300, "settled at {settled} ms");
    }

    #[test]
    fn overdamped_or_critical_springs_do_not_overshoot() {
        for spring in [Spring::new(400.0, 40.0), Spring::new(400.0, 80.0)] {
            let values = trajectory(spring, 3000);
            assert!(values.iter().all(|v| *v <= 1.0 + 1e-9), "{spring:?}");
            assert!((values[3000] - 1.0).abs() < 1e-3, "{spring:?}");
        }
    }

    #[test]
    fn the_fade_lasts_a_hover_and_does_not_jump() {
        let (low, high) = crate::metrics::HOVER_MS;
        assert!(FADE >= ms(low) && FADE <= ms(high));
        let mut fade = Fade::new(0.0, 1.0, ms(10), FADE);
        assert_eq!(fade.value(ms(10)), 0.0);
        assert!(fade.value(ms(80)) > 0.5 && fade.value(ms(80)) < 1.0);
        assert_eq!(fade.value(ms(10) + FADE), 1.0);
        assert!(fade.is_done(ms(10) + FADE));
        // Interrupted, it restarts from its value and comes back in less time.
        let midway = fade.value(ms(40));
        fade.retarget(ms(40), 0.0, FADE);
        assert_eq!(fade.value(ms(40)), midway);
        assert!(fade.is_done(ms(40) + FADE.mul_f32(midway)));
        assert_eq!(fade.value(ms(1000)), 0.0);
    }
}
