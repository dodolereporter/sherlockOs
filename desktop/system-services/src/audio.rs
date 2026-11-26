//! Sound of the default output and of the default microphone through `pactl`, which PipeWire
//! answers through its PulseAudio server, or else through `wpctl`, WirePlumber's own command:
//! PipeWire has no D-Bus interface. The quick settings of the bar read and set the output
//! (`Pactl`); the media keys step it and mute it, and mute the microphone (`Mixer`).

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::{Problem, Result};

/// Longest wait for `pactl` or `wpctl`: past it, the command is killed and the sound reads as
/// unavailable.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// How often a running command is looked at.
const POLL: Duration = Duration::from_millis(10);
/// Step of the volume keys, in percent.
pub const STEP: u16 = 5;
/// Highest volume the keys give: louder distorts.
pub const MAX: u16 = 100;

/// Volume and mute switch of a sink or a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    /// Volume of the first channel, in percent. It can exceed 100.
    pub volume: u16,
    pub muted: bool,
}

/// Parses the outputs of `pactl get-sink-volume @DEFAULT_SINK@` then `get-sink-mute`, joined
/// (or the same for a source).
pub fn parse(output: &str) -> Result<Sound> {
    let error = |detail: &str| Problem::Failed(format!("unreadable output from pactl: {detail}"));
    // "Volume: front-left: 36045 /  55% / -15.58 dB,   front-right: …"
    let volume = output
        .lines()
        .find(|line| line.starts_with("Volume:"))
        .and_then(|line| line.split('/').nth(1))
        .and_then(|part| part.trim().trim_end_matches('%').parse().ok())
        .ok_or_else(|| error("missing volume"))?;
    let mute = output
        .lines()
        .find_map(|line| line.strip_prefix("Mute:"))
        .map(str::trim);
    let muted = match mute {
        Some("yes") => true,
        Some("no") => false,
        _ => return Err(error("missing mute state")),
    };
    Ok(Sound { volume, muted })
}

/// Parses the output of `wpctl get-volume`: `Volume: 0.55`, with ` [MUTED]` when muted.
pub fn parse_wpctl(output: &str) -> Result<Sound> {
    let line = output.trim();
    let value = line
        .strip_prefix("Volume:")
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|v| v.parse::<f64>().ok())
        .ok_or_else(|| Problem::Failed(format!("unreadable output from wpctl: {line}")))?;
    Ok(Sound {
        volume: (value * 100.0).round().clamp(0.0, f64::from(u16::MAX)) as u16,
        muted: line.contains("[MUTED]"),
    })
}

/// The volume one key press away from `volume`: the next multiple of `STEP` up (at most `MAX`,
/// and never below a volume already louder) or down (at least 0).
pub fn step(volume: u16, up: bool) -> u16 {
    if up {
        let next = (volume / STEP + 1) * STEP;
        next.min(MAX).max(volume)
    } else {
        volume.saturating_sub(1) / STEP * STEP
    }
}

/// Reads `pipe` to its end on a thread of its own.
fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        bytes
    })
}

/// Runs `program` with `args` and returns its standard output, killing it past `TIMEOUT`.
fn run(program: &OsString, args: &[&str]) -> Result<String> {
    let shown = Path::new(program).file_name().unwrap_or(program).to_owned();
    let shown = shown.to_string_lossy();
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Problem::Absent(format!("{shown}: {e}")))?;
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
                    "{shown} {} gave no answer in {TIMEOUT:?}",
                    args.join(" ")
                )));
            }
            Err(e) => return Err(Problem::Failed(format!("{shown}: {e}"))),
        }
    };
    let text = |reader: Option<std::thread::JoinHandle<Vec<u8>>>| {
        let bytes = reader.and_then(|r| r.join().ok()).unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    let (out, err) = (text(stdout), text(stderr));
    if !status.success() {
        return Err(Problem::Failed(format!(
            "{shown} {}: {}",
            args.join(" "),
            err.trim()
        )));
    }
    Ok(out)
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
        run(&self.program, args)
    }

    /// The volume and the mute switch of the default output, or of the default microphone.
    pub fn sound(&self, device: Device) -> Result<Sound> {
        let (kind, name) = device.pactl();
        let volume = self.run(&[&format!("get-{kind}-volume"), name])?;
        let mute = self.run(&[&format!("get-{kind}-mute"), name])?;
        parse(&format!("{volume}{mute}"))
    }

    /// The default output, its volume and its name.
    pub fn read(&self) -> Result<Output> {
        let sound = self.sound(Device::Output)?;
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
        self.mute(Device::Output, muted)
    }

    /// Mutes or unmutes the default output or microphone.
    pub fn mute(&self, device: Device, muted: bool) -> Result<()> {
        let (kind, name) = device.pactl();
        let value = if muted { "1" } else { "0" };
        self.run(&[&format!("set-{kind}-mute"), name, value])
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

/// The default output or the default microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Device {
    Output,
    Microphone,
}

impl Device {
    fn pactl(self) -> (&'static str, &'static str) {
        match self {
            Self::Output => ("sink", "@DEFAULT_SINK@"),
            Self::Microphone => ("source", "@DEFAULT_SOURCE@"),
        }
    }

