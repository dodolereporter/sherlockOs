//! Where the time of the event loop goes (lot P-01). Each loop iteration is cut into the time
//! spent waiting for an event and the time spent in each part of the work: reading the clients,
//! input, building the scene, drawing it, the screens, the shell, the cleanup after each
//! iteration. The totals go to the log every `SUMMARY_EVERY` at the `debug` level, and for the
//! whole session when it ends; nothing is written per frame. An iteration that keeps the loop
//! busy for more than `STALL` is logged at once, with its parts, at most once a second: that is
//! what makes libinput say "your system is too slow". See "Measuring" in `docs/compositor.md`.

use std::{
    fmt,
    time::{Duration, Instant},
};

/// An iteration longer than this is a stall: a frame and a half at 60 frames per second.
pub const STALL: Duration = Duration::from_millis(25);
/// Time between two summaries in the log.
const SUMMARY_EVERY: Duration = Duration::from_secs(10);
/// Least time between two stall lines in the log.
const STALL_LOG_EVERY: Duration = Duration::from_secs(1);

/// A measured part of an iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Reading the requests of the Wayland clients, and answering them.
    Clients,
    /// Keyboard, mouse and touchpad events.
    Input,
    /// Building the elements of a frame, which uploads the client buffers that changed.
    Scene,
    /// Drawing a frame and handing it to the screen.
    Draw,
    /// Screens plugged in or out, console switches.
    Screens,
    /// Telling the shell programs what changed (`State::refresh_shell`).
    Shell,
    /// The cleanup after each iteration, and flushing the clients.
    Refresh,
}

const PHASES: usize = 7;

impl Phase {
    const ALL: [Phase; PHASES] = [
        Phase::Clients,
        Phase::Input,
        Phase::Scene,
        Phase::Draw,
        Phase::Screens,
        Phase::Shell,
        Phase::Refresh,
    ];

    fn name(self) -> &'static str {
        match self {
            Phase::Clients => "clients",
            Phase::Input => "input",
            Phase::Scene => "scene",
            Phase::Draw => "draw",
            Phase::Screens => "screens",
            Phase::Shell => "shell",
            Phase::Refresh => "refresh",
        }
    }
}

/// Count, total and longest of a measured time.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Tally {
    pub count: u32,
    pub total: Duration,
    pub max: Duration,
}

impl Tally {
    fn add(&mut self, time: Duration) {
        self.count += 1;
        self.total += time;
        self.max = self.max.max(time);
    }

    pub fn average(&self) -> Duration {
        self.total.checked_div(self.count).unwrap_or_default()
    }
}

/// Totals over a period, or over the whole session.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Totals {
    pub iterations: u32,
    /// Frames that went to a screen.
    pub frames: u32,
    /// Frames built that changed nothing on the screen, and were not shown.
    pub empty: u32,
    /// Each part, counted in the iterations where it ran.
    pub phases: [Tally; PHASES],
    /// Waiting for an event.
    pub wait: Tally,
    /// The rest of the work: the sources that are not measured (timers, XWayland).
    pub other: Tally,
    /// Each iteration, waiting aside.
    pub busy: Tally,
    /// Time covered.
    pub span: Duration,
}

impl Totals {
    pub fn phase(&self, phase: Phase) -> Tally {
        self.phases[phase as usize]
    }

    fn add(&mut self, iteration: &Iteration) {
        self.iterations += 1;
        self.span += iteration.total;
        self.wait.add(iteration.wait);
        self.busy.add(iteration.busy());
        self.other.add(iteration.other());
        for (tally, time) in self.phases.iter_mut().zip(iteration.phases) {
            if !time.is_zero() {
                tally.add(time);
            }
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl fmt::Display for Totals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let busy = if self.span.is_zero() {
            0.0
        } else {
            100.0 * self.busy.total.as_secs_f64() / self.span.as_secs_f64()
        };
        write!(
            f,
            "{} iterations over {:.1} s, busy {busy:.1} %, longest {:.2} ms; {} frames shown, \
             {} without damage",
            self.iterations,
            self.span.as_secs_f64(),
            ms(self.busy.max),
            self.frames,
            self.empty,
        )?;
        let named = Phase::ALL
            .iter()
            .map(|p| (p.name(), self.phase(*p)))
            .chain([("other", self.other), ("wait", self.wait)]);
        for (name, tally) in named.filter(|(_, t)| t.count > 0) {
            write!(
                f,
                "; {name} {} × {:.3} ms, max {:.2} ms",
                tally.count,
                ms(tally.average()),
                ms(tally.max)
            )?;
        }
        Ok(())
    }
}

/// One iteration, once it is over.
#[derive(Debug, Default, Clone, Copy)]
struct Iteration {
    total: Duration,
    wait: Duration,
    phases: [Duration; PHASES],
}

impl Iteration {
    fn busy(&self) -> Duration {
        self.total.saturating_sub(self.wait)
    }

