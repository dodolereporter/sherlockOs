//! When the dock hides (`AutoHide`), and the spring that slides it off the screen and back.
//!
//! - `never`: always shown, with its exclusive zone.
//! - `always`: hidden, with no exclusive zone.
//! - `smart`, the default: shown, except while a maximized, fullscreen or snapped (half,
//!   quarter) window is on the dock's screen, then it slides out. No exclusive zone either.
//!
//! As on macOS, a dock that hides by itself never reserves room (lot D-21g): the work area
//! depends on the mode alone, never on the dock sliding, being brought back or a window
//! covering it, so a maximized window is never resized by the dock.
//!
//! A hidden dock keeps a band of `dock::HIDE_STRIP` pixels on the screen, along its edge, that
//! takes the pointer: resting against it for `dock::REVEAL_DELAY` brings the dock back over
//! the windows, still without an exclusive zone, as on macOS (a pointer that only passes by the
//! edge does not). It hides again `dock::HIDE_DELAY` after the pointer left it. Pure state: the
//! dock turns it into margins, an exclusive zone and an input region.

use std::time::Duration;

use dock_config::AutoHide;
use sherlock_ui::dock;
use sherlock_ui::motion::{self, Animation};

/// Settled within half a pixel of a surface 100 deep.
const PRECISION: f64 = motion::GEOMETRY_PRECISION / 100.0;

#[derive(Debug, Clone, Copy)]
pub struct Hide {
    mode: AutoHide,
    /// A maximized, fullscreen or snapped window is on the dock's screen.
    covered: bool,
    /// Brought back by the pointer while it should hide.
    revealed: bool,
    /// When the pointer left the revealed dock: it hides `HIDE_DELAY` later.
    left_at: Option<Duration>,
    /// Since when the pointer rests on the band of the hidden dock: it comes back
    /// `REVEAL_DELAY` later.
    pushed_at: Option<Duration>,
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
            pushed_at: None,
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

    /// The dock reserves its room on the screen: only in the `never` mode, once in place (it
    /// slides when the mode changes). A dock that may hide by itself never does (lot D-21g):
    /// shown, brought back or sliding, it goes over the windows, and the work area does not
    /// change under a maximized window.
    pub fn exclusive(&self) -> bool {
        self.mode == AutoHide::Never && !self.sliding()
    }

    fn sliding(&self) -> bool {
        !self.slide.is_done(self.now, PRECISION)
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

    /// The pointer is on the dock at `now`. On the band of a hidden dock, it has to stay there
    /// `REVEAL_DELAY` before the dock comes back (`tick`).
    pub fn pointer_in(&mut self, now: Duration) {
        self.now = self.now.max(now);
        self.left_at = None;
        if self.hides() && !self.revealed {
            let since = *self.pushed_at.get_or_insert(self.now);
            if self.now >= since + dock::REVEAL_DELAY {
                self.reveal();
            }
        }
        self.aim(self.now);
    }

    fn reveal(&mut self) {
        self.pushed_at = None;
        self.revealed = true;
    }

    /// The pointer left the dock at `now`: a revealed dock hides `HIDE_DELAY` later, a hidden
    /// one stays hidden.
    pub fn pointer_out(&mut self, now: Duration) {
        self.pushed_at = None;
        if self.revealed {
            self.left_at = Some(now);
        }
    }

    /// Time passes. `busy`: a menu is open or an icon is held, the dock stays.
    pub fn tick(&mut self, now: Duration, busy: bool) {
        self.now = self.now.max(now);
        if let Some(since) = self.pushed_at
            && self.now >= since + dock::REVEAL_DELAY
        {
            self.reveal();
            self.aim(self.now);
        }
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
        if !self.sliding() {
            self.slide.target() as f32
        } else {
            self.slide.value(self.now).clamp(-0.1, 1.1) as f32
        }
    }

    /// Frames are needed: the dock slides, or waits to hide. `busy` (a menu open, an icon
    /// held): it does not wait, it stays.
    pub fn running(&self, busy: bool) -> bool {
        (self.left_at.is_some() && !busy) || self.pushed_at.is_some() || self.sliding()
    }

    /// The menu closed, or the icon was let go, at `now`: a dock the pointer left hides
    /// `HIDE_DELAY` from now.
    pub fn restart(&mut self, now: Duration) {
        self.now = self.now.max(now);
        if self.left_at.is_some() {
            self.left_at = Some(self.now);
        }
    }

    fn target(&self) -> f64 {
        if self.hidden() { 1.0 } else { 0.0 }
    }

    fn aim(&mut self, now: Duration) {
        self.now = self.now.max(now);
        if !self.hides() {
            self.revealed = false;
            self.left_at = None;
            self.pushed_at = None;
        }
        let target = self.target();
        if (self.slide.target() - target).abs() > f64::EPSILON {
            self.slide.retarget(self.now, target);
        }
    }
}

/// Distance from the screen edge to the dock surface at `progress` from shown (0: against the
/// edge, the gap of the board being inside the surface) to hidden (1, only `HIDE_STRIP` of a
/// surface `depth` deep left on the screen).
pub fn margin(progress: f32, depth: f32) -> i32 {
    let shown = 0.0;
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
        assert!(!hide.running(false));
        end
    }

