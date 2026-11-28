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

/// Curve of a timed animation: the share of the way done at each share of the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ease {
    /// Brisk start, gentle arrival: `1 - (1 - t)³`.
    OutCubic,
    /// Brisker start, longer arrival: `1 - (1 - t)⁵`.
    OutQuint,
    /// Gentle start, brisk end: `t²`, close to CSS `ease-in`.
    InQuad,
    /// Gentler start, brisker end: `t³`.
    InCubic,
}

impl Ease {
    /// Share of the way at the share of the time `t`, both from 0 to 1. Never past 1: no
    /// bounce.
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::OutCubic => 1.0 - (1.0 - t).powi(3),
            Self::OutQuint => 1.0 - (1.0 - t).powi(5),
            Self::InQuad => t * t,
            Self::InCubic => t * t * t,
        }
    }
}

/// Fade of a value, most often an opacity, over a fixed duration, with a brisk start and a
/// gentle arrival (`Ease::OutCubic`), or on the curve given to `eased`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fade {
    from: f32,
    to: f32,
    start: Duration,
    duration: Duration,
    ease: Ease,
}

impl Fade {
    pub fn new(from: f32, to: f32, start: Duration, duration: Duration) -> Self {
        Self::eased(from, to, start, duration, Ease::OutCubic)
    }

    /// The same, on the curve `ease`.
    pub fn eased(from: f32, to: f32, start: Duration, duration: Duration, ease: Ease) -> Self {
        Self {
            from,
            to,
            start,
            duration,
            ease,
        }
    }

