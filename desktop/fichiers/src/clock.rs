//! Heure locale, pour les dates de la liste et de la corbeille.

use std::mem::MaybeUninit;
use std::time::{SystemTime, UNIX_EPOCH};

use sherlock_ui::texts::LocalTime;

/// Convertit un instant en date et heure locales, fuseau du poste compris.
pub fn local(time: SystemTime) -> Option<LocalTime> {
    let seconds = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).ok()?,
        Err(before) => -i64::try_from(before.duration().as_secs()).ok()?,
    };
    let seconds: libc::time_t = seconds;
    let mut tm = MaybeUninit::<libc::tm>::uninit();
    // SAFETY: `localtime_r` est la version réentrante de `localtime` : elle n'écrit que dans
    // la structure qu'on lui passe, valide pour toute la durée de l'appel, et lit `seconds`
    // par un pointeur vers une variable locale. Elle rend un pointeur nul en cas d'échec ;
    // sinon la structure est entièrement remplie et peut être lue.
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
        // Une seconde intercalaire vaut 60 : on la ramène à 59.
        second: field(tm.tm_sec)?.min(59),
    })
}

/// Maintenant, en heure locale.
pub fn now() -> Option<LocalTime> {
    local(SystemTime::now())
}

/// Format de `DeletionDate` dans la corbeille freedesktop : `2026-11-24T21:46:00`.
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
    fn l_heure_locale_se_lit_et_s_ecrit() {
        let t = local(UNIX_EPOCH + Duration::from_secs(1_790_000_000)).expect("heure locale");
        // Le fuseau du poste décale l'heure, jamais de plus d'un jour.
        assert_eq!(t.year, 2026);
        assert!((t.days() - 20_717).abs() <= 1, "{t:?}");
        let text = iso(t);
        assert_eq!(text.len(), 19);
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[10..11], "T");
    }
}