    #[test]
    fn smart_hides_while_a_window_covers_the_screen() {
        let mut hide = Hide::new(AutoHide::Smart);
        assert!(!hide.hidden() && !hide.exclusive() && !hide.running(false));
        assert_eq!(hide.progress(), 0.0);
        // A window is maximized: the dock slides out.
        hide.set_covered(true, false, ms(0));
        assert!(hide.hidden() && !hide.exclusive() && hide.running(false));
        let t = settle(&mut hide, ms(0));
        assert_eq!(hide.progress(), 1.0);
        // The window is restored: the dock comes back, never taking room (lot D-21g).
        hide.set_covered(false, false, t);
        assert!(!hide.hidden() && !hide.exclusive());
        settle(&mut hide, t);
        assert_eq!(hide.progress(), 0.0);
        assert!(!hide.exclusive());
    }

    #[test]
    fn the_edge_brings_it_back_and_it_hides_again_after_the_delay() {
        let mut hide = Hide::new(AutoHide::Smart);
        hide.set_covered(true, false, ms(0));
        let t = settle(&mut hide, ms(0));
        hide.pointer_in(t);
        assert!(hide.hidden(), "not at once: the pointer may only pass by");
        assert!(hide.running(false), "frames while it waits");
        let t = t + dock::REVEAL_DELAY;
        hide.tick(t, false);
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
    fn a_menu_holds_the_dock_without_frames() {
        let mut hide = Hide::new(AutoHide::Smart);
        hide.set_covered(true, false, ms(0));
        let t = settle(&mut hide, ms(0));
        hide.pointer_in(t);
        hide.tick(t + dock::REVEAL_DELAY, false);
        let t = settle(&mut hide, t + dock::REVEAL_DELAY);
        assert!(!hide.hidden());
        // The pointer goes to the menu: the dock waits for nothing while it is open.
        hide.pointer_out(t);
        assert!(!hide.running(true), "no frame while the menu is open");
        hide.tick(t + Duration::from_secs(5), true);
        assert!(!hide.hidden());
        // It closes: the delay starts from there.
        let closed = t + Duration::from_secs(5);
        hide.restart(closed);
        assert!(hide.running(false));
        hide.tick(closed + dock::HIDE_DELAY - ms(1), false);
        assert!(!hide.hidden());
        hide.tick(closed + dock::HIDE_DELAY, false);
        assert!(hide.hidden());
    }

    #[test]
    fn a_pointer_passing_by_the_edge_does_not_bring_it_back() {
        let mut hide = Hide::new(AutoHide::Smart);
        hide.set_covered(true, false, ms(0));
        let t = settle(&mut hide, ms(0));
        hide.pointer_in(t);
        hide.tick(t + dock::REVEAL_DELAY - ms(10), false);
        hide.pointer_out(t + dock::REVEAL_DELAY - ms(5));
        hide.tick(t + Duration::from_secs(1), false);
        assert!(
            hide.hidden() && !hide.running(false),
            "it left before the delay"
        );
        // It comes back and rests there: the delay starts again, then the dock comes.
        let t = t + Duration::from_secs(1);
        hide.pointer_in(t);
        hide.pointer_in(t + ms(100));
        assert!(hide.hidden());
        hide.pointer_in(t + dock::REVEAL_DELAY);
        assert!(!hide.hidden(), "a motion past the delay brings it too");
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
        assert!(!never.hidden() && never.exclusive() && !never.running(false));
        let always = Hide::new(AutoHide::Always);
        assert!(always.hidden() && !always.exclusive());
        assert_eq!(
            always.progress(),
            1.0,
            "hidden from the start, without sliding"
        );
        assert!(!always.running(false));
        // The pointer is not on the dock: no delay waits.
        let mut always = always;
        always.pointer_out(ms(0));
        assert!(!always.running(false));
    }

    #[test]
    fn the_margin_leaves_the_strip_on_the_screen() {
        assert_eq!(margin(0.0, 160.0), 0);
        assert_eq!(margin(1.0, 160.0), -158);
        assert_eq!(margin(0.5, 160.0), -79);
    }
}
