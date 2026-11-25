//! Sources of system state. Each one implements `Source`; `Fixed` is the fake implementation
//! used by tests. `watch` runs a source on its own thread and turns it into a stream of states
//! that only carries changes.

pub mod battery;
pub mod clock;
pub mod network;
pub mod sound;
pub mod workspaces;

use std::fmt::Debug;
use std::io::{BufRead, BufReader, Lines};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

use iced::futures::Stream;
use iced::futures::channel::mpsc;

/// Delay before restarting a monitoring command that stopped.
const MONITOR_RETRY: Duration = Duration::from_secs(30);

/// Error from a source.
#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("could not start {command}: {source}")]
    Spawn {
        command: &'static str,
        source: std::io::Error,
    },
    #[error("{command} failed: {stderr}")]
    Failed {
        command: &'static str,
        stderr: String,
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
/// stops when the stream is dropped or when the source will never change again.
pub fn watch<S: Source>(mut source: S, name: &'static str) -> impl Stream<Item = S::State> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(format!("source-{name}"))
        .spawn(move || {
            let mut last = None;
            loop {
                match source.read() {
                    Ok(state) if last.as_ref() != Some(&state) => {
                        if sender.unbounded_send(state.clone()).is_err() {
                            return;
                        }
                        last = Some(state);
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(source = name, %error, "cannot read"),
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

/// Runs `program` with `args` in the C locale, for a stable output, and returns its output.
pub fn run(program: &'static str, args: &[&str]) -> Result<String, SourceError> {
    let output = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| SourceError::Spawn {
            command: program,
            source,
        })?;
    if !output.status.success() {
        return Err(SourceError::Failed {
            command: program,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A command that prints a line on every change, such as `upower --monitor`.
pub struct Monitor {
    program: &'static str,
    args: &'static [&'static str],
    /// Lines that announce a relevant change.
    relevant: fn(&str) -> bool,
    running: Option<(Child, Lines<BufReader<ChildStdout>>)>,
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

    /// Waits for the next relevant line. If the command stops, restarts it after a delay.
    pub fn wait(&mut self) -> bool {
        loop {
            let Some((_, lines)) = &mut self.running else {
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
            match lines.next() {
                Some(Ok(line)) if (self.relevant)(&line) => return true,
                Some(Ok(_)) => {}
                Some(Err(_)) | None => {
                    self.stop();
                    tracing::warn!(command = self.program, "monitor stopped, restarting");
                    std::thread::sleep(MONITOR_RETRY);
                    return true;
                }
            }
        }
    }

    fn start(&self) -> std::io::Result<(Child, Lines<BufReader<ChildStdout>>)> {
        let mut child = Command::new(self.program)
            .args(self.args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("no standard output"))?;
        Ok((child, BufReader::new(stdout).lines()))
    }

    fn stop(&mut self) {
        if let Some((mut child, _)) = self.running.take() {
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
    fn a_field_is_read_by_its_key() {
        let output = "  state:   charging\n  percentage:  55%\n";
        assert_eq!(field(output, "state"), Some("charging"));
        assert_eq!(field(output, "percentage"), Some("55%"));
        assert_eq!(field(output, "energy"), None);
    }
}
