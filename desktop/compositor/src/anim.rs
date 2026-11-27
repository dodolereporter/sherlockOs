//! Motion of the windows, of the snap preview and of the workspaces: the springs and fades of
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
    /// Share of the corners, border and shadow drawn: 1 for a floating or snapped window, 0
    /// for a maximized or fullscreen one, in between while it changes.
    pub decoration: f32,
    /// Scale of the whole window while it opens, is raised or steps back (1 at rest): its title
    /// bar, corners and shadow follow it (lots C-01, D-19d).
    pub scale: f64,
    /// Share of the shadow's offset and blur: 1 at rest, less while the window is raised, its
    /// shadow growing back (lot D-19d).
    pub shadow: f64,
}

/// `rect` scaled by `scale` around its center.
pub fn scaled(rect: RectF, scale: f64) -> RectF {
    let size = rect.size.upscale(scale);
    let loc = rect.loc + (rect.size.to_point() - size.to_point()).downscale(2.0);
    Rectangle::new(loc, size)
}

/// The content `rect` of a window with a title bar of `inset` above it, when the whole window,
/// bar included, is scaled by `scale` around its center: the bar shrinks with the content and
/// stays on it.
pub fn scaled_whole(rect: RectF, inset: f64, scale: f64) -> RectF {
    let whole = Rectangle::new(
        (rect.loc.x, rect.loc.y - inset).into(),
        (rect.size.w, rect.size.h + inset).into(),
    );
    let whole = scaled(whole, scale);
    let inset = inset * scale;
    Rectangle::new(
        (whole.loc.x, whole.loc.y + inset).into(),
        (whole.size.w, whole.size.h - inset).into(),
    )
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

/// How long a window waits for its client to take the size it was given, drawn stretched at
/// that size, before following its real geometry again (a client that refuses the size).
pub const SIZE_WAIT: Duration = Duration::from_secs(1);

/// Motion of a window: opening (scale and opacity, from its first content), then geometry changes announced by
/// `animate_next_change` or `animate_to`, and its decoration (corners, border, shadow). Other
/// changes, like those of a drag or a resize, are followed without delay.
#[derive(Debug, Default)]
pub struct WindowMotion {
    /// Opening: its progress from 0 to 1, on `motion::WINDOW_OPEN_EASE`, which gives the
    /// opacity and the scale. `None` once finished.
    open: Option<Fade>,
    opened: bool,
    /// Shown at once at its first frame: fullscreen, or window animations off (lot D-34).
    no_opening: bool,
    /// Animated geometry. `None` at rest: the window is drawn at its real place.
    geometry: Option<AnimatedRect>,
    /// Last geometry drawn, before the opening scale.
    last: Option<RectF>,
    animate_next: bool,
    /// Size the client was asked for, not taken yet, and until when to wait for it (set at the
    /// first frame that sees it).
    expected: Option<(RectF, Option<Duration>)>,
    /// Maximized or fullscreen: the decoration goes away.
    bare: bool,
    /// Raise (lot D-19d): the scale, from `motion::RAISE_SCALE` to 1. `None` at rest.
    raise: Option<Animation>,
    /// Step back behind a raised window (lot D-19d): when it started. `None` at rest.
    recede: Option<Duration>,
    /// Decoration drawn at the last frame, and its animation towards the state.
    decoration: f64,
    decoration_motion: Option<Animation>,
}

impl WindowMotion {
    /// The next geometry change will be animated: snap, maximize, restore. The flag only holds
    /// for the next frame: the geometry has already changed there, or will not change.
    pub fn animate_next_change(&mut self) {
        self.animate_next = true;
    }

    /// The window slides to `rect`, the place and size it was just given: from the next frame
    /// on, without waiting for the client to take the size. Its content is stretched there
    /// until the client draws at that size, for at most `SIZE_WAIT`.
    pub fn animate_to(&mut self, rect: Rectangle<i32, Logical>) {
        self.animate_next = true;
        self.expected = Some((rect.to_f64(), None));
    }

    /// Stops the geometry animation: a gesture starts, the window follows the pointer.
    pub fn follow(&mut self) {
        self.geometry = None;
        self.animate_next = false;
        self.expected = None;
    }

    /// The window comes to the front from behind another one (lot D-19d): it grows to its size
    /// on `motion::RAISE_SPRING`, from `motion::RAISE_SCALE`, its shadow with it. Raised again,
    /// or raised while it steps back, it grows from the scale it is drawn at: no jump.
    pub fn raise(&mut self, now: Duration) {
        let from = match (self.raise, self.recede) {
            (None, None) => motion::RAISE_SCALE,
            _ => self.scale_at(now),
        };
        self.recede = None;
        self.raise = Some(Animation::new(motion::RAISE_SPRING, from, 1.0, now));
    }

    /// A raised window now covers this one: it steps back and comes back over `motion::RECEDE`.
    /// A step under way goes on, without a jump.
    pub fn recede(&mut self, now: Duration) {
        if self.recede.is_none() {
            self.recede = Some(now);
        }
    }

    /// Scale of the raise and of the step back at `now`, 1 without them.
    fn scale_at(&self, now: Duration) -> f64 {
        let raise = self.raise.map_or(1.0, |r| r.value(now));
        let back = self.recede.map_or(0.0, |start| {
            let run = now.saturating_sub(start).as_secs_f32() / motion::RECEDE.as_secs_f32();
            motion::recede(run)
        });
        raise * (1.0 - (1.0 - motion::RECEDE_SCALE) * f64::from(back))
    }

    /// Scale and shadow share of the raise and of the step back at `now`; both forgotten once
    /// over.
    fn lift(&mut self, now: Duration) -> (f64, f64) {
        if self
            .raise
            .is_some_and(|r| r.is_done(now, motion::SCALE_PRECISION))
        {
            self.raise = None;
        }
        if self
            .recede
            .is_some_and(|start| now >= start + motion::RECEDE)
        {
            self.recede = None;
        }
        let raise = self.raise.map_or(1.0, |r| r.value(now));
        let risen = ((raise - motion::RAISE_SCALE) / (1.0 - motion::RAISE_SCALE)).clamp(0.0, 1.0);
        let shadow = motion::RAISE_SHADOW + (1.0 - motion::RAISE_SHADOW) * risen;
        (self.scale_at(now), shadow)
    }

    /// The window is maximized or fullscreen (`bare`): its corners, border and shadow go away
    /// on `DECORATION_SPRING`, and come back when it leaves that state.
    pub fn set_bare(&mut self, bare: bool) {
        self.bare = bare;
    }

    /// The window has not been drawn yet: it will appear at once, without its opening. Called
    /// for a fullscreen window, or with window animations off (lot D-34).
    pub fn skip_opening(&mut self) {
        self.no_opening = !self.opened;
    }

    /// Where to draw the window whose real geometry is `target`: there, or at the size it was
    /// given while its client has not taken it yet. Its place is always the real one, which
    /// the compositor sets at once.
    fn goal(&mut self, now: Duration, target: RectF) -> RectF {
        let Some((rect, mut until)) = self.expected else {
            return target;
        };
        let until = *until.get_or_insert(now + SIZE_WAIT);
        if self.last.is_none() || target.size == rect.size {
            // Never drawn, or taken: the real geometry.
            self.expected = None;
            return target;
        }
        if now >= until {
            // Refused: the window slides back to the size its client kept.
            self.expected = None;
            self.animate_next = true;
            return target;
        }
        self.expected = Some((rect, Some(until)));
        Rectangle::new(target.loc, rect.size)
    }

    /// Decoration to draw at `now`: settled on the state, or on its way there.
    fn decoration(&mut self, now: Duration, first: bool) -> f64 {
        let goal = if self.bare { 0.0 } else { 1.0 };
        match &mut self.decoration_motion {
            _ if first => self.decoration = goal,
            Some(animation) if animation.target() != goal => animation.retarget(now, goal),
            None if self.decoration != goal => {
                let from = self.decoration;
                let animation = Animation::new(motion::DECORATION_SPRING, from, goal, now);
                self.decoration_motion = Some(animation);
            }
            _ => {}
        }
        self.decoration = match self.decoration_motion {
            Some(a) if !a.is_done(now, motion::SCALE_PRECISION) => a.value(now).clamp(0.0, 1.0),
            _ => {
                self.decoration_motion = None;
                goal
            }
        };
        self.decoration
    }

    /// What to draw at time `now` for a window whose real geometry is `target`. The first call
    /// starts the opening.
    pub fn frame(&mut self, now: Duration, target: Rectangle<i32, Logical>) -> Visual {
        let target = target.to_f64();
        let first = !self.opened;
        if first && !self.no_opening {
            self.open = Some(Fade::eased(
                0.0,
                1.0,
                now,
                motion::WINDOW_OPEN,
                motion::WINDOW_OPEN_EASE,
            ));
        }
        self.opened = true;
        let decoration = self.decoration(now, first) as f32;
        let goal = self.goal(now, target);
        // The flag is consumed on every frame, whether an animation starts or not: otherwise the
        // next drag or resize would be animated. A size still awaited keeps it.
        let animate = std::mem::take(&mut self.animate_next) || self.expected.is_some();
        match (&mut self.geometry, self.last) {
            (Some(geometry), _) => geometry.retarget(now, goal),
            (None, Some(last)) if animate && last != goal => {
                self.geometry = Some(AnimatedRect::new(motion::SNAP_SPRING, last, goal, now));
            }
            // Never drawn: nothing to start from, it will appear at its place.
            _ => {}
        }
        let rect = match self.geometry {
            Some(geometry) if !geometry.is_done(now) => geometry.value(now),
            _ => {
                self.geometry = None;
                goal
            }
        };
        self.last = Some(rect);

        let progress = match self.open {
            Some(open) if !open.is_done(now) => open.value(now),
            _ => {
                self.open = None;
                1.0
            }
        };
        let start = motion::WINDOW_OPEN_SCALE;
        let (lift, shadow) = self.lift(now);
        Visual {
            rect,
            opacity: progress,
            moving: self.geometry.is_some() || self.expected.is_some(),
            decoration,
            scale: (start + (1.0 - start) * f64::from(progress)) * lift,
            shadow,
        }
    }

    /// An animation is running: the drawn geometry is not the real geometry.
    pub fn is_animating(&self) -> bool {
        self.open.is_some()
            || self.geometry.is_some()
            || self.expected.is_some()
            || self.decoration_motion.is_some()
            || self.raise.is_some()
            || self.recede.is_some()
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
            decoration: 1.0,
            scale: 1.0,
            shadow: 1.0,
        })
    }
}

