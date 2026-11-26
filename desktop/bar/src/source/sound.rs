//! Sound of the default output, read with the `pactl` command (PulseAudio or PipeWire).

use super::{Monitor, Source, SourceError, run};

pub use system_services::audio::Sound;

/// Parses the outputs of `pactl get-sink-volume @DEFAULT_SINK@` then `get-sink-mute`, joined.
pub fn parse(output: &str) -> Result<Sound, SourceError> {
    system_services::audio::parse(output).map_err(|e| SourceError::Parse {
        command: "pactl",
        detail: e.to_string(),
    })
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
