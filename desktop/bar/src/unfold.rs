//! A panel that unfolds from the bar and folds back, or a page that comes into a panel: a
//! spring from 0 (folded) to 1 (in place), and a fade. The bar gives their values to
//! `sherlock_ui::animated::Reveal` at each frame, and asks for frames only while they run.

use std::time::Duration;

use iced::Element;
use sherlock_ui::animated::reveal;
use sherlock_ui::motion::{Animation, Fade, SCALE_PRECISION};
use sherlock_ui::{color, panels};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unfold {
    spring: Animation,
    fade: Fade,
    now: Duration,
}

impl Default for Unfold {
    /// In place, at rest: what is shown without an animation.
    fn default() -> Self {
        Self {
            spring: Animation::new(panels::OPEN_SPRING, 1.0, 1.0, Duration::ZERO),
            fade: Fade::new(1.0, 1.0, Duration::ZERO, Duration::ZERO),
            now: Duration::ZERO,
        }
    }
}

impl Unfold {
    /// Folded at `now`, unfolding.
    pub fn opening(now: Duration) -> Self {
        Self {
            spring: Animation::new(panels::OPEN_SPRING, 0.0, 1.0, now),
            fade: Fade::new(0.0, 1.0, now, panels::OPEN_FADE),
            now,
        }
    }

    /// Heads for in place (`true`) or folded from `now`, from where it is.
    pub fn fold(&mut self, now: Duration, open: bool) {
        self.now = now;
        let target = if open { 1.0 } else { 0.0 };
        if self.spring.target() != target {
            self.spring.retarget(now, target);
        }
        if self.fade.target() != target as f32 {
            self.fade.retarget(now, target as f32, panels::OPEN_FADE);
        }
    }

    pub fn tick(&mut self, now: Duration) {
        self.now = now;
    }

    /// Heading for folded.
    pub fn folding(&self) -> bool {
        self.fade.target() == 0.0
    }

    /// Folded and faded out: the panel can go.
    pub fn folded(&self) -> bool {
        self.folding() && self.fade.is_done(self.now)
    }

    /// Frames are needed. Folding, only until it has faded out.
    pub fn running(&self) -> bool {
        let moving = !self.spring.is_done(self.now, SCALE_PRECISION);
        !self.fade.is_done(self.now) || (moving && !self.folding())
    }

    /// From 0 folded to 1 in place, a little beyond on the spring's overshoot.
    pub fn progress(&self) -> f32 {
        self.spring.value(self.now) as f32
    }

    pub fn opacity(&self) -> f32 {
        self.fade.value(self.now).clamp(0.0, 1.0)
    }

    /// A panel at this instant: faded, a little smaller and higher, around its top right
    /// corner, under the bar's end. `content` must be built inside `color::with_opacity` of
    /// `opacity()` for the colors it takes when it is built.
    pub fn panel<'a, Message: 'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
    ) -> Element<'a, Message> {
        let folded = 1.0 - self.progress();
        reveal(content)
            .opacity(self.opacity())
            .offset(0.0, -folded * panels::OPEN_RISE)
            .scale(1.0 - folded * (1.0 - panels::OPEN_SCALE), (1.0, 0.0))
            .into()
    }

    /// A page at this instant, built by `page` faded: it slides in from the right
    /// (`forward`) or from the left.
    pub fn page<'a, Message: 'a>(
        &self,
        forward: bool,
        page: impl FnOnce() -> Element<'a, Message>,
    ) -> Element<'a, Message> {
        let side = if forward { 1.0 } else { -1.0 };
        let offset = side * (1.0 - self.progress()) * panels::PAGE_SLIDE;
        let content = color::with_opacity(self.opacity(), page);
        reveal(content)
            .opacity(self.opacity())
            .offset(offset, 0.0)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_panel_unfolds_then_rests() {
        let mut unfold = Unfold::opening(ms(100));
        assert_eq!((unfold.progress(), unfold.opacity()), (0.0, 0.0));
        assert!(unfold.running());
        unfold.tick(ms(150));
        let (progress, opacity) = (unfold.progress(), unfold.opacity());
        assert!(progress > 0.2 && progress < 1.0, "{progress}");
        assert!(opacity > 0.2 && opacity < 1.0, "{opacity}");
        unfold.tick(ms(800));
        assert!(!unfold.running(), "no frame at rest");
        assert_eq!(unfold.opacity(), 1.0);
        assert!((unfold.progress() - 1.0).abs() < 1e-3);
        assert!(!Unfold::default().running());
    }

    #[test]
    fn a_panel_folds_from_where_it_is_and_can_come_back() {
        let mut unfold = Unfold::opening(Duration::ZERO);
        unfold.tick(ms(60));
        let halfway = unfold.opacity();
        unfold.fold(ms(60), false);
        assert!(unfold.folding() && !unfold.folded());
        assert_eq!(unfold.opacity(), halfway, "nothing jumps");
        unfold.tick(ms(90));
        assert!(unfold.opacity() < halfway);
        // Clicked again while folding: it unfolds from there.
        unfold.fold(ms(90), true);
        assert!(!unfold.folding());
        unfold.tick(ms(1000));
        assert!(!unfold.running() && unfold.opacity() == 1.0);
        unfold.fold(ms(1000), false);
        unfold.tick(ms(1000) + panels::OPEN_FADE);
        assert!(unfold.folded() && !unfold.running());
    }
}
