//! Sources of system state. Each one implements `Source`; `Fixed` is the fake implementation
//! used by tests. `watch` runs a source on its own thread and turns it into a stream of states
//! that only carries changes.
//!
//! Every command started here dies with the bar: the kernel sends it SIGTERM when its parent
//! goes away (`PR_SET_PDEATHSIG`), whatever the way out (normal exit, signal, panic), and
//! `shutdown` stops the monitors that are still running when `run` returns.

pub mod battery;
pub mod clock;
pub mod network;
pub mod shell;
pub mod sound;
pub mod workspaces;

use std::fmt::Debug;
use std::io::{self, BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use iced::futures::Stream;
use iced::futures::channel::mpsc as stream;
use sherlock_ui::metrics;

/// Delay before restarting a monitoring command that stopped.
const MONITOR_RETRY: Duration = Duration::from_secs(30);

/// Window in which a burst of monitor lines becomes a single refresh.
const COALESCE: Duration = Duration::from_millis(metrics::EVENT_COALESCE_MS);
/// Longest burst: past it, the refresh happens even if lines keep coming.
const COALESCE_MAX: Duration = Duration::from_millis(metrics::EVENT_COALESCE_MAX_MS);

/// Set by `shutdown`: monitors stop quietly and are not restarted.
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

/// Process ids of the monitors still running, stopped by `shutdown`.
static CHILDREN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Error from a source.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("could not start {command}: {source}")]
    Spawn {
        command: &'static str,
        source: io::Error,
    },
    #[error("{command} failed: {stderr}")]
    Failed {
        command: &'static str,
        stderr: String,
    },
    #[error("{command} gave no answer in {limit:?}")]
    Timeout {
        command: &'static str,
        limit: Duration,
    },
    #[error("unreadable output from {command}: {detail}")]
    Parse {
        command: &'static str,
        detail: String,
    },
}

/// A source of system state.
pub trait Source: Send + 'static {
    type State: Clone + PartialEq + Send + Debug + 'static;

    /// Reads the current state.
    fn read(&mut self) -> Result<Self::State, SourceError>;

    /// Waits until the state may have changed. Returns `false` once it will never change again.
    fn wait(&mut self) -> bool;
}

/// Fake source: a fixed state that never changes.
#[derive(Debug, Clone)]
pub struct Fixed<S>(pub S);

impl<S: Clone + PartialEq + Send + Debug + 'static> Source for Fixed<S> {
    type State = S;

    fn read(&mut self) -> Result<S, SourceError> {
        Ok(self.0.clone())
    }

    fn wait(&mut self) -> bool {
        false
    }
}

/// Runs `source` on a thread: read, send the state if it changed, wait, repeat. The thread
/// stops when the stream is dropped or when the source will never change again. A read error
/// is logged once, until the source reads fine again or fails differently.
pub fn watch<S: Source>(mut source: S, name: &'static str) -> impl Stream<Item = S::State> {
    let (sender, receiver) = stream::unbounded();
    let spawned = std::thread::Builder::new()
        .name(format!("source-{name}"))
        .spawn(move || {
            let mut last = None;
            let mut last_error = None;
            loop {
                match source.read() {
                    Ok(state) => {
                        last_error = None;
                        if last.as_ref() != Some(&state) {
                            if sender.unbounded_send(state.clone()).is_err() {
                                return;
                            }
                            last = Some(state);
                        }
                    }
                    Err(error) => {
                        let text = error.to_string();
                        if last_error.as_ref() != Some(&text) {
                            tracing::warn!(source = name, %error, "cannot read");
                            last_error = Some(text);
                        }
                    }
                }
                if !source.wait() {
                    return;
                }
            }
        });
    if let Err(error) = spawned {
        tracing::error!(source = name, %error, "cannot start the source thread");
    }
    receiver
}

