//! Son de la sortie par défaut, lu par la commande `pactl` (PulseAudio ou PipeWire).

use super::{Monitor, Source, SourceError, field, run};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    /// Volume du premier canal, en pour cent. Il peut dépasser 100.
    pub volume: u16,
    pub muted: bool,
}

/// Lit les sorties de `pactl get-sink-volume @DEFAULT_SINK@` puis `get-sink-mute`, mises
/// bout à bout.
pub fn parse(output: &str) -> Result<Sound, SourceError> {
    let error = |detail: &str| SourceError::Parse {
        command: "pactl",
        detail: detail.to_owned(),
    };
    // « Volume: front-left: 36045 /  55% / -15.58 dB,   front-right: … »
    let volume = output
        .lines()
        .find(|line| line.starts_with("Volume:"))
        .and_then(|line| line.split('/').nth(1))
        .and_then(|part| part.trim().trim_end_matches('%').parse().ok())
        .ok_or_else(|| error("volume absent"))?;
    let muted = match field(output, "Mute") {
        Some("yes") => true,
        Some("no") => false,
        _ => return Err(error("sourdine absente")),
    };
    Ok(Sound { volume, muted })
}

/// Le son du poste, surveillé par `pactl subscribe`.
pub struct Pactl {
    monitor: Monitor,
}

impl Default for Pactl {
    fn default() -> Self {
        Self {
            // Seuls les changements d'une sortie ou du serveur (sortie par défaut) comptent :
            // pas ceux des flux des applications.
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
    fn son_actif() {
        // Enregistrée sur le portable de référence.
        let sound = parse(include_str!("../../sorties/pactl-55.txt")).unwrap();
        assert_eq!(
            sound,
            Sound {
                volume: 55,
                muted: false
            }
        );
    }

    #[test]
    fn son_coupe() {
        let sound = parse(include_str!("../../sorties/pactl-coupe.txt")).unwrap();
        assert!(sound.muted);
    }

    #[test]
    fn une_sortie_incomplete_est_refusee() {
        assert!(parse("Mute: no\n").is_err());
        assert!(parse("Volume: front-left: 36045 /  55% / -15.58 dB\n").is_err());
    }
}
