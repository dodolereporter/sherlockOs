//! Smoothness measurement, started by `evidence --measure <folder>`: open the folder, then
//! three seconds of continuous scrolling, then three seconds of a selection moving down one
//! row every few frames. Each phase logs its summary (`tracing`), then the application quits.
//! The folder is never modified.

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Duration of each measured phase.
const PHASE: Duration = Duration::from_secs(3);
/// Scroll distance per frame, in pixels.
pub const SCROLL_STEP: f32 = 24.0;
/// Frames between two selection steps.
pub const SELECT_EVERY: usize = 9;
/// A frame beyond this interval misses 60 frames per second.
const LATE: Duration = Duration::from_micros(16_667);

/// What the application must do on this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Open,
    Wait,
    Scroll,
    Select,
    Idle,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Start,
    Opening,
    /// The folder is loaded: the next frame proves it has been drawn.
    Drawn,
    Scrolling,
    Selecting,
    Done,
}

/// Measurement state.
#[derive(Debug, Clone)]
pub struct Probe {
    pub target: PathBuf,
    phase: Phase,
    since: Option<Instant>,
    last: Option<Instant>,
    intervals: Vec<Duration>,
}

impl Probe {
    pub fn new(target: PathBuf) -> Self {
        Self {
            target,
            phase: Phase::Start,
            since: None,
            last: None,
            intervals: Vec::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.phase != Phase::Done
    }

    /// A frame was just requested. `loaded`: the measured folder is displayed.
    pub fn frame(&mut self, now: Instant, loaded: bool) -> Step {
        if let Some(last) = self.last.replace(now) {
            self.intervals.push(now - last);
        }
        let since = *self.since.get_or_insert(now);
        match self.phase {
            Phase::Start => {
                self.phase = Phase::Opening;
                self.since = Some(now);
                Step::Open
            }
            Phase::Opening if loaded => {
                self.phase = Phase::Drawn;
                Step::Wait
            }
            Phase::Opening => Step::Wait,
            Phase::Drawn => {
                tracing::info!(
                    "measure: folder opened in {:.1} ms",
                    (now - since).as_secs_f64() * 1000.0
                );
                self.next(Phase::Scrolling, now);
                Step::Scroll
            }
            Phase::Scrolling if now - since >= PHASE => {
                self.report("scroll", now - since);
                self.next(Phase::Selecting, now);
                Step::Idle
            }
            Phase::Scrolling => Step::Scroll,
            Phase::Selecting if now - since >= PHASE => {
                self.report("animated selection", now - since);
                self.phase = Phase::Done;
                Step::Exit
            }
            Phase::Selecting if self.intervals.len().is_multiple_of(SELECT_EVERY) => Step::Select,
            Phase::Selecting => Step::Idle,
            Phase::Done => Step::Exit,
        }
    }

    fn next(&mut self, phase: Phase, now: Instant) {
        self.phase = phase;
        self.since = Some(now);
        self.intervals.clear();
    }

    fn report(&self, what: &str, elapsed: Duration) {
        let mut sorted = self.intervals.clone();
        sorted.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let count = sorted.len();
        let p95 = sorted.get(count * 95 / 100).copied().unwrap_or_default();
        let max = sorted.last().copied().unwrap_or_default();
        let late = sorted.iter().filter(|d| **d > LATE).count();
        tracing::info!(
            "measure: {what}: {count} frames in {:.2} s, {:.1} frames/s, p95 interval {:.1} ms, max {:.1} ms, {late} over 16.7 ms",
            elapsed.as_secs_f64(),
            count as f64 / elapsed.as_secs_f64(),
            ms(p95),
            ms(max),
        );
    }
}