/// Slide from one workspace to another. The workspaces stand in a row, side by side; the view
/// is a position along that row, in workspaces (0 is the first one), carried by `SNAP_SPRING`
/// to the active one. Each screen shows its own part of the row: a workspace is shifted by its
/// distance to the view times the width of the screen. Only the workspaces the slide goes
/// through are drawn: the spring's overshoot does not show the next one.
///
/// A touchpad swipe (lot D-31) holds the view where the fingers put it (`hold`), then lets it
/// go on the same spring with the velocity of the fingers (`release`).
#[derive(Debug, Clone, Copy, Default)]
pub struct WorkspaceSlide {
    view: Option<Animation>,
    /// First and last index of the workspaces the slide goes through.
    span: (usize, usize),
    /// Position held by a swipe, in workspaces.
    held: Option<f64>,
}

impl WorkspaceSlide {
    /// Half a pixel on a screen 4096 wide, in workspaces.
    const PRECISION: f64 = motion::GEOMETRY_PRECISION / 4096.0;

    /// Slides from the workspace at `from` (the one shown at rest) to the one at `to`. During a
    /// slide, the view leaves from where it is: nothing jumps.
    pub fn go(&mut self, now: Duration, from: usize, to: usize) {
        self.release(now, from, 0.0);
        let span = match &mut self.view {
            Some(view) if !view.is_done(now, Self::PRECISION) => {
                let at = view.value(now).max(0.0);
                view.retarget(now, to as f64);
                (at.floor() as usize, at.ceil() as usize)
            }
            _ => {
                let (from_f, to_f) = (from as f64, to as f64);
                self.view = Some(Animation::new(motion::SNAP_SPRING, from_f, to_f, now));
                (from, from)
            }
        };
        self.span = (span.0.min(to), span.1.max(to));
    }

