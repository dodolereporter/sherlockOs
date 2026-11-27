//! The animations of each icon, kept with it by identity when the icons are built again: its
//! size under the pointer, its arrival and departure (room in the bar, scale, opacity), and
//! its offset from its place when the icons move (reordering, an icon dropped).
//!
//! An icon that moves is drawn where it was, then led back to its new place by a spring: the
//! offset is the old place minus the new one, and heads for 0.

use std::time::Duration;

use sherlock_ui::dock;
use sherlock_ui::motion::{Animation, Fade, GEOMETRY_PRECISION, SCALE_PRECISION};

use crate::items::{Item, Target};
use crate::layout::Slot;

/// The animations of one icon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Track {
    /// Side, led towards the size the pointer gives it.
    pub size: Animation,
    /// Share of its room in the bar: 0 absent, 1 in place. Its neighbours move aside as it
    /// opens.
    pub room: Animation,
    /// Scale it is drawn at: 0 absent, 1 in place.
    pub scale: Animation,
    pub opacity: Fade,
    /// Offset from its place, along the dock and away from its edge.
    pub along: Animation,
    pub depth: Animation,
    /// Opacity of the dot under the icon: 1 while its application has a window.
    pub dot: Fade,
    /// The dot is green, the application in front. Kept while the dot fades out.
    pub dot_active: bool,
}

impl Track {
    /// An icon of `side` in its place, at rest.
    pub fn settled(side: f32) -> Self {
        let still = |value| Animation::new(dock::SHIFT_SPRING, value, value, Duration::ZERO);
        let present = || Animation::new(dock::ARRIVAL_SPRING, 1.0, 1.0, Duration::ZERO);
        Self {
            size: Animation::new(dock::SPRING, side.into(), side.into(), Duration::ZERO),
            room: present(),
            scale: present(),
            opacity: Fade::new(1.0, 1.0, Duration::ZERO, Duration::ZERO),
            along: still(0.0),
            depth: still(0.0),
            dot: Fade::new(0.0, 0.0, Duration::ZERO, Duration::ZERO),
            dot_active: false,
        }
    }

    /// The dot shows (`open`) or goes from `now`, faded with `dock::DOT_FADE`; at once without
    /// `animate`. Its color follows `active` at once while it shows.
    pub fn show_dot(&mut self, now: Duration, open: bool, active: bool, animate: bool) {
        if open {
            self.dot_active = active;
        }
        let target = if open { 1.0 } else { 0.0 };
        if !animate {
            self.dot = Fade::new(target, target, now, Duration::ZERO);
        } else if self.dot.target() != target {
            self.dot.retarget(now, target, dock::DOT_FADE);
        }
    }

    /// An icon of `side` arriving at `now`: its room opens, it grows and fades in.
    pub fn arriving(side: f32, now: Duration) -> Self {
        let mut track = Self::settled(side);
        track.room = Animation::new(dock::ARRIVAL_SPRING, 0.0, 1.0, now);
        track.scale = track.room;
        track.opacity = Fade::new(0.0, 1.0, now, dock::ICON_FADE);
        track
    }

    /// Leads the icon in (`true`) or out of the dock from `now`, from where it is: its room,
    /// its scale and its opacity.
    pub fn show(&mut self, now: Duration, shown: bool) {
        let target = if shown { 1.0 } else { 0.0 };
        self.open(now, shown);
        if self.scale.target() != target {
            self.scale.retarget(now, target);
        }
        if self.opacity.target() != target as f32 {
            self.opacity.retarget(now, target as f32, dock::ICON_FADE);
        }
    }

    /// Opens or closes its room in the bar from `now`, without changing how it is drawn: an
    /// icon dragged out of the dock and back.
    pub fn open(&mut self, now: Duration, open: bool) {
        let target = if open { 1.0 } else { 0.0 };
        if self.room.target() != target {
            self.room.retarget(now, target);
        }
    }

