//! Sound of the default output through `pactl`, which PipeWire answers through its PulseAudio
//! server: PipeWire has no D-Bus interface. The volume, the mute switch and the name of the
//! output. The bar's indicator reads the same values (`source::sound`).

use std::ffi::OsString;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Problem, Result};
use crate::source::sound::{self, Sound};

/// Longest wait for `pactl`: past it, the command is killed and the sound reads as
/// unavailable.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// How often a running `pactl` is looked at.
const POLL: Duration = Duration::from_millis(10);

/// Reads `pipe` to its end on a thread of its own.
fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

/// The default output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub sound: Sound,
    /// Its description (« Haut-parleurs intégrés »), or its name without one.
    pub name: String,
}

/// `pactl`, or a stand-in for the tests.
#[derive(Debug, Clone)]
pub struct Pactl {
    pub program: OsString,
}

impl Default for Pactl {
    fn default() -> Self {
        Self {
            program: OsString::from("pactl"),
        }
    }
}

impl Pactl {
    fn run(&self, args: &[&str]) -> Result<String> {
        let mut child = Command::new(&self.program)
            .args(args)
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Problem::Absent(format!("pactl: {e}")))?;
        // Read on threads of their own: a full pipe never holds the command back.
        let stdout = child.stdout.take().map(drain);
        let stderr = child.stderr.take().map(drain);
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(POLL),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Problem::Absent(format!(
                        "pactl {} gave no answer in {TIMEOUT:?}",
                        args.join(" ")
                    )));
                }
                Err(e) => return Err(Problem::Failed(format!("pactl: {e}"))),
            }
        };
        let text = |reader: Option<std::thread::JoinHandle<Vec<u8>>>| {
            let bytes = reader.and_then(|r| r.join().ok()).unwrap_or_default();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let (out, err) = (text(stdout), text(stderr));
        if !status.success() {
            return Err(Problem::Failed(format!(
                "pactl {}: {}",
                args.join(" "),
                err.trim()
            )));
        }
        Ok(out)
    }

    /// The default output, its volume and its name.
    pub fn read(&self) -> Result<Output> {
        let volume = self.run(&["get-sink-volume", "@DEFAULT_SINK@"])?;
        let mute = self.run(&["get-sink-mute", "@DEFAULT_SINK@"])?;
        let sound =
            sound::parse(&format!("{volume}{mute}")).map_err(|e| Problem::Failed(e.to_string()))?;
        let default = self.run(&["get-default-sink"])?;
        let sinks = self.run(&["list", "sinks"])?;
        Ok(Output {
            sound,
            name: description(&sinks, default.trim()),
        })
    }

    /// Sets the volume of the default output, 0 to 100 %.
    pub fn set_volume(&self, percent: u8) -> Result<()> {
        let value = format!("{}%", percent.min(100));
        self.run(&["set-sink-volume", "@DEFAULT_SINK@", &value])
            .map(|_| ())
    }

    pub fn set_mute(&self, muted: bool) -> Result<()> {
        let value = if muted { "1" } else { "0" };
        self.run(&["set-sink-mute", "@DEFAULT_SINK@", value])
            .map(|_| ())
    }
}

/// The description of the sink `name` in the output of `pactl list sinks`; the name itself
/// when it has none.
pub fn description(sinks: &str, name: &str) -> String {
    let mut current = false;
    for line in sinks.lines().map(str::trim) {
        if let Some(found) = line.strip_prefix("Name:") {
            current = found.trim() == name;
        } else if current && let Some(text) = line.strip_prefix("Description:") {
            return text.trim().to_owned();
        }
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_output_is_named_by_its_description() {
        let sinks = include_str!("../../fixtures/pactl-sinks.txt");
        let name = "alsa_output.pci-0000_05_00.6.analog-stereo";
        assert_eq!(description(sinks, name), "Haut-parleurs intégrés");
        let hdmi = "alsa_output.pci-0000_05_00.1.hdmi-stereo";
        assert_eq!(description(sinks, hdmi), "Sortie HDMI");
        assert_eq!(description(sinks, "unknown"), "unknown");
    }
}
