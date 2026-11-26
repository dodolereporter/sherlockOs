//! When the dock hides (`AutoHide`), and the spring that slides it off the screen and back.
//!
//! - `never`: always shown, with its exclusive zone.
//! - `always`: hidden, with no exclusive zone.
//! - `smart`, the default: like `never`, except while a maximized or fullscreen window is on
//!   the dock's screen. That window takes the whole screen below the bar: the dock gives up its
//!   exclusive zone and slides out.
//!
//! A hidden dock keeps a band of `dock::HIDE_STRIP` pixels on the screen, along its edge, that
//! takes the pointer: touching it brings the dock back over the windows, still without an
//! exclusive zone. It hides again `dock::HIDE_DELAY` after the pointer left it. Pure state: the
//! dock turns it into margins, an exclusive zone and an input region.

use std::time::Duration;

use dock_config::AutoHide;
use sherlock_ui::dock;
use sherlock_ui::motion::{self, Animation};

#[derive(Debug, Clone, Copy)]
pub struct Hide {
    mode: AutoHide,
    /// A maximized or fullscreen window is on the dock's screen.
    covered: bool,
    /// Brought back by the pointer while it should hide.
    revealed: bool,
    /// When the pointer left the revealed dock: it hides `HIDE_DELAY` later.
    left_at: Option<Duration>,
    /// 0 shown, 1 hidden.
    slide: Animation,
    now: Duration,
}

impl Hide {
    /// In `mode`, nothing covering the screen yet, at rest where it should be.
    pub fn new(mode: AutoHide) -> Self {
        let mut hide = Self {
            mode,
            covered: false,
            revealed: false,
            left_at: None,
            slide: Animation::new(dock::HIDE_SPRING, 0.0, 0.0, Duration::ZERO),
            now: Duration::ZERO,
        };
        let rest = hide.target();
        hide.slide = Animation::new(dock::HIDE_SPRING, rest, rest, Duration::ZERO);
        hide
    }

    /// The mode hides the dock now: `always`, or `smart` with a window covering its screen.
    pub fn hides(&self) -> bool {
        match self.mode {
            AutoHide::Never => false,
            AutoHide::Always => true,
            AutoHide::Smart => self.covered,
        }
    }

    /// The dock is hidden, or on its way out.
    pub fn hidden(&self) -> bool {
        self.hides() && !self.revealed
    }

    /// The dock reserves its room on the screen: only when its mode does not hide it. A dock
    /// brought back by the pointer goes over the windows.
    pub fn exclusive(&self) -> bool {
        !self.hides()
    }

    /// A new mode, at `now`.
    pub fn set_mode(&mut self, mode: AutoHide, now: Duration) {
        self.mode = mode;
        self.aim(now);
    }

    /// Whether a window covers the dock's screen, at `now`. A dock under the pointer stays
    /// until the pointer leaves it.
    pub fn set_covered(&mut self, covered: bool, pointer_over: bool, now: Duration) {
        if covered && !self.covered && pointer_over {
            self.revealed = true;
        }
        self.covered = covered;
        self.aim(now);
    }

    /// The pointer is on the dock, the band of a hidden dock included: it comes back.
    pub fn pointer_in(&mut self, now: Duration) {
        self.left_at = None;
        if self.hides() {
            self.revealed = true;
        }
        self.aim(now);
    }

    /// The pointer left the dock at `now`: a revealed dock hides `HIDE_DELAY` later.
    pub fn pointer_out(&mut self, now: Duration) {
        if self.revealed {
            self.left_at = Some(now);
        }
    }

    /// Time passes. `busy`: a menu is open or an icon is held, the dock stays.
    pub fn tick(&mut self, now: Duration, busy: bool) {
        self.now = self.now.max(now);
        if let Some(left) = self.left_at
            && !busy
            && self.now >= left + dock::HIDE_DELAY
        {
            self.left_at = None;
            self.revealed = false;
            self.aim(self.now);
        }
    }

    /// From shown (0) to hidden (1), at the last tick.
    pub fn progress(&self) -> f32 {
        if self
            .slide
            .is_done(self.now, motion::GEOMETRY_PRECISION / 100.0)
        {
            self.slide.target() as f32
        } else {
            self.slide.value(self.now).clamp(-0.1, 1.1) as f32
        }
    }

    /// Frames are needed: the dock slides, or waits to hide.
    pub fn running(&self) -> bool {
        self.left_at.is_some()
            || !self
                .slide
                .is_done(self.now, motion::GEOMETRY_PRECISION / 100.0)
    }