/// A command in the C locale, for a stable output, that the kernel kills with SIGTERM when the
/// thread that started it ends, and so when the bar ends.
fn command(program: &str) -> Command {
    let mut command = Command::new(program);
    sherlock_ui::gpu::for_child(&mut command);
    command.env("LC_ALL", "C").stdin(Stdio::null());
    let parent = libc::pid_t::try_from(std::process::id()).unwrap_or(0);
    // SAFETY: the closure runs in the child between fork and exec. It only calls prctl(2) and
    // getppid(2), which are async-signal-safe, and builds its error from a raw errno without
    // allocating.
    unsafe {
        command.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl: the signal would never come.
            if libc::getppid() != parent {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    command
}

/// Longest wait for a command that reads a state (`upower -i`, `nmcli`): a service that no
/// longer answers leaves the icon as it was, and the next reading tries again.
pub const RUN_LIMIT: Duration = Duration::from_secs(10);

/// Runs `program` with `args` and returns its output; killed past `RUN_LIMIT`.
pub fn run(program: &'static str, args: &[&str]) -> Result<String, SourceError> {
    run_within(program, args, RUN_LIMIT)
}

/// Runs `program` with `args` and returns its output, killed past `limit`.
fn run_within(
    program: &'static str,
    args: &[&str],
    limit: Duration,
) -> Result<String, SourceError> {
    use std::io::Read;
    let mut child = command(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| SourceError::Spawn {
            command: program,
            source,
        })?;
    // Read on threads of their own: a full pipe never holds the command back.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(20)),
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(match result {
                    Err(source) => SourceError::Spawn {
                        command: program,
                        source,
                    },
                    _ => SourceError::Timeout {
                        command: program,
                        limit,
                    },
                });
            }
        }
    };
    let text = |reader: std::thread::JoinHandle<Vec<u8>>| {
        String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned()
    };
    let (out, err) = (text(stdout), text(stderr));
    if !status.success() {
        return Err(SourceError::Failed {
            command: program,
            stderr: err.trim().to_owned(),
        });
    }
    Ok(out)
}

/// Stops the monitors still running. Called when the bar leaves `run`.
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::SeqCst);
    // The lock is held while signalling: `Monitor::stop` removes a pid under the same lock
    // before it waits for that child, so every pid signalled here is still unreaped.
    let mut children = lock_children();
    for pid in children.drain(..) {
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // SAFETY: kill(2) touches no memory of ours. The pid is one of our children that
            // has not been waited for yet (see above), so no other process can have reused it.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }
}

fn shutting_down() -> bool {
    SHUTTING_DOWN.load(Ordering::SeqCst)
}

fn lock_children() -> MutexGuard<'static, Vec<u32>> {
    CHILDREN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A command that prints a line on every change, such as `upower --monitor`.
pub struct Monitor {
    program: &'static str,
    args: &'static [&'static str],
    /// Lines that announce a relevant change.
    relevant: fn(&str) -> bool,
    running: Option<(Child, Receiver<String>)>,
}