    /// Stops the slide: the active workspace is shown at once.
    pub fn stop(&mut self) {
        self.view = None;
        self.held = None;
    }

    /// Where the view is at `now`, in workspaces, when the one at `active` is the active one.
    pub fn position(&self, now: Duration, active: usize) -> f64 {
        match (self.held, self.view) {
            (Some(held), _) => held,
            (None, Some(view)) if self.is_animating(now) => view.value(now),
            _ => active as f64,
        }
    }

    /// Holds the view at `view`, in workspaces: it follows the fingers, without a spring.
    pub fn hold(&mut self, view: f64) {
        self.held = Some(view);
    }

    /// Lets go of the view held by `hold`: it leaves with `velocity` (workspaces per second)
    /// towards the workspace at `active`. `go` then turns it towards another one without a
    /// jolt.
    pub fn release(&mut self, now: Duration, active: usize, velocity: f64) {
        let Some(at) = self.held.take() else {
            return;
        };
        let to = active as f64;
        self.view = Some(Animation::with_velocity(
            motion::SNAP_SPRING,
            at,
            to,
            velocity,
            now,
        ));
        let at = at.max(0.0);
        self.span = (
            (at.floor() as usize).min(active),
            (at.ceil() as usize).max(active),
        );
    }

    pub fn is_animating(&self, now: Duration) -> bool {
        self.view.is_some_and(|v| !v.is_done(now, Self::PRECISION))
    }