    fn target(&self) -> f64 {
        if self.hidden() { 1.0 } else { 0.0 }
    }

    fn aim(&mut self, now: Duration) {
        self.now = self.now.max(now);
        if !self.hides() {
            self.revealed = false;
            self.left_at = None;
        }
        let target = self.target();
        if (self.slide.target() - target).abs() > f64::EPSILON {
            self.slide.retarget(self.now, target);
        }
    }
}

/// Distance from the screen edge to the dock surface at `progress` from shown (0, the gap of
/// the board) to hidden (1, only `HIDE_STRIP` of a surface `depth` deep left on the screen).
pub fn margin(progress: f32, depth: f32) -> i32 {
    let shown = dock::MARGIN_BOTTOM;
    let hidden = dock::HIDE_STRIP - depth;
    (shown + (hidden - shown) * progress).round() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Lets the spring end.
    fn settle(hide: &mut Hide, from: Duration) -> Duration {
        let end = from + Duration::from_secs(2);
        hide.tick(end, false);
        assert!(!hide.running());
        end
    }

    #[test]
    fn smart_hides_while_a_window_covers_the_screen() {
        let mut hide = Hide::new(AutoHide::Smart);
        assert!(!hide.hidden() && hide.exclusive() && !hide.running());
        assert_eq!(hide.progress(), 0.0);
        // A window is maximized: no exclusive zone at once, and the dock slides out.
        hide.set_covered(true, false, ms(0));
        assert!(hide.hidden() && !hide.exclusive() && hide.running());
        let t = settle(&mut hide, ms(0));
        assert_eq!(hide.progress(), 1.0);
        // The window is restored: the dock comes back and takes its room again.
        hide.set_covered(false, false, t);
        assert!(!hide.hidden() && hide.exclusive());
        settle(&mut hide, t);
        assert_eq!(hide.progress(), 0.0);
    }

    #[test]
    fn the_edge_brings_it_back_and_it_hides_again_after_the_delay() {
        let mut hide = Hide::new(AutoHide::Smart);
        hide.set_covered(true, false, ms(0));
        let t = settle(&mut hide, ms(0));
        hide.pointer_in(t);
        assert!(!hide.hidden(), "shown over the window");
        assert!(!hide.exclusive(), "without taking its room back");
        let t = settle(&mut hide, t);
        assert_eq!(hide.progress(), 0.0);
        hide.pointer_out(t);
        hide.tick(t + dock::HIDE_DELAY - ms(1), false);
        assert!(!hide.hidden(), "not before the delay");
        // Back in time: the delay starts again on the next leave.
        hide.pointer_in(t + ms(100));
        hide.tick(t + dock::HIDE_DELAY + ms(50), false);
        assert!(!hide.hidden());
        hide.pointer_out(t + ms(200));
        // A menu keeps it.
        hide.tick(t + ms(200) + dock::HIDE_DELAY, true);
        assert!(!hide.hidden());
        hide.tick(t + ms(210) + dock::HIDE_DELAY, false);
        assert!(hide.hidden());
    }

    #[test]
    fn a_dock_under_the_pointer_stays_until_the_pointer_leaves() {
        let mut hide = Hide::new(AutoHide::Smart);
        hide.set_covered(true, true, ms(0));
        assert!(!hide.hidden() && !hide.exclusive());
        hide.pointer_out(ms(10));
        hide.tick(ms(10) + dock::HIDE_DELAY, false);
        assert!(hide.hidden());
    }

    #[test]
    fn never_and_always() {
        let mut never = Hide::new(AutoHide::Never);
        never.set_covered(true, false, ms(0));
        never.pointer_out(ms(0));
        assert!(!never.hidden() && never.exclusive() && !never.running());
        let always = Hide::new(AutoHide::Always);
        assert!(always.hidden() && !always.exclusive());
        assert_eq!(
            always.progress(),
            1.0,
            "hidden from the start, without sliding"
        );
        assert!(!always.running());
        // The pointer is not on the dock: no delay waits.
        let mut always = always;
        always.pointer_out(ms(0));
        assert!(!always.running());
    }

    #[test]
    fn the_margin_leaves_the_strip_on_the_screen() {
        assert_eq!(margin(0.0, 160.0), 8);
        assert_eq!(margin(1.0, 160.0), -158);
        assert_eq!(margin(0.5, 160.0), -75);
    }
}