    /// Vanishes where it is from `now`, from the opacity `from`: an icon dragged out and let go.
    pub fn vanish(&mut self, now: Duration, from: f32) {
        self.scale = Animation::new(dock::ARRIVAL_SPRING, self.scale.value(now), 0.0, now);
        self.opacity = Fade::new(from, 0.0, now, dock::ICON_FADE);
    }

    /// The icon is drawn `from` its place at `now` and heads for `to`: 0 to go back to its
    /// place, `from` to stay where it is.
    pub fn moved(&mut self, now: Duration, from: (f32, f32), to: (f32, f32)) {
        let lead =
            |from: f32, to: f32| Animation::new(dock::SHIFT_SPRING, from.into(), to.into(), now);
        self.along = lead(from.0, to.0);
        self.depth = lead(from.1, to.1);
    }

    /// Scale it is drawn at, never below 0 (the spring passes it a little).
    pub fn drawn_scale(&self, now: Duration) -> f32 {
        (self.scale.value(now) as f32).max(0.0)
    }

    /// The icon in the flow of the dock at `now`.
    pub fn slot(&self, now: Duration) -> Slot {
        Slot {
            side: self.size.value(now) as f32,
            room: (self.room.value(now) as f32).max(0.0),
            shift: (self.along.value(now) as f32, self.depth.value(now) as f32),
        }
    }

    /// Whether nothing of the icon moves any more at `now`.
    pub fn is_done(&self, now: Duration) -> bool {
        self.size.is_done(now, GEOMETRY_PRECISION)
            && self.room.is_done(now, SCALE_PRECISION)
            && self.scale.is_done(now, SCALE_PRECISION)
            && self.opacity.is_done(now)
            && self.along.is_done(now, GEOMETRY_PRECISION)
            && self.depth.is_done(now, GEOMETRY_PRECISION)
            && self.dot.is_done(now)
    }

    /// Whether an icon led out of the dock is gone.
    pub fn is_gone(&self, now: Duration) -> bool {
        self.scale.target() == 0.0 && self.is_done(now)
    }
}

/// An icon that left the dock and is still drawn while it goes.
#[derive(Debug, Clone)]
pub struct Leaving {
    pub item: Item,
    pub track: Track,
    /// The icon it comes before, closing its room there; `None` at the end.
    pub next: Option<Target>,
    /// It was after the separator.
    pub unpinned: bool,
    /// Dragged out of the dock and let go: where it vanishes, relative to the middle of the
    /// dock (`Layout::offset`), without room in the bar.
    pub dropped: Option<(f32, f32)>,
}

/// The puffs of a `Poof` at one instant: the center and radius of each, and their opacity.
pub type Puffs = (Vec<((f32, f32), f32)>, f32);

/// The puff of smoke where a pinned application was let go out of the dock (lot D-21d).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Poof {
    /// Its center, in the dock surface.
    pub at: (f32, f32),
    pub side: f32,
    pub start: Duration,
}

impl Poof {
    /// Each puff at `now`: its center and radius, and the opacity of all. Nothing once over.
    pub fn puffs(&self, now: Duration) -> Option<Puffs> {
        let t = now.saturating_sub(self.start).as_secs_f32() / dock::POOF_DURATION.as_secs_f32();
        if t >= 1.0 {
            return None;
        }
        // Out fast, then slower, as smoke that spreads.
        let eased = 1.0 - (1.0 - t).powi(3);
        let lerp = |(a, b): (f32, f32)| (a + (b - a) * eased) * self.side;
        let (drift, radius) = (lerp(dock::POOF_DRIFT), lerp(dock::POOF_RADIUS));
        let count = dock::POOF_PUFFS;
        let puffs = (0..count)
            .map(|k| {
                let angle =
                    std::f32::consts::TAU * k as f32 / count as f32 - std::f32::consts::FRAC_PI_2;
                let center = (
                    self.at.0 + drift * angle.cos(),
                    self.at.1 + drift * angle.sin(),
                );
                (center, radius)
            })
            .collect();
        Some((puffs, 1.0 - t * t))
    }