    /// Progress from 0 to 1.
    fn progress(&self, now: Duration) -> f32 {
        if self.duration.is_zero() {
            return 1.0;
        }
        let t = now.saturating_sub(self.start).as_secs_f32() / self.duration.as_secs_f32();
        self.ease.apply(t)
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
        *self = Self::eased(from, to, now, full.mul_f32(share), self.ease);
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

/// Knob of a toggle sliding from one end to the other (lot UI-01): brisker than a window, with
/// the slight overshoot of the desktop's springs (ratio 0.78, 2 %), settled in about 220 ms.
pub const TOGGLE_SPRING: Spring = Spring::new(600.0, 38.0);
/// Share of the knob's run below which it is settled: under a fifth of a pixel on its 18.
pub const KNOB_PRECISION: f32 = 0.01;
/// A control pressed or released: its veil deepens faster than a hover, so that the press
/// answers the finger at once (a choice of UI-01, under the 110 ms of a hover).
pub const PRESS_FADE: Duration = Duration::from_millis(90);

/// An icon of the bar changing level or state (a Wi-Fi arc, a sound muted): a short
/// cross-fade rather than a jump (UI-01), a hover's.
pub const GLYPH_FADE: Duration = FADE;
/// One step of an icon that cycles while it waits: the arcs of a Wi-Fi connecting (UI-01).
pub const PULSE: Duration = Duration::from_millis(400);

/// One turn of a loading indicator (UI-01).
pub const SPIN: Duration = Duration::from_millis(900);
/// The positions of a turn of the loading indicator: one every 10°.
pub const SPIN_STEPS: u32 = 36;

/// Rows of a list moving to their new place, the panel holding them growing or shrinking, the
/// selection sliding from one row to the next (lot P-02d): brisk, without overshoot, so that a
/// row never passes its place nor the panel its height (ratio 0.97), settled in about 250 ms.
pub const LIST_SPRING: Spring = Spring::new(520.0, 44.0);
/// A row appearing in a list: it fades in while it slides down to its place.
pub const LIST_APPEAR: Duration = Duration::from_millis(160);

/// Scale of a window at the start of its opening.
pub const OPEN_SCALE: f64 = 0.92;
/// Fade of the snap preview appearing, and of hovers: the duration of a hover, in the 110 to
/// 150 ms range.
pub const FADE: Duration = Duration::from_millis(crate::metrics::HOVER_MS.1);

/// Opening of a window in the compositor, as on macOS (lots D-34, C-01): from its first
/// content, it grows from `WINDOW_OPEN_SCALE` to 1 around its center, title bar and shadow
/// included, and goes from transparent to opaque, both over this duration on
/// `WINDOW_OPEN_EASE`.
pub const WINDOW_OPEN: Duration = Duration::from_millis(250);
/// Scale of a window at the start of its opening.
pub const WINDOW_OPEN_SCALE: f64 = 0.90;
/// Curve of the opening: a frank ease-out, without a bounce. Half the way is done in a fifth of
/// the time, the rest settles gently.
pub const WINDOW_OPEN_EASE: Ease = Ease::OutCubic;
/// Closing of a window (lots D-34, C-01): its last image shrinks to `WINDOW_CLOSE_SCALE` and
/// fades out over this duration on `WINDOW_CLOSE_EASE`, then it is freed.
pub const WINDOW_CLOSE: Duration = Duration::from_millis(180);
/// Scale of a window at the end of its closing.
pub const WINDOW_CLOSE_SCALE: f64 = 0.92;
/// Curve of the closing: an ease-in, close to CSS `ease-in`. The window leaves slowly, then
/// faster: it never lingers once nearly transparent.
pub const WINDOW_CLOSE_EASE: Ease = Ease::InQuad;

/// Menus and panels of the shell (the bar's, the dock's) that close without their own animation
/// (lot UI-06): a click elsewhere, a workspace switch, another menu taking their place. The
/// compositor keeps their last image, which shrinks to `POPUP_CLOSE_SCALE` and fades out over
/// `POPUP_CLOSE` on `WINDOW_CLOSE_EASE`, as quickly as a menu fades on a Mac.
pub const POPUP_CLOSE: Duration = Duration::from_millis(crate::metrics::HOVER_MS.1);
/// Scale of a shell popup at the end of that closing: the size a panel unfolds from.
pub const POPUP_CLOSE_SCALE: f64 = 0.96;

/// Scale and opacity of a shell popup closing, `opacity` going from 1 to 0 on its fade.
pub fn popup_closing(opacity: f32) -> (f64, f32) {
    let opacity = opacity.clamp(0.0, 1.0);
    let scale = POPUP_CLOSE_SCALE + (1.0 - POPUP_CLOSE_SCALE) * f64::from(opacity);
    (scale, opacity)
}

/// Menus and popups of the windows appear with this fade, without moving: short, so that a
/// menu is readable at once (lot C-01). The lower end of a hover.
pub const MENU_FADE: Duration = Duration::from_millis(crate::metrics::HOVER_MS.0);

/// Minimize and restore of a window (lot D-19): the genie effect into its dock icon, or the
/// scale fallback. About half a second in lot D-19, found slow on the machine: 300 ms since
/// lot Q-14, the phases keeping their shares.
pub const MINIMIZE: Duration = Duration::from_millis(300);
/// Scale of a window at the end of the minimize fallback, at the bottom of its screen.
pub const MINIMIZE_SCALE: f64 = 0.2;

/// Raise of a window that was partly hidden (lot D-19d): from the dock, the window switcher, a
/// click, Mission Control or its application. The whole window grows from `RAISE_SCALE` to 1
/// around its center on this spring (ratio 1.0: no visible bounce, settled in about 200 ms),
/// and its shadow grows and softens from `RAISE_SHADOW` of its offset and blur to its own.
pub const RAISE_SPRING: Spring = Spring::new(1000.0, 63.3);
/// Scale of a raised window at the start of its raise.
pub const RAISE_SCALE: f64 = 0.97;
/// Share of the shadow's offset and blur at the start of a raise.
pub const RAISE_SHADOW: f64 = 0.6;
/// The windows a raised window now covers step back, then come back, over this duration: down
/// to `RECEDE_SCALE` around their center at the middle, along `recede`.
pub const RECEDE: Duration = Duration::from_millis(200);
/// Scale of a window that steps back, at the middle of its step.
pub const RECEDE_SCALE: f64 = 0.995;

/// Share of the step back at the share of the time `t`, from 0 to 1: there and back, half a
/// period of a sinusoid, level at both ends and 1 at the middle.
pub fn recede(t: f32) -> f32 {
    (std::f32::consts::PI * t.clamp(0.0, 1.0)).sin()
}

/// A fullscreen window covers the bar and the dock (lot D-21e). The pointer rests this long
/// against an edge before the surfaces along it come back over the window, as on macOS: a
/// pointer passing by, or aiming at the top of the window, does not bring them out.
pub const EDGE_REVEAL_DELAY: Duration = Duration::from_millis(150);
/// Their slide in, and out once the pointer has left them, on `Ease::OutCubic`.
pub const EDGE_REVEAL: Duration = Duration::from_millis(250);

/// Fade of the whole desktop from one theme to the other (`color::ThemeFade`). A choice of
/// D-05b: the 0.25 s of the cross-fades between two animations of Sherlock
/// (`docs/contexte.md`), longer than a hover, since every surface changes at once.
pub const THEME_FADE: Duration = Duration::from_millis(250);

/// A drag and drop refused or cancelled (D-29c): what was dragged glides back to where it was
/// taken, on `Ease::OutCubic`, and fades a little on the way, as on macOS.
pub const DRAG_RETURN: Duration = Duration::from_millis(300);
/// Opacity of the dragged icon when it is back where it was taken.
pub const DRAG_RETURN_ALPHA: f32 = 0.4;

/// Frame rate the desktop must hold, animations included (`docs/contexte.md`).
pub const FRAME_RATE: u32 = 60;

/// Precision below which an animated geometry is settled, in pixels.
pub const GEOMETRY_PRECISION: f64 = 0.5;
/// Precision below which an animated scale is settled.
pub const SCALE_PRECISION: f64 = 0.001;

/// The "Reduce motion" setting (UI-11): when on, what only decorates a change (a scrollbar
/// that fades, widens or stretches at a limit) jumps to its end instead of moving. The
/// desktop has no such setting yet: a program that reads one calls `set_reduced` at start-up
/// and when it changes. Off by default.
static REDUCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turns "Reduce motion" on or off for the whole process.
pub fn set_reduced(on: bool) {
    REDUCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether "Reduce motion" is on.
pub fn reduced() -> bool {
    REDUCED.load(std::sync::atomic::Ordering::Relaxed)
}

/// The « Effets visuels » setting of Control Room's Accessibility page (lot D-26p): the light
/// mode of old machines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Effects {
    /// Every effect: the genie, the 3D Sherlock, the live thumbnails of Mission Control.
    #[default]
    Full,
    /// The light mode: a fade instead of the genie, Sherlock as a still image, Mission
    /// Control's thumbnails without the live windows.
    Reduced,
    /// No effect: windows and views appear and go at once.
    None,
}

impl Effects {
    pub const ALL: [Self; 3] = [Self::Full, Self::Reduced, Self::None];

    /// The word of the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Reduced => "reduced",
            Self::None => "none",
        }
    }
}

