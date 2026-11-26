//! When a screen of the real session draws (lot P-01): only when something changed since its
//! last frame, or while an animation runs, and never while its previous frame waits for the
//! vertical blank. At rest nothing is drawn and the loop sleeps until an event comes.
//!
//! "Something changed" is counted by `State::redraw_epoch`: every client request, input event,
//! screen change or new wallpaper moves it on (`State::queue_redraw`). A frame built at an
//! epoch is up to date until the next one. See "Frame pacing" in `docs/compositor.md`.

/// Where a screen is in its frame cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// Ready to draw.
    #[default]
    Idle,
    /// A frame went to the screen and waits for its vertical blank.
    Flip,
    /// The last frame changed nothing on the screen, or failed: the next one waits for the
    /// estimated vertical blank, so that clients are paced as if a frame had been shown.
    Estimate,
}

/// The frame cycle of one screen.
#[derive(Debug, Default)]
pub struct Pacing {
    phase: Phase,
    /// Epoch of the last frame built, `None` before the first one or after a failure.
    drawn: Option<u64>,
    /// The last frame was part of an animation: the next one is due anyway.
    animating: bool,
}

impl Pacing {
    /// A frame should be built now, the state being at `epoch`.
    pub fn wants_frame(&self, epoch: u64) -> bool {
        self.phase == Phase::Idle && (self.animating || self.drawn != Some(epoch))
    }

    /// A frame was built at `epoch`. `shown`: it went to the screen (it had damage).
    pub fn drawn(&mut self, epoch: u64, shown: bool, animating: bool) {
        self.drawn = Some(epoch);
        self.animating = animating;
        self.phase = if shown { Phase::Flip } else { Phase::Estimate };
    }

    /// The frame could not be drawn or shown: tried again after the estimated blank.
    pub fn failed(&mut self) {
        self.drawn = None;
        self.phase = Phase::Estimate;
    }

    /// The vertical blank came, or its estimate.
    pub fn presented(&mut self) {
        self.phase = Phase::Idle;
    }

    /// The screen was reset (console switch): the next frame is due whatever the epoch.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_drawn_at_rest() {
        let mut pacing = Pacing::default();
        // The first frame is due.
        assert!(pacing.wants_frame(0));
        pacing.drawn(0, true, false);
        // Waiting for the blank: never a second frame, whatever happens.
        assert!(!pacing.wants_frame(1));
        pacing.presented();
        // Nothing changed since: nothing to draw, for as long as it lasts.
        assert!(!pacing.wants_frame(0));
        // A client draws: one frame.
        assert!(pacing.wants_frame(1));
    }

    #[test]
    fn an_animation_draws_at_each_blank_until_it_ends() {
        let mut pacing = Pacing::default();
        pacing.drawn(4, true, true);
        assert_eq!(pacing.phase(), Phase::Flip);
        assert!(!pacing.wants_frame(4));
        pacing.presented();
        assert!(pacing.wants_frame(4));
        // Its last frame: then rest.
        pacing.drawn(4, true, false);
        pacing.presented();
        assert!(!pacing.wants_frame(4));
    }

    #[test]
    fn a_frame_without_damage_waits_for_the_estimated_blank() {
        let mut pacing = Pacing::default();
        pacing.drawn(2, false, false);
        assert_eq!(pacing.phase(), Phase::Estimate);
        // A change meanwhile waits for the estimate, as it would wait for a real blank.
        assert!(!pacing.wants_frame(3));
        pacing.presented();
        assert!(pacing.wants_frame(3));
    }

    #[test]
    fn a_failed_or_reset_screen_draws_again() {
        let mut pacing = Pacing::default();
        pacing.drawn(7, true, false);
        pacing.presented();
        pacing.failed();
        assert!(!pacing.wants_frame(7));
        pacing.presented();
        assert!(pacing.wants_frame(7));
        pacing.drawn(7, true, false);
        pacing.reset();
        assert!(pacing.wants_frame(7));
    }
}
