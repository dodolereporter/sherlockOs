//! The arrival of the desktop once the session opens (lot D-24): the bar slides down from the
//! top and the dock rises from the bottom, on a spring, each from its first content, the dock
//! `ARRIVAL_STAGGER` after the bar. The windows that open then fade in with their own
//! opening (lot D-34). Only within `ARRIVAL_WINDOW` after the compositor started, so that a bar
//! started again later shows at once; never on the login screen. They wait for the curtain of the
//! opening to be half gone (`curtain.rs`, lot D-24e). A key or a click ends it at once, the
//! curtain with it. Tokens: `sherlock_ui::greeter`.

use std::time::Duration;

use sherlock_ui::greeter::{ARRIVAL_SPRING, ARRIVAL_STAGGER, ARRIVAL_WINDOW};
use sherlock_ui::motion::Animation;
use smithay::utils::{Logical, Rectangle};

use crate::State;

/// The surfaces of the shell that arrive, by their layer-shell namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Slides down from above the screen.
    Bar,
    /// Rises from below the screen.
    Dock,
}

impl Part {
    pub fn of(namespace: &str) -> Option<Self> {
        match namespace {
            "bar" => Some(Self::Bar),
            "dock" => Some(Self::Dock),
            _ => None,
        }
    }

    fn delay(self) -> Duration {
        match self {
            Self::Bar => Duration::ZERO,
            Self::Dock => ARRIVAL_STAGGER,
        }
    }
}

/// When each part first showed, and whether a key or a click ended the arrival.
#[derive(Debug, Default)]
pub struct Arrival {
    shown: Vec<(Part, Duration)>,
    ended: bool,
}

/// Where a part stands at `now`, from 0 (out of the screen) to 1 (in place); it may overshoot
/// a little, on the spring. `None` once in place.
pub fn progress(part: Part, shown: Duration, now: Duration) -> Option<f64> {
    let animation = Animation::new(ARRIVAL_SPRING, 0.0, 1.0, shown + part.delay());
    let done = now.saturating_sub(shown + part.delay()) > Duration::from_millis(800);
    (!done).then(|| animation.value(now))
}

/// How far the part at `rect` (in the screen `screen`) moves at `progress`: up by the bar's
/// distance to the top of the screen plus its height, down by the dock's to the bottom.
pub fn offset(
    part: Part,
    rect: Rectangle<i32, Logical>,
    screen: Rectangle<i32, Logical>,
    progress: f64,
) -> f64 {
    let travel = match part {
        Part::Bar => -f64::from(rect.loc.y - screen.loc.y + rect.size.h),
        Part::Dock => f64::from(screen.loc.y + screen.size.h - rect.loc.y),
    };
    travel * (1.0 - progress)
}

impl State {
    /// A surface of the shell named `namespace` shows its first content.
    pub(crate) fn layer_arrives(&mut self, namespace: &str) {
        let now = self.now();
        let Some(part) = Part::of(namespace) else {
            return;
        };
        if self.greeter || now > ARRIVAL_WINDOW || self.arrival.shown.iter().any(|s| s.0 == part) {
            return;
        }
        self.arrival.shown.push((part, now));
        self.queue_redraw();
    }

    /// A key or a click: everything is in place at once.
    pub(crate) fn end_arrival(&mut self) {
        self.arrival.ended = true;
        self.end_curtain_opening();
    }

    /// Where the part named `namespace` stands at `now`; `None` once in place.
    pub fn arrival_progress(&self, namespace: &str, now: Duration) -> Option<(Part, f64)> {
        let part = Part::of(namespace)?;
        if self.arrival.ended {
            return None;
        }
        let shown = self.arrival.shown.iter().find(|s| s.0 == part)?.1;
        let shown = shown.max(self.curtain_revealed());
        Some((part, progress(part, shown, now)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_slides_down_then_the_dock_rises() {
        let at = |ms| Duration::from_millis(ms);
        let shown = at(1000);
        assert_eq!(progress(Part::Bar, shown, shown), Some(0.0));
        // The dock waits for its turn.
        assert_eq!(progress(Part::Dock, shown, shown + at(30)), Some(0.0));
        let half = progress(Part::Bar, shown, shown + at(120)).unwrap();
        assert!(half > 0.3 && half < 1.05, "{half}");
        assert_eq!(progress(Part::Bar, shown, shown + at(900)), None);
        let screen = Rectangle::new((0, 0).into(), (1920, 1080).into());
        let bar = Rectangle::new((12, 10).into(), (1896, 36).into());
        let dock = Rectangle::new((600, 980).into(), (720, 100).into());
        assert_eq!(offset(Part::Bar, bar, screen, 0.0), -46.0);
        assert_eq!(offset(Part::Dock, dock, screen, 0.0), 100.0);
        assert_eq!(offset(Part::Dock, dock, screen, 1.0), 0.0);
    }
}