static EFFECTS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// Sets the visual effects for the whole process (lot D-26p).
pub fn set_effects(effects: Effects) {
    let value = Effects::ALL.iter().position(|e| *e == effects).unwrap_or(0) as u8;
    EFFECTS.store(value, std::sync::atomic::Ordering::Relaxed);
}

/// The visual effects of the process.
pub fn effects() -> Effects {
    let value = EFFECTS.load(std::sync::atomic::Ordering::Relaxed);
    Effects::ALL[usize::from(value).min(Effects::ALL.len() - 1)]
}

/// `duration`, or nothing while "Reduce motion" is on.
pub fn unless_reduced(duration: Duration) -> Duration {
    if reduced() { Duration::ZERO } else { duration }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Spring; 4] = [SNAP_SPRING, OPEN_SPRING, PREVIEW_SPRING, TOGGLE_SPRING];

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
    fn a_raised_window_settles_in_about_200_ms_without_a_bounce() {
        let values = trajectory(RAISE_SPRING, 1000);
        assert!(values.iter().all(|v| *v <= 1.0 + 1e-9), "no bounce");
        // The scale is settled when it is within `SCALE_PRECISION` of 1.
        let share = SCALE_PRECISION / (1.0 - RAISE_SCALE);
        let settled = values.iter().rposition(|v| 1.0 - v > share).unwrap() + 1;
        assert!((150..=230).contains(&settled), "settled at {settled} ms");
        // The step back of the covered windows: there and back.
        assert_eq!(recede(0.0), 0.0);
        assert!(recede(1.0).abs() < 1e-6 && (recede(0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_window_opens_and_closes_like_on_macos() {
        // Opening: about 250 ms, from 0.90, a frank ease-out without a bounce.
        assert_eq!(WINDOW_OPEN, ms(250));
        assert_eq!(WINDOW_OPEN_SCALE, 0.90);
        let open = Fade::eased(0.0, 1.0, ms(0), WINDOW_OPEN, WINDOW_OPEN_EASE);
        let values: Vec<f32> = (0..=300).map(|t| open.value(ms(t))).collect();
        assert!(values.windows(2).all(|w| w[1] >= w[0]), "it only grows");
        assert!(values.iter().all(|v| *v <= 1.0), "no bounce");
        assert!(values[50] > 0.45, "brisk: {} at 50 ms", values[50]);
        assert!(values[150] > 0.9 && values[150] < 1.0, "{}", values[150]);
        assert_eq!(values[250], 1.0);
        // Closing: about 180 ms, to 0.92, slow first then faster.
        assert_eq!(WINDOW_CLOSE, ms(180));
        assert_eq!(WINDOW_CLOSE_SCALE, 0.92);
        let close = Fade::eased(1.0, 0.0, ms(0), WINDOW_CLOSE, WINDOW_CLOSE_EASE);
        assert!(close.value(ms(45)) > 0.9, "{}", close.value(ms(45)));
        assert!(close.value(ms(135)) < 0.5, "{}", close.value(ms(135)));
        assert_eq!(close.value(ms(180)), 0.0);
        assert!(WINDOW_CLOSE < WINDOW_OPEN && WINDOW_OPEN > FADE);
        // A shell popup closes faster than a window, and shrinks less (lot UI-06).
        assert!(POPUP_CLOSE < WINDOW_CLOSE && POPUP_CLOSE_SCALE > WINDOW_CLOSE_SCALE);
        assert_eq!(popup_closing(1.0), (1.0, 1.0));
        assert_eq!(popup_closing(0.0), (POPUP_CLOSE_SCALE, 0.0));
        let (half, _) = popup_closing(0.5);
        assert!(half > POPUP_CLOSE_SCALE && half < 1.0);
    }

    #[test]
    fn every_curve_goes_from_0_to_1_without_a_bounce() {
        for ease in [Ease::OutCubic, Ease::OutQuint, Ease::InQuad, Ease::InCubic] {
            assert_eq!((ease.apply(0.0), ease.apply(1.0)), (0.0, 1.0), "{ease:?}");
            assert_eq!((ease.apply(-1.0), ease.apply(2.0)), (0.0, 1.0), "{ease:?}");
            let values: Vec<f32> = (0..=100).map(|i| ease.apply(i as f32 / 100.0)).collect();
            assert!(values.windows(2).all(|w| w[1] >= w[0]), "{ease:?}");
        }
        // Out: ahead of the time; in: behind it.
        assert!(Ease::OutQuint.apply(0.2) > Ease::OutCubic.apply(0.2));
        assert!(Ease::InCubic.apply(0.5) < Ease::InQuad.apply(0.5));
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
