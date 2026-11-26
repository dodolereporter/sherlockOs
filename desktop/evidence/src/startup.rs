//! Timing of the start (lot E-01): each step logs when it ends, counted from the start of the
//! process, so that a slow start says which step held it (`RUST_LOG=evidence=info`). Evidence
//! once took more than 10 s to show its window in the author's session.

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
        took_ms = took.as_millis(),
        at_ms = elapsed().as_millis(),
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
        tracing::info!(step, at_ms = elapsed().as_millis(), "start");
    }
}
