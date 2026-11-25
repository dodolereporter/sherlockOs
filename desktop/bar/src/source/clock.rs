//! Local clock. The standard library does not know the time zone: local time comes from the
//! `date` command, run once a minute.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sherlock_ui::strings;

use super::{Source, SourceError, run};

/// Longest sleep between two looks at the clock. The thread wakes up more often than once a
/// minute so it does not lag behind after a suspend; it only runs `date` once the minute
/// changed.
const MAX_SLEEP: Duration = Duration::from_secs(5);

/// Format given to `date`: day of the week (1 is Monday), day, month, hour, minute.
const DATE_FORMAT: &str = "+%u %d %m %H %M";

/// Local time, to the minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
    pub hour: u8,
    pub minute: u8,
}

impl LocalTime {
    /// Short date, such as « mar. 24 nov. ».
    pub fn date(&self) -> String {
        strings::short_date(self.weekday, self.day, self.month).unwrap_or_default()
    }

    /// Time, such as « 21:46 ».
    pub fn time(&self) -> String {
        strings::clock(self.hour, self.minute)
    }
}

/// Parses the output of `date "+%u %d %m %H %M"`.
pub fn parse(output: &str) -> Result<LocalTime, SourceError> {
    let error = || SourceError::Parse {
        command: "date",
        detail: output.trim().to_owned(),
    };
    let fields: Vec<u8> = output
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| error())?;
    let [weekday, day, month, hour, minute] = fields[..] else {
        return Err(error());
    };
    let valid = (1..=7).contains(&weekday)
        && (1..=31).contains(&day)
        && (1..=12).contains(&month)
        && hour < 24
        && minute < 60;
    if !valid {
        return Err(error());
    }
    Ok(LocalTime {
        weekday,
        day,
        month,
        hour,
        minute,
    })
}

/// The system clock.
#[derive(Debug, Default)]
pub struct SystemClock {
    minute: u64,
}

/// Minutes since 1970. Time zones are offset by whole minutes: the local minute changes at
/// the same time as this one.
fn epoch_minute() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 60)
}

impl Source for SystemClock {
    type State = LocalTime;

    fn read(&mut self) -> Result<LocalTime, SourceError> {
        self.minute = epoch_minute();
        parse(&run("date", &[DATE_FORMAT])?)
    }

    fn wait(&mut self) -> bool {
        while epoch_minute() == self.minute {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let to_next = Duration::from_secs(60 - now.as_secs() % 60);
            std::thread::sleep(to_next.min(MAX_SLEEP));
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_output_gives_the_time_of_the_design_board() {
        // Output of `date "+%u %d %m %H %M"` on Tuesday 24 November 2026 at 21:46.
        let time = parse("2 24 11 21 46\n").unwrap();
        assert_eq!(time.date(), "mar. 24 nov.");
        assert_eq!(time.time(), "21:46");
    }

    #[test]
    fn midnight_and_small_numbers_keep_two_digits() {
        let time = parse("7 01 08 00 05").unwrap();
        assert_eq!(time.date(), "dim. 1 août");
        assert_eq!(time.time(), "00:05");
    }

    #[test]
    fn unexpected_output_is_rejected() {
        assert!(parse("").is_err());
        assert!(parse("2 24 11 21").is_err());
        assert!(parse("8 24 11 21 46").is_err());
        assert!(parse("2 24 11 24 00").is_err());
        assert!(parse("mar. 24 nov.").is_err());
    }
}
