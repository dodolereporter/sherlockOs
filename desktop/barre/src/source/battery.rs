//! Batterie, lue chez UPower par la commande `upower`. Le périphérique « DisplayDevice » est
//! celui qu'UPower compose pour l'affichage : toutes les batteries du portable en une.

use super::{Monitor, Source, SourceError, field, run};

const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

/// Sous ce pourcentage, une batterie qui se vide est signalée.
pub const LOW_PERCENT: u8 = 15;

/// Ce que fait la batterie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    Full,
    /// Branchée sans charger, ou état inconnu.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charge: Charge,
}

impl Battery {
    /// Faible : elle se vide et passe sous `LOW_PERCENT`.
    pub fn is_low(&self) -> bool {
        self.charge == Charge::Discharging && self.percent < LOW_PERCENT
    }
}

/// Lit la sortie de `upower -i …/DisplayDevice`. `None` : pas de batterie.
pub fn parse(output: &str) -> Result<Option<Battery>, SourceError> {
    let error = |detail: &str| SourceError::Parse {
        command: "upower",
        detail: detail.to_owned(),
    };
    if field(output, "present") != Some("yes") || field(output, "power supply") == Some("no") {
        return Ok(None);
    }
    let percent = field(output, "percentage")
        .and_then(|p| p.trim_end_matches('%').parse::<f32>().ok())
        .ok_or_else(|| error("pourcentage absent"))?;
    let charge = match field(output, "state") {
        Some("charging") => Charge::Charging,
        Some("discharging") => Charge::Discharging,
        Some("fully-charged") => Charge::Full,
        _ => Charge::Idle,
    };
    Ok(Some(Battery {
        percent: percent.round().clamp(0.0, 100.0) as u8,
        charge,
    }))
}

/// La batterie du poste, surveillée par `upower --monitor`.
pub struct UPower {
    monitor: Monitor,
}

impl Default for UPower {
    fn default() -> Self {
        Self {
            monitor: Monitor::new("upower", &["--monitor"], |line| {
                line.contains("device changed")
                    || line.contains("device added")
                    || line.contains("device removed")
            }),
        }
    }
}

impl Source for UPower {
    type State = Option<Battery>;

    fn read(&mut self) -> Result<Option<Battery>, SourceError> {
        parse(&run("upower", &["-i", DISPLAY_DEVICE])?)
    }

    fn wait(&mut self) -> bool {
        self.monitor.wait()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batterie_pleine_sur_secteur() {
        // Enregistrée sur le portable de référence, branché.
        let battery = parse(include_str!("../../sorties/upower-pleine.txt")).unwrap();
        assert_eq!(
            battery,
            Some(Battery {
                percent: 100,
                charge: Charge::Full
            })
        );
        assert!(!battery.unwrap().is_low());
    }

    #[test]
    fn batterie_faible_qui_se_vide() {
        // La même, réécrite à 12 % et en décharge.
        let battery = parse(include_str!("../../sorties/upower-faible.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(battery.percent, 12);
        assert_eq!(battery.charge, Charge::Discharging);
        assert!(battery.is_low());
    }

    #[test]
    fn sans_batterie_rien_ne_s_affiche() {
        let output = "  power supply:         no\n  unknown\n    present:             no\n";
        assert_eq!(parse(output).unwrap(), None);
        assert!(parse("  battery\n    present:  yes\n").is_err());
    }

    #[test]
    fn un_pourcentage_decimal_est_arrondi() {
        let output = "  battery\n    present: yes\n    state: charging\n    percentage: 54.6%\n";
        let battery = parse(output).unwrap().unwrap();
        assert_eq!((battery.percent, battery.charge), (55, Charge::Charging));
    }
}
