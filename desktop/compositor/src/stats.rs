//! Frame measurement (`--measure`): frame rate, render time, late frames. Times are `Duration`s
//! counted from startup, so that the computation can be tested without a clock.

use std::{fmt, time::Duration};

use sherlock_ui::motion;

/// Time between two log summaries.
const PERIOD: Duration = Duration::from_secs(1);

/// Summary of a run of frames.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Tally {
    pub frames: u32,
    /// Time from the first to the last frame.
    pub span: Duration,
    pub render_total: Duration,
    pub render_max: Duration,
    /// Longest interval between two frames.
    pub interval_max: Duration,
    /// Frames that came more than one period of the target frame rate after the previous one.
    pub late: u32,
}

impl Tally {
    fn add(&mut self, interval: Option<Duration>, render: Duration) {
        self.frames += 1;
        self.render_total += render;
        self.render_max = self.render_max.max(render);
        if let Some(interval) = interval {
            self.span += interval;
            self.interval_max = self.interval_max.max(interval);
            self.late += u32::from(interval > frame_budget());
        }
    }

    /// Frames per second over the period.
    pub fn fps(&self) -> f64 {
        let intervals = self.frames.saturating_sub(1);
        if self.span.is_zero() {
            return 0.0;
        }
        f64::from(intervals) / self.span.as_secs_f64()
    }
}

/// Longest interval between two frames that keeps the target frame rate.
pub fn frame_budget() -> Duration {
    Duration::from_secs(1) / motion::FRAME_RATE
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let average = self
            .render_total
            .checked_div(self.frames)
            .unwrap_or_default();
        write!(
            f,
            "{} frames, {:.1} frames/s, average render {:.2} ms, max render {:.2} ms, \
             max interval {:.2} ms, {} late (> {:.2} ms)",
            self.frames,
            self.fps(),
            ms(average),
            ms(self.render_max),
            ms(self.interval_max),
            self.late,
            ms(frame_budget()),
        )
    }
}

/// Statistics kept over the whole session.
#[derive(Debug, Default)]
pub struct FrameStats {
    last: Option<Duration>,
    period_start: Option<Duration>,
    period: Tally,
    /// Frames drawn during an animation, over the whole session.
    pub animated: Tally,
    /// All the frames of the session.
    pub all: Tally,
}

impl FrameStats {
    /// Records a frame finished at time `now`, drawn in `render`. Returns the summary of the
    /// elapsed period, once per period.
    pub fn record(&mut self, now: Duration, render: Duration, animating: bool) -> Option<Tally> {
        let interval = self.last.map(|last| now.saturating_sub(last));
        self.last = Some(now);
        if let Some(interval) = interval.filter(|i| *i > frame_budget()) {
            tracing::debug!(?now, ?interval, ?render, animating, "late frame");
        }
        self.all.add(interval, render);
        if animating {
            self.animated.add(interval, render);
        }
        let start = *self.period_start.get_or_insert(now);
        self.period
            .add(interval.filter(|_| self.period.frames > 0), render);
        if now.saturating_sub(start) < PERIOD {
            return None;
        }
        self.period_start = Some(now);
        Some(std::mem::take(&mut self.period))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn sixty_regular_frames_keep_the_frame_rate() {
        let mut stats = FrameStats::default();
        let step = Duration::from_micros(16_000);
        let mut report = None;
        for i in 0..=63 {
            report = report.or(stats.record(step * i, ms(2), true));
        }
        let report = report.unwrap();
        assert_eq!(report.late, 0);
        assert!((report.fps() - 62.5).abs() < 0.01, "{report}");
        assert_eq!(report.render_max, ms(2));
        assert_eq!(stats.animated.late, 0);
        assert_eq!(stats.animated.frames, 64);
    }

    #[test]
    fn a_missed_frame_is_counted_late() {
        let mut stats = FrameStats::default();
        for t in [0, 16, 32, 66, 82] {
            stats.record(ms(t), ms(1), t < 70);
        }
        assert_eq!(stats.all.late, 1);
        assert_eq!(stats.all.interval_max, ms(34));
        // The last frame, without animation, does not count among the animated frames.
        assert_eq!(stats.animated.frames, 4);
        assert_eq!(stats.animated.late, 1);
    }
}
