//! Local clock. The standard library does not know the time zone: local time comes from the
//! C library (`localtime_r`), read once a minute.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sherlock_ui::strings;

use super::{Source, SourceError};

/// Longest sleep between two looks at the clock. The thread wakes up more often than once a
/// minute so it does not lag behind after a suspend; it only reads the local time once the
/// minute changed.
const MAX_SLEEP: Duration = Duration::from_secs(5);

/// Local time, to the minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub year: i32,
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

impl LocalTime {
    /// The fields of a C `struct tm`: year from 1900, day of the week from 0 (Sunday), day,
    /// month from 0, hour, minute.
    pub fn from_tm(year: i32, wday: i32, mday: i32, mon: i32, hour: i32, min: i32) -> Option<Self> {
        let field = |v: i32, range: std::ops::RangeInclusive<i32>| {
            range.contains(&v).then(|| u8::try_from(v).ok()).flatten()
        };
        Some(Self {
            year: year.checked_add(1900)?,
            // Monday is 1 and Sunday 7, as in `strings::short_date`.
            weekday: field(wday, 0..=6).map(|d| if d == 0 { 7 } else { d })?,
            day: field(mday, 1..=31)?,
            month: field(mon + 1, 1..=12)?,
            hour: field(hour, 0..=23)?,
            minute: field(min, 0..=59)?,
        })
    }
}

/// Local time of `epoch`, in seconds since 1970, from `localtime_r`
/// (`appearance_config::local`, which reads the time zone again first).
pub fn local(epoch: i64) -> Result<LocalTime, SourceError> {
    let error = |detail: String| SourceError::Parse {
        command: "localtime_r",
        detail,
    };
    let tm = appearance_config::local(epoch).map_err(|e| error(e.to_string()))?;
    LocalTime::from_tm(
        tm.tm_year, tm.tm_wday, tm.tm_mday, tm.tm_mon, tm.tm_hour, tm.tm_min,
    )
    .ok_or_else(|| error(format!("{epoch} is out of range")))
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
        local(i64::try_from(self.minute * 60).unwrap_or(i64::MAX))
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
    fn the_fields_of_the_c_library_give_the_time_of_the_design_board() {
        // Tuesday 24 November 2026 at 21:46: day 2 of the week, month 10 from 0.
        let time = LocalTime::from_tm(126, 2, 24, 10, 21, 46).unwrap();
        assert_eq!(time.year, 2026);
        assert_eq!(time.date(), "mar. 24 nov.");
        assert_eq!(time.time(), "21:46");
    }

    #[test]
    fn sunday_midnight_and_small_numbers_keep_two_digits() {
        let time = LocalTime::from_tm(126, 0, 1, 7, 0, 5).unwrap();
        assert_eq!(time.weekday, 7);
        assert_eq!(time.date(), "dim. 1 août");
        assert_eq!(time.time(), "00:05");
    }

    #[test]
    fn fields_out_of_range_are_rejected() {
        assert!(LocalTime::from_tm(126, 7, 24, 10, 21, 46).is_none());
        assert!(LocalTime::from_tm(126, 2, 0, 10, 21, 46).is_none());
        assert!(LocalTime::from_tm(126, 2, 24, 12, 21, 46).is_none());
        assert!(LocalTime::from_tm(126, 2, 24, 10, 24, 0).is_none());
        assert!(LocalTime::from_tm(126, 2, 24, 10, 21, 60).is_none());
    }

    #[test]
    fn the_local_time_follows_the_clock_of_the_machine() {
        // 2026-11-24 20:46 UTC: the local time is a whole number of quarter hours away, within
        // a day, and the minute stays.
        let time = local(1_795_553_160).unwrap();
        assert_eq!(time.minute % 15, 46 % 15);
        assert!((23..=25).contains(&time.day), "{time:?}");
        let mut clock = SystemClock::default();
        assert!(clock.read().is_ok());
    }
}