    fn wpctl(self) -> &'static str {
        match self {
            Self::Output => "@DEFAULT_AUDIO_SINK@",
            Self::Microphone => "@DEFAULT_AUDIO_SOURCE@",
        }
    }
}

/// What the media keys go through: `pactl` when it is there, `wpctl` otherwise.
#[derive(Debug, Clone)]
pub enum Mixer {
    Pactl(Pactl),
    Wpctl(OsString),
}

impl Mixer {
    /// `pactl` if `PATH` has it, else `wpctl` if it has that, else `pactl`, which will then
    /// report its absence.
    pub fn find() -> Self {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let has = |name: &str| std::env::split_paths(&path).any(|dir| dir.join(name).is_file());
        if !has("pactl") && has("wpctl") {
            return Self::Wpctl(OsString::from("wpctl"));
        }
        Self::Pactl(Pactl::default())
    }

    /// The volume and mute switch of `device`.
    pub fn sound(&self, device: Device) -> Result<Sound> {
        match self {
            Self::Pactl(pactl) => pactl.sound(device),
            Self::Wpctl(program) => parse_wpctl(&run(program, &["get-volume", device.wpctl()])?),
        }
    }

    /// Sets the volume of the default output, in percent, and unmutes it.
    pub fn set_volume(&self, percent: u16) -> Result<()> {
        match self {
            Self::Pactl(pactl) => {
                let value = format!("{percent}%");
                pactl.run(&["set-sink-volume", "@DEFAULT_SINK@", &value])?;
            }
            Self::Wpctl(program) => {
                let value = format!("{:.2}", f64::from(percent) / 100.0);
                run(program, &["set-volume", Device::Output.wpctl(), &value])?;
            }
        }
        self.mute(Device::Output, false)
    }

    pub fn mute(&self, device: Device, muted: bool) -> Result<()> {
        match self {
            Self::Pactl(pactl) => pactl.mute(device, muted),
            Self::Wpctl(program) => {
                let value = if muted { "1" } else { "0" };
                run(program, &["set-mute", device.wpctl(), value]).map(|_| ())
            }
        }
    }

    /// `steps` steps of the output volume, up when positive, down when negative, set at once;
    /// it also unmutes it. Returns the new sound.
    pub fn step_volume(&self, steps: i32) -> Result<Sound> {
        let now = self.sound(Device::Output)?;
        let volume = (0..steps.unsigned_abs()).fold(now.volume, |v, _| step(v, steps > 0));
        self.set_volume(volume)?;
        Ok(Sound {
            volume,
            muted: false,
        })
    }

    /// Mutes `device` if it plays, unmutes it otherwise. Returns the new sound.
    pub fn toggle_mute(&self, device: Device) -> Result<Sound> {
        let now = self.sound(device)?;
        self.mute(device, !now.muted)?;
        Ok(Sound {
            muted: !now.muted,
            ..now
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_on() {
        // Recorded on the reference laptop.
        let sound = parse(include_str!("../fixtures/pactl-55.txt")).unwrap();
        assert_eq!(
            sound,
            Sound {
                volume: 55,
                muted: false
            }
        );
    }

    #[test]
    fn sound_muted() {
        let sound = parse(include_str!("../fixtures/pactl-muted.txt")).unwrap();
        assert!(sound.muted);
    }

    #[test]
    fn incomplete_output_is_rejected() {
        assert!(parse("Mute: no\n").is_err());
        assert!(parse("Volume: front-left: 36045 /  55% / -15.58 dB\n").is_err());
    }

    #[test]
    fn wpctl_gives_a_fraction() {
        let sound = parse_wpctl("Volume: 0.55\n").unwrap();
        assert_eq!((sound.volume, sound.muted), (55, false));
        let sound = parse_wpctl("Volume: 0.30 [MUTED]\n").unwrap();
        assert_eq!((sound.volume, sound.muted), (30, true));
        assert!(parse_wpctl("nothing").is_err());
    }

    #[test]
    fn the_keys_step_by_five_up_to_one_hundred() {
        assert_eq!(step(55, true), 60);
        assert_eq!(step(57, true), 60);
        assert_eq!(step(57, false), 55);
        assert_eq!(step(55, false), 50);
        assert_eq!(step(0, false), 0);
        assert_eq!(step(3, false), 0);
        assert_eq!(step(98, true), 100);
        assert_eq!(step(100, true), 100);
        // Louder than the keys go, set elsewhere: up keeps it, down comes back.
        assert_eq!(step(120, true), 120);
        assert_eq!(step(120, false), 115);
    }

    #[test]
    fn the_output_is_named_by_its_description() {
        let sinks = include_str!("../fixtures/pactl-sinks.txt");
        let name = "alsa_output.pci-0000_05_00.6.analog-stereo";
        assert_eq!(description(sinks, name), "Haut-parleurs intégrés");
        let hdmi = "alsa_output.pci-0000_05_00.1.hdmi-stereo";
        assert_eq!(description(sinks, hdmi), "Sortie HDMI");
        assert_eq!(description(sinks, "unknown"), "unknown");
    }
}