    /// Horizontal shift at `now` of the workspace at `index`, on a screen `width` wide, when
    /// the one at `active` is the active one. `None` when it is out of the screen.
    pub fn shift(&self, now: Duration, index: usize, active: usize, width: f64) -> Option<f64> {
        let view = match (self.held, self.view) {
            (Some(held), _) => held,
            (None, Some(view)) if self.is_animating(now) => {
                if index < self.span.0 || index > self.span.1 {
                    return None;
                }
                view.value(now)
            }
            _ => active as f64,
        };
        let distance = index as f64 - view;
        (distance.abs() < 1.0).then_some(distance * width)
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
        assert_eq!(first.scale, motion::WINDOW_OPEN_SCALE);
        // The place is the real one: the scale is applied around the whole window, title bar
        // included (`scaled_whole`).
        assert_eq!(first.rect, place.to_f64());
        let midway = motion.frame(ms(60), place);
        assert!(midway.opacity > 0.0 && midway.opacity < 1.0);
        assert!(midway.scale > motion::WINDOW_OPEN_SCALE && midway.scale < 1.0);
        assert!(motion.is_animating());
        let end = motion.frame(ms(1000), place);
        assert_eq!(
            (end.rect, end.opacity, end.scale),
            (place.to_f64(), 1.0, 1.0)
        );
        assert!(!motion.is_animating());
    }

    #[test]
    fn the_opening_grows_and_fades_in_over_250_ms_without_a_bounce() {
        let mut motion = WindowMotion::default();
        let place = rect(100, 100, 400, 300);
        let steps: Vec<Visual> = [0, 50, 100, 150, 200, 249]
            .map(|t| motion.frame(ms(t), place))
            .to_vec();
        // Scale and opacity only grow, together, and are still moving at 200 ms.
        for pair in steps.windows(2) {
            assert!(pair[1].scale > pair[0].scale);
            assert!(pair[1].opacity > pair[0].opacity);
        }
        assert!(steps[1].opacity > 0.45, "a frank ease-out");
        let at_200 = steps[4];
        assert!(at_200.opacity < 1.0 && at_200.scale < 1.0);
        assert!(at_200.scale > 0.99, "nearly there");
        assert!(steps.iter().all(|v| v.scale <= 1.0 && v.opacity <= 1.0));
        assert!(motion.is_animating());
        assert_eq!(motion.frame(ms(250), place).scale, 1.0);
        assert!(!motion.is_animating());
    }

