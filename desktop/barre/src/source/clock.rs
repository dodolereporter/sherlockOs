//! Horloge locale. La bibliothèque standard ne connaît pas le fuseau horaire : l'heure locale
//! vient de la commande `date`, lancée une fois par minute.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sherlock_ui::strings;

use super::{Source, SourceError, run};

/// Le plus long sommeil entre deux regards sur l'horloge. Le fil se réveille plus souvent que
/// la minute pour ne pas rester en retard après une mise en veille ; il ne relit `date` que
/// quand la minute a changé.
const MAX_SLEEP: Duration = Duration::from_secs(5);

/// Format passé à `date` : jour de la semaine (1 pour lundi), jour, mois, heure, minute.
const DATE_FORMAT: &str = "+%u %d %m %H %M";

/// Heure locale, à la minute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub weekday: u8,
    pub day: u8,
    pub month: u8,
    pub hour: u8,
    pub minute: u8,
}

impl LocalTime {
    /// Date courte, comme « mar. 24 nov. ».
    pub fn date(&self) -> String {
        strings::short_date(self.weekday, self.day, self.month).unwrap_or_default()
    }

    /// Heure, comme « 21:46 ».
    pub fn time(&self) -> String {
        strings::clock(self.hour, self.minute)
    }
}

/// Lit la sortie de `date "+%u %d %m %H %M"`.
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

/// L'horloge du système.
#[derive(Debug, Default)]
pub struct SystemClock {
    minute: u64,
}

/// Minutes écoulées depuis 1970. Les fuseaux sont décalés de minutes entières : la minute
/// locale change en même temps que celle-ci.
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
    fn la_sortie_de_date_donne_l_heure_de_la_planche() {
        // Sortie de `date "+%u %d %m %H %M"`, le mardi 24 novembre 2026 à 21:46.
        let time = parse("2 24 11 21 46\n").unwrap();
        assert_eq!(time.date(), "mar. 24 nov.");
        assert_eq!(time.time(), "21:46");
    }

    #[test]
    fn minuit_et_les_petits_nombres_gardent_deux_chiffres() {
        let time = parse("7 01 08 00 05").unwrap();
        assert_eq!(time.date(), "dim. 1 août");
        assert_eq!(time.time(), "00:05");
    }

    #[test]
    fn une_sortie_inattendue_est_refusee() {
        assert!(parse("").is_err());
        assert!(parse("2 24 11 21").is_err());
        assert!(parse("8 24 11 21 46").is_err());
        assert!(parse("2 24 11 24 00").is_err());
        assert!(parse("mar. 24 nov.").is_err());
    }
}
