//! Local time, for the dates of the listing and the trash.

use std::mem::MaybeUninit;
use std::time::{SystemTime, UNIX_EPOCH};

use sherlock_ui::texts::LocalTime;

/// Converts an instant to a local date and time, in the machine's time zone.
pub fn local(time: SystemTime) -> Option<LocalTime> {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).ok()?,
        Err(before) => -i64::try_from(before.duration().as_secs()).ok()?,
    };
    let seconds: libc::time_t = seconds;
    let mut tm = MaybeUninit::<libc::tm>::uninit();
    // SAFETY: `localtime_r` is the reentrant version of `localtime`: it only writes into the
    // structure passed to it, valid for the whole call, and reads `seconds` through a pointer
    // to a local variable. It returns a null pointer on failure; otherwise the structure is
    // fully filled in and can be read.
    let tm = unsafe {
        if libc::localtime_r(&seconds, tm.as_mut_ptr()).is_null() {
            return None;
        }
        tm.assume_init()
    };
    let field = |v: libc::c_int| u8::try_from(v).ok();
    Some(LocalTime {
        year: tm.tm_year + 1900,
        month: field(tm.tm_mon + 1)?,
        day: field(tm.tm_mday)?,
        hour: field(tm.tm_hour)?,
        minute: field(tm.tm_min)?,
        // A leap second is 60: clamp it to 59.
        second: field(tm.tm_sec)?.min(59),
    })
}

/// Converts a local date and time back to an instant (`mktime`, the machine's time zone).
pub fn to_system(t: LocalTime) -> Option<SystemTime> {
    // SAFETY: an all-zero `tm` is a valid value (integers and a null zone pointer); every
    // field mktime reads is set below.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = t.year - 1900;
    tm.tm_mon = libc::c_int::from(t.month) - 1;
    tm.tm_mday = libc::c_int::from(t.day);
    tm.tm_hour = libc::c_int::from(t.hour);
    tm.tm_min = libc::c_int::from(t.minute);
    tm.tm_sec = libc::c_int::from(t.second);
    // Let the C library decide whether summer time applies.
    tm.tm_isdst = -1;
    // SAFETY: `tm` is a valid, initialised structure owned by this function; mktime reads
    // it and normalises it in place.
    let seconds = unsafe { libc::mktime(&mut tm) };
    if seconds == -1 {
        return None;
    }
    let seconds = u64::try_from(seconds).ok()?;
    Some(UNIX_EPOCH + std::time::Duration::from_secs(seconds))
}

/// Now, in local time.
pub fn now() -> Option<LocalTime> {
    local(SystemTime::now())
}

/// Format of `DeletionDate` in the freedesktop trash: `2026-11-24T21:46:00`.
pub fn iso(t: LocalTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn local_time_reads_and_writes() {
        let t = local(UNIX_EPOCH + Duration::from_secs(1_790_000_000)).expect("local time");
        // The machine's time zone shifts the hour, never by more than a day.
        assert_eq!(t.year, 2026);
        assert!((t.days() - 20_717).abs() <= 1, "{t:?}");
        let text = iso(t);
        assert_eq!(text.len(), 19);
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[10..11], "T");
    }
}
