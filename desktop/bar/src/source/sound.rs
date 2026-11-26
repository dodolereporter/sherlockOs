//! Sound of the default output, read with the `pactl` command (PulseAudio or PipeWire).

use super::{Monitor, Source, SourceError};

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
            monitor: Monitor::new(
                "pactl",
                &["subscribe"],
                system_services::watch::sound_changed,
            ),
        }
    }
}

impl Source for Pactl {
    type State = Sound;

    /// Read through `system_services`, each call killed past its time limit: a sound server
    /// that hangs never holds the icon (lot D-35d).
    fn read(&mut self) -> Result<Sound, SourceError> {
        let pactl = system_services::audio::Pactl::default();
        pactl
            .sound(system_services::audio::Device::Output)
            .map_err(|e| SourceError::Parse {
                command: "pactl",
                detail: e.to_string(),
            })
    }

    fn wait(&mut self) -> bool {
        self.monitor.wait()
    }
}
