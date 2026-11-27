//! Timing of the start and of every opening (lot P-02): each step logs when it ends, counted
//! from the start of the process or from the request that opened the surface, so that a slow
//! start says which step held it (`RUST_LOG=launcher=info`, the session's log). The applications
//! view uses it too.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();

/// The start of the process: the first call sets it.
pub fn begin() -> Instant {
    *START.get_or_init(Instant::now)
}

/// Time since the start.
pub fn elapsed() -> Duration {
    begin().elapsed()
}

/// Logs that `step` ended, and how long it took.
pub fn step(step: &'static str, took: Duration) {
    tracing::info!(
        step,
        took_ms = millis(took),
        at_ms = millis(elapsed()),
        "start"
    );
}

/// Runs `f` and logs it as `name`.
pub fn timed<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    let t0 = Instant::now();
    let result = f();
    step(name, t0.elapsed());
    result
}

/// Logs `step` the first time only, for what happens on every frame (the view).
pub fn first(flag: &AtomicBool, step: &'static str) {
    if !flag.swap(true, Ordering::Relaxed) {
        tracing::info!(step, at_ms = millis(elapsed()), "start");
    }
}

/// Milliseconds with a tenth, for the log.
pub fn millis(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 10_000.0).round() / 10.0
}

/// The intervals between the frames of one animation, logged when it ends: a frame that took
/// two intervals of the screen is a missed one.
#[derive(Debug, Default, Clone)]
pub struct Frames {
    last: Option<Instant>,
    intervals: Vec<Duration>,
    /// Time spent building the view, frame by frame.
    views: Vec<Duration>,
}

impl Frames {
    /// A frame was asked for at `at`.
    pub fn tick(&mut self, at: Instant) {
        if let Some(last) = self.last {
            self.intervals.push(at.saturating_duration_since(last));
        }
        self.last = Some(at);
    }

    /// The view of a frame took `took`.
    pub fn viewed(&mut self, took: Duration) {
        self.views.push(took);
    }

    /// Logs the animation `what` and starts again.
    pub fn report(&mut self, what: &'static str) {
        let mut intervals = std::mem::take(&mut self.intervals);
        let views = std::mem::take(&mut self.views);
        self.last = None;
        if intervals.is_empty() {
            return;
        }
        intervals.sort();
        let at = |share: f64| {
            let index = ((intervals.len() - 1) as f64 * share).round() as usize;
            millis(intervals[index])
        };
        let view = views.iter().max().copied().unwrap_or_default();
        tracing::info!(
            what,
            frames = intervals.len() + 1,
            median_ms = at(0.5),
            p95_ms = at(0.95),
            max_ms = at(1.0),
            view_max_ms = millis(view),
            "frames"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_intervals_are_counted_between_frames() {
        let mut frames = Frames::default();
        let t0 = Instant::now();
        for i in 0..4 {
            frames.tick(t0 + Duration::from_millis(16 * i));
        }
        assert_eq!(frames.intervals.len(), 3);
        frames.report("test");
        assert!(frames.intervals.is_empty() && frames.last.is_none());
        assert_eq!(millis(Duration::from_micros(16_666)), 16.7);
    }
}