impl Monitor {
    pub fn new(
        program: &'static str,
        args: &'static [&'static str],
        relevant: fn(&str) -> bool,
    ) -> Self {
        Self {
            program,
            args,
            relevant,
            running: None,
        }
    }

    /// Waits for the next relevant line, then lets the burst it belongs to pass. If the command
    /// stops, restarts it after a delay. Returns `false` once the bar shuts down.
    pub fn wait(&mut self) -> bool {
        loop {
            if shutting_down() {
                self.stop();
                return false;
            }
            let Some((_, lines)) = &self.running else {
                match self.start() {
                    Ok(running) => self.running = Some(running),
                    Err(error) => {
                        tracing::warn!(command = self.program, %error, "cannot monitor");
                        std::thread::sleep(MONITOR_RETRY);
                        return true;
                    }
                }
                continue;
            };
            if coalesce(lines, self.relevant, COALESCE, COALESCE_MAX) {
                return true;
            }
            self.stop();
            if shutting_down() {
                return false;
            }
            tracing::warn!(command = self.program, "monitor stopped, restarting");
            std::thread::sleep(MONITOR_RETRY);
            return true;
        }
    }

    fn start(&self) -> io::Result<(Child, Receiver<String>)> {
        let mut child = command(self.program)
            .args(self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        lock_children().push(child.id());
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("no standard output"))?;
        // Lines are read on their own thread, so that a burst can be drained with a timeout.
        // That thread ends with the command's output.
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name(format!("lines-{}", self.program))
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if sender.send(line).is_err() {
                        return;
                    }
                }
            })?;
        Ok((child, receiver))
    }

    fn stop(&mut self) {
        if let Some((mut child, _)) = self.running.take() {
            let id = child.id();
            // Out of the list before the child is reaped: `shutdown` never signals a pid that
            // could have been reused.
            lock_children().retain(|pid| *pid != id);
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Blocks until a relevant line arrives, then drains the lines that follow it within `window`
/// of each other, for `max` at most. Returns `false` when the lines end before any relevant
/// one.
fn coalesce(
    lines: &Receiver<String>,
    relevant: fn(&str) -> bool,
    window: Duration,
    max: Duration,
) -> bool {
    loop {
        match lines.recv() {
            Ok(line) if relevant(&line) => break,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    // A pause ends the burst, and so does its length, or the end of the command, whose
    // restart comes next.
    let end = Instant::now() + max;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() || lines.recv_timeout(window.min(left)).is_err() {
            return true;
        }
    }
}

/// Value of a `key: value` line in an output, trimmed.
fn field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key).then(|| v.trim())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::StreamExt;
    use iced::futures::executor::block_on;

    #[test]
    fn a_command_that_hangs_is_stopped() {
        let start = Instant::now();
        let hung = run_within("sleep", &["30"], Duration::from_millis(200));
        assert!(matches!(hung, Err(SourceError::Timeout { .. })), "{hung:?}");
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(run("echo", &["ready"]).ok().as_deref(), Some("ready\n"));
        assert!(matches!(run("false", &[]), Err(SourceError::Failed { .. })));
    }

    #[test]
    fn a_fixed_source_yields_its_state_once() {
        let states: Vec<_> = block_on(watch(Fixed(42), "test").collect());
        assert_eq!(states, [42]);
    }

    /// A source that yields a sequence of states, then stops.
    struct Sequence(Vec<u8>);

    impl Source for Sequence {
        type State = u8;

        fn read(&mut self) -> Result<u8, SourceError> {
            self.0.first().copied().ok_or(SourceError::Parse {
                command: "test",
                detail: String::from("empty"),
            })
        }

        fn wait(&mut self) -> bool {
            self.0.remove(0);
            !self.0.is_empty()
        }
    }

    #[test]
    fn only_changes_are_sent() {
        let states: Vec<_> = block_on(watch(Sequence(vec![1, 1, 2, 2, 1]), "test").collect());
        assert_eq!(states, [1, 2, 1]);
    }

    #[test]
    fn a_burst_of_lines_is_one_refresh() {
        let (sender, lines) = mpsc::channel();
        for line in ["noise", "change", "change", "change"] {
            sender.send(String::from(line)).unwrap();
        }
        let (window, max) = (Duration::from_millis(20), Duration::from_secs(5));
        assert!(coalesce(&lines, |l| l == "change", window, max));
        // The whole burst is gone: nothing is left to trigger a second refresh.
        assert!(lines.try_recv().is_err());
        drop(sender);
        assert!(!coalesce(&lines, |l| l == "change", window, max));
    }

    #[test]
    fn an_endless_burst_still_refreshes() {
        let (sender, lines) = mpsc::channel();
        // A slider dragged for a long time: a line every 5 ms, never a pause.
        let feeder = std::thread::spawn(move || {
            while sender.send(String::from("change")).is_ok() {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let started = Instant::now();
        let (window, max) = (Duration::from_millis(50), Duration::from_millis(200));
        assert!(coalesce(&lines, |l| l == "change", window, max));
        assert!(started.elapsed() < Duration::from_secs(2));
        drop(lines);
        let _ = feeder.join();
    }

    #[test]
    fn a_field_is_read_by_its_key() {
        let output = "  state:   charging\n  percentage:  55%\n";
        assert_eq!(field(output, "state"), Some("charging"));
        assert_eq!(field(output, "percentage"), Some("55%"));
        assert_eq!(field(output, "energy"), None);
    }
}
