//! Double click recognised by the application rather than by a widget. A widget keeps its
//! last press in the widget tree, which iced rebuilds when the view changes shape: a first
//! click that selects an item can change the view and drop that press (lot Q-13). This
//! tracker lives in the state of the application and outlives any view.

use std::time::Duration;

use crate::decorations::DOUBLE_CLICK;

/// How far, in logical pixels, the second press may land from the first.
pub const DOUBLE_CLICK_DISTANCE: f32 = 6.0;

/// The last press: on what, where and when.
#[derive(Debug, Clone, PartialEq)]
struct Press<T> {
    target: T,
    position: (f32, f32),
    at: Duration,
}

/// Recognises two presses on the same target, within [`DOUBLE_CLICK`] and
/// [`DOUBLE_CLICK_DISTANCE`] of each other.
#[derive(Debug, Clone, PartialEq)]
pub struct DoubleClick<T> {
    last: Option<Press<T>>,
}

impl<T> Default for DoubleClick<T> {
    fn default() -> Self {
        Self { last: None }
    }
}

impl<T: PartialEq> DoubleClick<T> {
    /// Records a press on `target` at `position`, `at` some time on a clock that only moves
    /// forward. Returns whether it ends a double click; a third press then starts over.
    pub fn press(&mut self, target: T, position: (f32, f32), at: Duration) -> bool {
        let double = self.last.as_ref().is_some_and(|last| {
            let (dx, dy) = (position.0 - last.position.0, position.1 - last.position.1);
            last.target == target
                && at.saturating_sub(last.at) <= DOUBLE_CLICK
                && dx.hypot(dy) <= DOUBLE_CLICK_DISTANCE
        });
        self.last = (!double).then_some(Press {
            target,
            position,
            at,
        });
        double
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn two_quick_presses_on_one_target_are_a_double_click() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(3, (10.0, 10.0), ms(1000)));
        assert!(clicks.press(3, (12.0, 11.0), ms(1300)));
    }

    #[test]
    fn slow_presses_are_two_clicks() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(3, (10.0, 10.0), ms(1000)));
        assert!(!clicks.press(3, (10.0, 10.0), ms(1000) + DOUBLE_CLICK + ms(1)));
    }

    #[test]
    fn presses_on_two_targets_are_two_clicks() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(3, (10.0, 10.0), ms(1000)));
        assert!(!clicks.press(4, (10.0, 10.0), ms(1100)));
    }

    #[test]
    fn presses_far_apart_are_two_clicks() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(3, (10.0, 10.0), ms(1000)));
        assert!(!clicks.press(3, (20.0, 10.0), ms(1100)));
    }

    #[test]
    fn a_third_press_starts_over() {
        let mut clicks = DoubleClick::default();
        assert!(!clicks.press(3, (10.0, 10.0), ms(1000)));
        assert!(clicks.press(3, (10.0, 10.0), ms(1100)));
        assert!(!clicks.press(3, (10.0, 10.0), ms(1200)));
    }
}