    fn other(&self) -> Duration {
        self.busy().saturating_sub(self.phases.iter().sum())
    }
}

impl fmt::Display for Iteration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} ms", ms(self.busy()))?;
        let parts = Phase::ALL.iter().map(|p| (p.name(), self.phases[*p as usize]));
        for (name, time) in parts.chain([("other", self.other())]) {
            if !time.is_zero() {
                write!(f, ", {name} {:.1} ms", ms(time))?;
            }
        }
        Ok(())
    }
}

/// The measurements of the loop, kept in `State::perf`.
#[derive(Debug, Default)]
pub struct LoopStats {
    /// Start of the current iteration: the end of the previous one.
    started: Option<Instant>,
    /// Start of its first measured part: the wait ended there.
    first: Option<Instant>,
    phases: [Duration; PHASES],
    recent: Totals,
    session: Totals,
    summary_at: Option<Instant>,
    stall_logged_at: Option<Instant>,
    /// Stalls not logged yet, held back by `STALL_LOG_EVERY`.
    stalls: u32,
}

impl LoopStats {
    /// A measured part starts now.
    pub fn start(&mut self) -> Instant {
        let now = Instant::now();
        self.first.get_or_insert(now);
        now
    }

    /// The part `phase`, started at `since`, ends now.
    pub fn stop(&mut self, phase: Phase, since: Instant) {
        self.phases[phase as usize] += since.elapsed();
    }

    /// A frame was built: `shown` if it went to the screen, not if it changed nothing.
    pub fn frame(&mut self, shown: bool) {
        for totals in [&mut self.recent, &mut self.session] {
            if shown {
                totals.frames += 1;
            } else {
                totals.empty += 1;
            }
        }
    }

    /// The iteration ends now, the next one starts: totals, stall and summary in the log.
    pub fn end_iteration(&mut self) {
        let now = Instant::now();
        let Some(started) = self.started.replace(now) else {
            self.summary_at = Some(now);
            self.reset_iteration();
            return;
        };
        let total = now.saturating_duration_since(started);
        let wait = self.first.map_or(total, |first| {
            first.saturating_duration_since(started).min(total)
        });
        let iteration = Iteration {
            total,
            wait,
            phases: self.phases,
        };
        self.reset_iteration();
        self.recent.add(&iteration);
        self.session.add(&iteration);
        if iteration.busy() > STALL {
            self.stalls += 1;
            if self
                .stall_logged_at
                .is_none_or(|at| now.saturating_duration_since(at) >= STALL_LOG_EVERY)
            {
                tracing::warn!(
                    stalls = self.stalls,
                    "loop busy for {iteration} (input waits meanwhile)"
                );
                self.stall_logged_at = Some(now);
                self.stalls = 0;
            }
        }
        if self
            .summary_at
            .is_some_and(|at| now.saturating_duration_since(at) >= SUMMARY_EVERY)
        {
            tracing::debug!("loop: {}", self.recent);
            self.recent = Totals::default();
            self.summary_at = Some(now);
        }
    }

    fn reset_iteration(&mut self) {
        self.first = None;
        self.phases = [Duration::ZERO; PHASES];
    }

    /// Totals since the start of the session.
    pub fn session(&self) -> &Totals {
        &self.session
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn an_iteration_is_split_into_wait_and_parts() {
        let iteration = Iteration {
            total: ms(30),
            wait: ms(10),
            phases: [ms(2), ms(0), ms(5), ms(8), ms(0), ms(1), ms(1)],
        };
        assert_eq!(iteration.busy(), ms(20));
        assert_eq!(iteration.other(), ms(3));
        assert_eq!(
            iteration.to_string(),
            "20.0 ms, clients 2.0 ms, scene 5.0 ms, draw 8.0 ms, shell 1.0 ms, refresh 1.0 ms, \
             other 3.0 ms"
        );
    }

    #[test]
    fn totals_count_each_part_where_it_ran() {
        let mut totals = Totals::default();
        let mut iteration = Iteration {
            total: ms(16),
            wait: ms(14),
            phases: [Duration::ZERO; PHASES],
        };
        iteration.phases[Phase::Input as usize] = ms(1);
        totals.add(&iteration);
        iteration.phases[Phase::Draw as usize] = ms(1);
        totals.add(&iteration);
        assert_eq!(totals.iterations, 2);
        assert_eq!(totals.phase(Phase::Input).count, 2);
        assert_eq!(totals.phase(Phase::Draw).count, 1);
        assert_eq!(totals.phase(Phase::Scene).count, 0);
        assert_eq!(totals.busy.max, ms(2));
        assert_eq!(totals.wait.average(), ms(14));
        assert_eq!(totals.span, ms(32));
    }

    #[test]
    fn the_loop_measures_its_parts() {
        let mut stats = LoopStats::default();
        stats.end_iteration();
        let since = stats.start();
        stats.stop(Phase::Clients, since);
        stats.frame(true);
        stats.frame(false);
        stats.end_iteration();
        let session = stats.session();
        assert_eq!(session.iterations, 1);
        assert_eq!(session.phase(Phase::Clients).count, 1);
        assert_eq!((session.frames, session.empty), (1, 1));
        // The next iteration starts from scratch.
        stats.end_iteration();
        assert_eq!(stats.session().phase(Phase::Clients).count, 1);
    }
}
