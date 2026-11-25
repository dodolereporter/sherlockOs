//! Battery, read from UPower with the `upower` command. The "DisplayDevice" is the device
//! UPower composes for display: all the laptop's batteries as one.

use super::{Monitor, Source, SourceError, field, run};

const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

/// Below this percentage, a discharging battery is flagged.
pub const LOW_PERCENT: u8 = 15;

/// What the battery is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    Full,
    /// Plugged in without charging, or unknown state.
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charge: Charge,
}

impl Battery {
    /// Low: discharging and below `LOW_PERCENT`.
    pub fn is_low(&self) -> bool {
        self.charge == Charge::Discharging && self.percent < LOW_PERCENT
    }
}

/// Parses the output of `upower -i …/DisplayDevice`. `None`: no battery.
pub fn parse(output: &str) -> Result<Option<Battery>, SourceError> {
    let error = |detail: &str| SourceError::Parse {
        command: "upower",
        detail: detail.to_owned(),
    };
    match field(output, "present") {
        Some("yes") => {}
        Some(_) => return Ok(None),
        // Not even a battery section: an output we do not understand, logged by `watch`.
        None => return Err(error("no \"present\" field")),
    }
    if field(output, "power supply") == Some("no") {
        return Ok(None);
    }
    let percent = field(output, "percentage")
        .and_then(|p| p.trim_end_matches('%').parse::<f32>().ok())
        .ok_or_else(|| error("missing percentage"))?;
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

/// The machine's battery, watched with `upower --monitor`.
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
    fn full_battery_on_mains() {
        // Recorded on the reference laptop, plugged in.
        let battery = parse(include_str!("../../fixtures/upower-full.txt")).unwrap();
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
    fn low_battery_discharging() {
        // The same output, rewritten at 12 % and discharging.
        let battery = parse(include_str!("../../fixtures/upower-low.txt"))
            .unwrap()
            .unwrap();
        assert_eq!(battery.percent, 12);
        assert_eq!(battery.charge, Charge::Discharging);
        assert!(battery.is_low());
    }

    #[test]
    fn without_battery_nothing_is_shown() {
        let output = "  power supply:         no\n  unknown\n    present:             no\n";
        assert_eq!(parse(output).unwrap(), None);
        assert!(parse("  battery\n    present:  yes\n").is_err());
    }

    #[test]
    fn unexpected_output_is_an_error_not_a_missing_battery() {
        assert!(parse("").is_err());
        assert!(parse("failed to get device\n").is_err());
    }

    #[test]
    fn a_decimal_percentage_is_rounded() {
        let output = "  battery\n    present: yes\n    state: charging\n    percentage: 54.6%\n";
        let battery = parse(output).unwrap().unwrap();
        assert_eq!((battery.percent, battery.charge), (55, Charge::Charging));
    }
}