    pub fn is_done(&self, now: Duration) -> bool {
        now >= self.start + dock::POOF_DURATION
    }
}

/// Height of the bounce of an icon of `side` at `elapsed` into it: hops of `BOUNCE_PERIOD`,
/// each a parabola, as a thrown object falls, `BOUNCE_HEIGHT` of the side high.
pub fn hop(elapsed: Duration, side: f32) -> f32 {
    let period = dock::BOUNCE_PERIOD.as_secs_f32();
    let phase = (elapsed.as_secs_f32() / period).fract();
    4.0 * dock::BOUNCE_HEIGHT * side * phase * (1.0 - phase)
}

/// The bounce of an icon while its application starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounce {
    start: Duration,
    /// When it lands for good: the end of the hop in progress when it was told to stop.
    end: Option<Duration>,
}

impl Bounce {
    pub fn new(start: Duration) -> Self {
        Self { start, end: None }
    }

    /// Height above its place at `now`, for an icon of `side`.
    pub fn lift(&self, now: Duration, side: f32) -> f32 {
        if self.end.is_some_and(|end| now >= end) {
            return 0.0;
        }
        hop(now.saturating_sub(self.start), side)
    }

    /// Stops at the end of the hop in progress at `now`: the icon lands, nothing jumps.
    pub fn stop(&mut self, now: Duration) {
        if self.end.is_some() {
            return;
        }
        let period = dock::BOUNCE_PERIOD.as_secs_f64();
        let hops = (now.saturating_sub(self.start).as_secs_f64() / period).ceil();
        self.end = Some(self.start + dock::BOUNCE_PERIOD.mul_f64(hops));
    }

    /// Told to stop, or still bouncing.
    pub fn stopping(&self) -> bool {
        self.end.is_some()
    }

    /// Landed for good at `now`.
    pub fn is_done(&self, now: Duration) -> bool {
        self.end.is_some_and(|end| now >= end)
    }

    /// Stops it once `BOUNCE_TIMEOUT` passed without a window.
    pub fn time_out(&mut self, now: Duration) {
        if now.saturating_sub(self.start) >= dock::BOUNCE_TIMEOUT {
            self.stop(now);
        }
    }
}

/// An application started from the dock, bouncing until its first window comes.
#[derive(Debug, Clone)]
pub struct Launch {
    pub target: Target,
    pub bounce: Bounce,
    /// The number of windows of the application when it started: one more is its window.
    /// `None` while the source of windows said nothing yet.
    pub before: Option<usize>,
    /// The most recent `recency` of its windows when it started: an application with a single
    /// instance answers by bringing a window it already had forward, which then gets a higher
    /// one.
    pub recent: Option<u64>,
}

impl Launch {
    /// Started at `now`, while the source counts `before` windows of the application, the
    /// most recent of which has the recency `recent`.
    pub fn new(target: Target, now: Duration, before: Option<(usize, u64)>) -> Self {
        Self {
            target,
            bounce: Bounce::new(now),
            before: before.map(|(count, _)| count),
            recent: before.map(|(_, recent)| recent),
        }
    }