    /// A window opened at once, as with animations off: at rest from its first frame.
    fn at_rest() -> WindowMotion {
        let mut motion = WindowMotion::default();
        motion.skip_opening();
        motion.frame(ms(0), rect(100, 100, 400, 300));
        motion
    }

    #[test]
    fn a_raised_window_grows_its_shadow_with_it_then_rests() {
        let (mut motion, place) = (at_rest(), rect(100, 100, 400, 300));
        motion.raise(ms(1000));
        let steps: Vec<Visual> = [1000, 1050, 1100, 1150]
            .map(|t| motion.frame(ms(t), place))
            .to_vec();
        assert_eq!(steps[0].scale, motion::RAISE_SCALE);
        assert_eq!(steps[0].shadow, motion::RAISE_SHADOW);
        for pair in steps.windows(2) {
            assert!(pair[1].scale > pair[0].scale && pair[1].scale <= 1.0);
            assert!(pair[1].shadow > pair[0].shadow && pair[1].shadow <= 1.0);
        }
        // Its place and opacity do not move.
        assert!(
            steps
                .iter()
                .all(|v| v.rect == place.to_f64() && v.opacity == 1.0)
        );
        assert!(motion.is_animating());
        let end = motion.frame(ms(1250), place);
        assert_eq!((end.scale, end.shadow), (1.0, 1.0));
        assert!(!motion.is_animating());
    }

    #[test]
    fn a_covered_window_steps_back_and_comes_back() {
        let (mut motion, place) = (at_rest(), rect(100, 100, 400, 300));
        motion.recede(ms(1000));
        let scale = |motion: &mut WindowMotion, t| motion.frame(ms(t), place).scale;
        assert_eq!(scale(&mut motion, 1000), 1.0);
        let middle = scale(&mut motion, 1100);
        assert!((middle - motion::RECEDE_SCALE).abs() < 1e-6, "{middle}");
        assert!(scale(&mut motion, 1150) > middle);
        assert_eq!(motion.frame(ms(1200), place).shadow, 1.0);
        assert_eq!(scale(&mut motion, 1200), 1.0);
        assert!(!motion.is_animating());
    }

    #[test]
    fn a_raise_turned_around_halfway_goes_on_without_a_jump() {
        let place = rect(100, 100, 400, 300);
        // Raised, then covered again by the window it had just covered.
        let mut raised = at_rest();
        raised.raise(ms(1000));
        let before = raised.frame(ms(1060), place).scale;
        raised.recede(ms(1060));
        assert_eq!(raised.frame(ms(1060), place).scale, before);
        assert!(raised.frame(ms(1120), place).scale < 1.0);
        assert_eq!(raised.frame(ms(1500), place).scale, 1.0);
        // Stepping back, then raised: it grows from where it is, not from `RAISE_SCALE`.
        let mut covered = at_rest();
        covered.recede(ms(1000));
        let before = covered.frame(ms(1060), place).scale;
        covered.raise(ms(1060));
        let now = covered.frame(ms(1060), place).scale;
        assert!((now - before).abs() < 1e-9, "{before} then {now}");
        assert!(covered.frame(ms(1100), place).scale > now);
        assert_eq!(covered.frame(ms(1500), place).scale, 1.0);
        assert!(!covered.is_animating());
    }

    #[test]
    fn the_title_bar_scales_with_the_window_around_the_center_of_both() {
        let place = rect(100, 142, 400, 300).to_f64();
        let drawn = scaled_whole(place, 42.0, 0.9);
        // The whole window, 342 high from 100, scaled around its middle at 271.
        let top = drawn.loc.y - 42.0 * 0.9;
        assert!((top - (271.0 - 342.0 * 0.9 / 2.0)).abs() < 1e-9);
        assert!((drawn.loc.y + drawn.size.h - (271.0 + 342.0 * 0.9 / 2.0)).abs() < 1e-9);
        assert!((drawn.size.w - 360.0).abs() < 1e-9);
        assert!((drawn.size.h - 270.0).abs() < 1e-9);
        // Without a bar: the content around its own center.
        assert_eq!(scaled_whole(place, 0.0, 0.9), scaled(place, 0.9));
    }

