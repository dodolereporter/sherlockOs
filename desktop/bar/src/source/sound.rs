//! Sound of the default output, read with the `pactl` command (PulseAudio or PipeWire).

use super::{Monitor, Source, SourceError, field, run};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    /// Volume of the first channel, in percent. It can exceed 100.
    pub volume: u16,
    pub muted: bool,
}

/// Parses the outputs of `pactl get-sink-volume @DEFAULT_SINK@` then `get-sink-mute`, joined.
pub fn parse(output: &str) -> Result<Sound, SourceError> {
    let error = |detail: &str| SourceError::Parse {
        command: "pactl",
        detail: detail.to_owned(),
    };
    // "Volume: front-left: 36045 /  55% / -15.58 dB,   front-right: …"
    let volume = output
        .lines()
        .find(|line| line.starts_with("Volume:"))
        .and_then(|line| line.split('/').nth(1))
        .and_then(|part| part.trim().trim_end_matches('%').parse().ok())
        .ok_or_else(|| error("missing volume"))?;
    let muted = match field(output, "Mute") {
        Some("yes") => true,
        Some("no") => false,
        _ => return Err(error("missing mute state")),
    };
    Ok(Sound { volume, muted })
}

/// The machine's sound, watched with `pactl subscribe`.
pub struct Pactl {
    monitor: Monitor,
}

impl Default for Pactl {
    fn default() -> Self {
        Self {
            // Only changes of an output or of the server (default output) matter, not those of
            // application streams.
            monitor: Monitor::new("pactl", &["subscribe"], |line| {
                line.contains(" on sink #") || line.contains(" on server")
            }),
        }
    }
}

impl Source for Pactl {
    type State = Sound;

    fn read(&mut self) -> Result<Sound, SourceError> {
        let volume = run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"])?;
        let mute = run("pactl", &["get-sink-mute", "@DEFAULT_SINK@"])?;
        parse(&format!("{volume}{mute}"))
    }

    fn wait(&mut self) -> bool {
        self.monitor.wait()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_on() {
        // Recorded on the reference laptop.
        let sound = parse(include_str!("../../fixtures/pactl-55.txt")).unwrap();
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
        let sound = parse(include_str!("../../fixtures/pactl-muted.txt")).unwrap();
        assert!(sound.muted);
    }

    #[test]
    fn incomplete_output_is_rejected() {
        assert!(parse("Mute: no\n").is_err());
        assert!(parse("Volume: front-left: 36045 /  55% / -15.58 dB\n").is_err());
    }
}
