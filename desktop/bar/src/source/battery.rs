//! Battery, read from UPower with the `upower` command. The "DisplayDevice" is the device
//! UPower composes for display: all the laptop's batteries as one. `upower --monitor` prints a
//! line on each `PropertiesChanged` of a device: the battery is read again then, never polled.

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
    /// Minutes until empty while discharging, until full while charging, when UPower knows.
    pub minutes: Option<u32>,
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
    let left = match charge {
        Charge::Charging => field(output, "time to full"),
        Charge::Discharging => field(output, "time to empty"),
        Charge::Full | Charge::Idle => None,
    };
    Ok(Some(Battery {
        percent: percent.round().clamp(0.0, 100.0) as u8,
        charge,
        minutes: left.and_then(minutes),
    }))
}

/// A duration as `upower` prints it (« 3.2 hours », « 45.0 minutes »), in minutes.
fn minutes(value: &str) -> Option<u32> {
    let (number, unit) = value.split_once(' ')?;
    let number: f32 = number.trim().parse().ok()?;
    let per = match unit.trim() {
        "seconds" | "second" => 1.0 / 60.0,
        "minutes" | "minute" => 1.0,
        "hours" | "hour" => 60.0,
        "days" | "day" => 1440.0,
        _ => return None,
    };
    let total = (number * per).round();
    (total.is_finite() && total > 0.0 && total < 1e6).then_some(total as u32)
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
                charge: Charge::Full,
                minutes: None,
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

    #[test]
    fn the_time_left_comes_with_the_state() {
        let output = "  battery\n    present: yes\n    state: discharging\n    \
                      time to empty:       3.2 hours\n    percentage: 40%\n";
        assert_eq!(parse(output).unwrap().unwrap().minutes, Some(192));
        let output = "  battery\n    present: yes\n    state: charging\n    \
                      time to full:        45.0 minutes\n    time to empty: 2 hours\n    \
                      percentage: 70%\n";
        assert_eq!(parse(output).unwrap().unwrap().minutes, Some(45));
        assert_eq!(minutes("1.5 days"), Some(2160));
        assert_eq!(minutes("soon"), None);
        assert_eq!(minutes("0.0 seconds"), None);
    }
}