    #[test]
    fn a_window_whose_opening_is_skipped_appears_at_once() {
        let mut motion = WindowMotion::default();
        let place = rect(0, 0, 800, 600);
        motion.skip_opening();
        let first = motion.frame(ms(0), place);
        assert_eq!(
            (first.rect, first.opacity, first.scale),
            (place.to_f64(), 1.0, 1.0)
        );
        assert!(!motion.is_animating());
        // Too late once drawn: nothing changes.
        motion.skip_opening();
        assert_eq!(motion.frame(ms(10), place).opacity, 1.0);
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
    fn a_new_size_slides_without_waiting_for_the_client() {
        // Maximized under a dock that goes away: same place, 76 more in height.
        let (under_dock, whole) = (rect(0, 0, 800, 524), rect(0, 0, 800, 600));
        let mut motion = WindowMotion::default();
        motion.frame(ms(0), under_dock);
        motion.frame(ms(1000), under_dock);
        motion.animate_to(whole);
        // The client has not answered yet: the window grows all the same, without a jump.
        let growing = motion.frame(ms(1010), under_dock).rect;
        assert_eq!(growing, under_dock.to_f64());
        let growing = motion.frame(ms(1050), under_dock).rect;
        assert!(growing.size.h > 524.0 && growing.size.h < 600.0);
        // Settled at the size given while the client is still drawing.
        assert_eq!(motion.frame(ms(1500), under_dock).rect, whole.to_f64());
        assert!(motion.is_animating());
        // The client takes it: nothing moves, and the wait is over.
        assert_eq!(motion.frame(ms(1600), whole).rect, whole.to_f64());
        assert!(!motion.is_animating());
    }

    #[test]
    fn a_refused_size_is_not_awaited_forever() {
        let (small, whole) = (rect(0, 0, 300, 200), rect(0, 0, 800, 600));
        let mut motion = WindowMotion::default();
        motion.frame(ms(0), small);
        motion.frame(ms(1000), small);
        motion.animate_to(whole);
        motion.frame(ms(1010), small);
        assert_eq!(motion.frame(ms(1900), small).rect, whole.to_f64());
        // Past the wait, the window slides back to the size its client kept.
        let back = motion.frame(ms(1010) + SIZE_WAIT, small).rect;
        assert_eq!(back, whole.to_f64(), "no jump");
        assert_eq!(motion.frame(ms(5000), small).rect, small.to_f64());
        assert!(!motion.is_animating());
    }

    #[test]
    fn maximized_windows_lose_their_decoration_on_a_spring() {
        let place = rect(100, 100, 400, 300);
        let mut motion = WindowMotion::default();
        // Maximized from the start: never decorated, nothing animated.
        motion.set_bare(true);
        assert_eq!(motion.frame(ms(0), place).decoration, 0.0);
        let mut motion = WindowMotion::default();
        assert_eq!(motion.frame(ms(0), place).decoration, 1.0);
        motion.frame(ms(1000), place);
        motion.set_bare(true);
        let decorations: Vec<f32> = (1000..1400)
            .step_by(10)
            .map(|t| motion.frame(ms(t), place).decoration)
            .collect();
        assert_eq!(decorations[0], 1.0);
        assert!(
            decorations.windows(2).all(|d| d[1] <= d[0]),
            "it goes one way"
        );
        assert!(decorations.iter().any(|d| *d > 0.1 && *d < 0.9), "no jump");
        assert_eq!(motion.frame(ms(2000), place).decoration, 0.0);
        assert!(!motion.is_animating());
        // Restored halfway back: it turns around from where it is.
        motion.set_bare(false);
        motion.frame(ms(2000), place);
        let midway = motion.frame(ms(2040), place).decoration;
        motion.set_bare(true);
        let turned = motion.frame(ms(2041), place).decoration;
        assert!((turned - midway).abs() < 0.05 && turned > 0.0);
        assert_eq!(motion.frame(ms(3000), place).decoration, 0.0);
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

    #[test]
    fn workspaces_slide_side_by_side() {
        let mut slide = WorkspaceSlide::default();
        let shift = |s: &WorkspaceSlide, now, index| s.shift(ms(now), index, 1, 800.0);
        // At rest, only the active workspace is on the screen.
        assert_eq!(shift(&slide, 0, 1), Some(0.0));
        assert_eq!((shift(&slide, 0, 0), shift(&slide, 0, 2)), (None, None));
        // From the first to the second: the first leaves to the left, the second comes in from
        // the right, and both are on the screen together.
        slide.go(ms(1000), 0, 1);
        assert_eq!(shift(&slide, 1000, 0), Some(0.0));
        assert_eq!(shift(&slide, 1000, 1), None, "not yet on the screen");
        let (out, coming) = (shift(&slide, 1050, 0), shift(&slide, 1050, 1));
        assert!(out.is_some_and(|x| x < 0.0 && x > -800.0));
        assert!(coming.is_some_and(|x| x > 0.0 && x < 800.0));
        assert!(
            coming
                .zip(out)
                .is_some_and(|(c, o)| (c - o - 800.0).abs() < 1e-9)
        );
        assert!(slide.is_animating(ms(1050)));
        assert_eq!(shift(&slide, 2000, 1), Some(0.0));
        assert_eq!(shift(&slide, 2000, 0), None);
        assert!(!slide.is_animating(ms(2000)));
        // Back halfway: the view turns around from where it is.
        slide.go(ms(3000), 1, 0);
        let before = shift(&slide, 3050, 1);
        slide.go(ms(3050), 1, 1);
        assert_eq!(shift(&slide, 3050, 1), before);
        slide.stop();
        assert_eq!(shift(&slide, 3060, 1), Some(0.0));
    }

    #[test]
    fn a_held_view_follows_then_leaves_with_its_velocity() {
        let mut slide = WorkspaceSlide::default();
        let shift = |s: &WorkspaceSlide, now, index| s.shift(ms(now), index, 0, 1000.0);
        // Held between the first and the second: both show, where the fingers put them.
        slide.hold(0.25);
        assert_eq!(shift(&slide, 0, 0), Some(-250.0));
        assert_eq!(shift(&slide, 0, 1), Some(750.0));
        assert_eq!(shift(&slide, 0, 2), None);
        assert!(
            !slide.is_animating(ms(0)),
            "no frame while the fingers rest"
        );
        assert_eq!(slide.position(ms(5000), 0), 0.25);
        // Past the first one: it shows beyond its edge, nothing on the other side.
        slide.hold(-0.1);
        assert_eq!(shift(&slide, 0, 0), Some(100.0));
        // Let go towards the second at 3 workspaces a second: it goes on at that speed.
        slide.hold(0.25);
        slide.release(ms(1000), 0, 3.0);
        slide.go(ms(1000), 0, 1);
        let early = slide.position(ms(1010), 1);
        // 0.25 + 3 × 0.01, and a little more from the spring pulling towards the second.
        assert!(early > 0.28 && early < 0.31, "{early}");
        assert!(slide.is_animating(ms(1010)));
        assert_eq!(slide.shift(ms(2000), 1, 1, 1000.0), Some(0.0));
        assert!(!slide.is_animating(ms(2000)));
        // A key during the swipe lets the view go from where it is.
        slide.hold(1.4);
        slide.go(ms(3000), 1, 0);
        assert!((slide.position(ms(3000), 0) - 1.4).abs() < 1e-9);
        assert_eq!(slide.position(ms(4000), 0), 0.0);
    }

    #[test]
    fn the_overshoot_does_not_show_the_next_workspace() {
        let mut slide = WorkspaceSlide::default();
        slide.go(ms(0), 0, 1);
        let shift = |t, index| slide.shift(ms(t), index, 1, 1920.0);
        assert!(
            (0..2000).any(|t| shift(t, 1).is_some_and(|x| x < 0.0)),
            "no overshoot"
        );
        assert_eq!(
            (0..2000).find_map(|t| shift(t, 2)),
            None,
            "the third one was drawn"
        );
    }
}
