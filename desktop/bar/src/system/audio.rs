//! Sound of the default output through `pactl`, which PipeWire answers through its PulseAudio
//! server: PipeWire has no D-Bus interface. The volume, the mute switch and the name of the
//! output. The bar's indicator reads the same values (`source::sound`).

use std::ffi::OsString;
use std::process::Command;

use super::{Problem, Result};
use crate::source::sound::{self, Sound};

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
        let output = Command::new(&self.program)
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .map_err(|e| Problem::Absent(format!("pactl: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(Problem::Failed(format!(
                "pactl {}: {stderr}",
                args.join(" ")
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The default output, its volume and its name.
    pub fn read(&self) -> Result<Output> {
        let volume = self.run(&["get-sink-volume", "@DEFAULT_SINK@"])?;
        let mute = self.run(&["get-sink-mute", "@DEFAULT_SINK@"])?;
        let sound = sound::parse(&format!("{volume}{mute}"))
            .map_err(|e| Problem::Failed(e.to_string()))?;
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