    /// The source now counts `count` windows of the application, the most recent with the
    /// recency `recent`. Stops the bounce at `now` when one more than at the start came, or
    /// when one of its windows got the keyboard since; returns whether it did.
    pub fn windows(&mut self, count: usize, recent: u64, now: Duration) -> bool {
        let raised = self.recent.is_some_and(|r| recent > r);
        match self.before {
            Some(before) if count > before || raised => {
                self.bounce.stop(now);
                return true;
            }
            // A window closed meanwhile: the next one to come is still new.
            Some(before) => self.before = Some(before.min(count)),
            None => {
                self.before = Some(count);
                self.recent = Some(recent);
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn an_arriving_icon_opens_its_room_and_fades_in() {
        let track = Track::arriving(52.0, ms(100));
        let slot = track.slot(ms(100));
        assert_eq!((slot.room, track.opacity.value(ms(100))), (0.0, 0.0));
        assert_eq!(track.drawn_scale(ms(100)), 0.0);
        // Half way in less than the fade; past 1 a little (the spring of a window opening),
        // then in place.
        let rooms: Vec<f32> = (100..700).map(|t| track.slot(ms(t)).room).collect();
        let half = rooms.iter().position(|r| *r >= 0.5).unwrap_or(usize::MAX);
        assert!(half < 100, "half open after {half} ms");
        let peak = rooms.iter().copied().fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.04, "peak {peak}");
        assert_eq!(track.opacity.value(ms(100) + dock::ICON_FADE), 1.0);
        assert!(!track.is_done(ms(150)));
        assert!(track.is_done(ms(700)));
        assert_eq!(track.slot(ms(700)).side, 52.0);
    }

    #[test]
    fn a_leaving_icon_closes_its_room_and_is_gone() {
        let mut track = Track::settled(52.0);
        track.show(ms(0), false);
        assert_eq!(track.drawn_scale(ms(0)), 1.0);
        assert!(!track.is_gone(ms(50)));
        assert!(track.drawn_scale(ms(80)) < 0.5 && track.slot(ms(80)).room < 0.5);
        assert_eq!(track.opacity.value(ms(0) + dock::ICON_FADE), 0.0);
        // The spring passes 0 a little: neither the scale nor the room go below.
        assert!((0..600).all(|t| track.drawn_scale(ms(t)) >= 0.0 && track.slot(ms(t)).room >= 0.0));
        assert!(track.is_gone(ms(600)));
        // Shown again on the way: it comes back from where it is.
        let mut back = Track::settled(52.0);
        back.show(ms(0), false);
        let midway = back.drawn_scale(ms(40));
        back.show(ms(40), true);
        assert_eq!(back.drawn_scale(ms(40)), midway);
        assert!(back.is_done(ms(800)) && !back.is_gone(ms(800)));
    }

    #[test]
    fn a_dragged_out_icon_closes_its_room_and_keeps_its_size() {
        let mut track = Track::settled(52.0);
        track.open(ms(0), false);
        assert!(track.slot(ms(600)).room < 0.01);
        assert_eq!(track.drawn_scale(ms(600)), 1.0);
        assert!(!track.is_gone(ms(600)), "still dragged");
        // Let go out of the dock: it shrinks and fades where it is.
        track.vanish(ms(600), dock::DRAG_OUT_OPACITY);
        assert_eq!(track.opacity.value(ms(600)), dock::DRAG_OUT_OPACITY);
        assert!(track.drawn_scale(ms(680)) < 0.5);
        assert!(track.is_gone(ms(1200)));
    }

    #[test]
    fn a_moved_icon_starts_where_it_was_and_springs_to_its_place() {
        let mut track = Track::settled(52.0);
        track.moved(ms(10), (-60.0, 30.0), (0.0, 0.0));
        assert_eq!(track.slot(ms(10)).shift, (-60.0, 30.0));
        // 90 % of the way within 150 ms, past it a little, settled to half a pixel soon after.
        let shift = |t| track.slot(ms(t)).shift.0;
        assert!(shift(160).abs() < 6.0, "{}", shift(160));
        let overshoot = (10..600).map(shift).fold(0.0, f32::max);
        assert!(overshoot > 0.0 && overshoot < 2.5, "overshoot {overshoot}");
        assert!(!track.is_done(ms(200)) && track.is_done(ms(500)));
    }

    #[test]
    fn the_bounce_hops_and_lands_at_the_end_of_a_hop() {
        let mut bounce = Bounce::new(ms(1000));
        let period = dock::BOUNCE_PERIOD;
        // On its place at the start of each hop, highest in its middle.
        assert_eq!(bounce.lift(ms(1000), 52.0), 0.0);
        let top = bounce.lift(ms(1000) + period / 2, 52.0);
        assert!((top - 26.0).abs() < 1e-3, "{top}");
        assert!(bounce.lift(ms(1000) + period / 4, 52.0) < top);
        assert!((bounce.lift(ms(1000) + period + period / 2, 52.0) - top).abs() < 1e-3);
        // Told to stop in the middle of its third hop: it finishes it, then stays down.
        let stop = ms(1000) + period * 2 + period / 2;
        bounce.stop(stop);
        assert!(bounce.stopping() && !bounce.is_done(stop));
        assert!(bounce.lift(stop + period / 4, 52.0) > 0.0);
        let end = ms(1000) + period * 3;
        assert!(bounce.is_done(end));
        assert_eq!(bounce.lift(end, 52.0), 0.0);
        assert_eq!(bounce.lift(end + period / 2, 52.0), 0.0);
    }

    #[test]
    fn the_bounce_gives_up_after_its_timeout() {
        let mut bounce = Bounce::new(ms(0));
        bounce.time_out(dock::BOUNCE_TIMEOUT - ms(1));
        assert!(!bounce.stopping());
        bounce.time_out(dock::BOUNCE_TIMEOUT);
        assert!(bounce.stopping());
        assert!(bounce.is_done(dock::BOUNCE_TIMEOUT + dock::BOUNCE_PERIOD));
    }

    #[test]
    fn a_launch_ends_with_one_more_window() {
        // The source said nothing yet: its first count is the start.
        let mut launch = Launch::new(Target::App(String::from("a")), ms(0), None);
        assert!(
            !launch.windows(1, 3, ms(10)),
            "the first count is the start"
        );
        assert!(!launch.windows(1, 3, ms(20)));
        assert!(!launch.windows(0, 3, ms(30)), "one closed");
        assert!(launch.windows(1, 3, ms(40)), "the new one");
        assert!(launch.bounce.stopping());
    }

    #[test]
    fn a_single_instance_ends_the_bounce_by_raising_its_window() {
        // Started again with two windows, the most recent of recency 4: no new window comes,
        // the application brings one of its windows forward instead.
        let mut launch = Launch::new(Target::App(String::from("a")), ms(0), Some((2, 4)));
        assert!(!launch.windows(2, 4, ms(10)));
        assert!(launch.windows(2, 5, ms(20)), "a window got the keyboard");
    }

    #[test]
    fn the_poof_spreads_and_fades() {
        let poof = Poof {
            at: (100.0, 50.0),
            side: 52.0,
            start: ms(1000),
        };
        let (first, opacity) = poof.puffs(ms(1000)).expect("puffs");
        assert_eq!((first.len(), opacity), (dock::POOF_PUFFS, 1.0));
        let (later, faded) = poof.puffs(ms(1300)).expect("puffs");
        assert!(later[0].1 > first[0].1 && faded < 0.5, "bigger, fainter");
        assert!(poof.puffs(ms(1000) + dock::POOF_DURATION).is_none());
        assert!(poof.is_done(ms(1400)) && !poof.is_done(ms(1399)));
    }

    #[test]
    fn the_dot_fades_in_and_out() {
        let mut track = Track::settled(52.0);
        track.show_dot(ms(0), true, true, true);
        assert_eq!(track.dot.value(ms(0)), 0.0);
        assert!(track.dot_active);
        assert_eq!(track.dot.value(dock::DOT_FADE), 1.0);
        assert!(!track.is_done(ms(10)) && track.is_done(dock::DOT_FADE));
        // The last window closes: it fades out, green until gone.
        track.show_dot(ms(500), false, false, true);
        assert!(track.dot_active);
        assert!(track.dot.value(ms(520)) > 0.0);
        assert_eq!(track.dot.value(ms(500) + dock::DOT_FADE), 0.0);
        // At startup, in place at once.
        let mut still = Track::settled(52.0);
        still.show_dot(ms(0), true, false, false);
        assert_eq!(still.dot.value(ms(0)), 1.0);
    }
}
