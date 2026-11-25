//! Textes d'interface partagés, en français et au tutoiement. Un programme n'écrit aucun texte
//! d'interface lui-même : il le prend ici.

/// Jours de la semaine abrégés, du lundi au dimanche.
pub const WEEKDAYS_SHORT: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];

/// Mois abrégés, de janvier à décembre.
pub const MONTHS_SHORT: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Date courte de la barre du haut, comme « mar. 24 nov. ». `weekday` va de 1 (lundi) à 7,
/// `month` de 1 à 12. Rend `None` hors de ces bornes.
pub fn short_date(weekday: u8, day: u8, month: u8) -> Option<String> {
    let weekday = WEEKDAYS_SHORT.get(usize::from(weekday).checked_sub(1)?)?;
    let month = MONTHS_SHORT.get(usize::from(month).checked_sub(1)?)?;
    Some(format!("{weekday} {day} {month}"))
}

/// Heure sur 24 heures, comme « 09:05 ».
pub fn clock(hour: u8, minute: u8) -> String {
    format!("{hour:02}:{minute:02}")
}

/// Pourcentage, avec l'espace fine insécable du français : « 55 % ».
pub fn percent(value: u8) -> String {
    format!("{value}\u{202F}%")
}

/// Textes de la barre du haut.
pub mod top_bar {
    /// Nom d'un bureau : son numéro, puis son nom s'il en a un (« 1 · Code »).
    pub fn workspace(number: usize, name: Option<&str>) -> String {
        match name {
            Some(name) if !name.is_empty() => format!("{number} · {name}"),
            _ => number.to_string(),
        }
    }

    /// Sous le nom de l'utilisateur, en tête du menu Sherlock.
    pub const SESSION_OPEN: &str = "Session ouverte";
    pub const ABOUT: &str = "À propos de cet ordinateur";
    pub const SETTINGS: &str = "Réglages";
    pub const SETTINGS_SHORTCUT: &str = "Super+,";
    pub const UPDATES: &str = "Mises à jour";
    pub const GAME_MODE: &str = "Passer en mode jeu";
    pub const GAME_MODE_SHORTCUT: &str = "Super+G";
    pub const LOCK: &str = "Verrouiller";
    pub const LOCK_SHORTCUT: &str = "Super+L";
    pub const LOG_OUT: &str = "Fermer la session";
    pub const POWER_OFF: &str = "Éteindre";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_date_courte_suit_la_planche() {
        assert_eq!(short_date(2, 24, 11).as_deref(), Some("mar. 24 nov."));
        assert_eq!(short_date(7, 1, 8).as_deref(), Some("dim. 1 août"));
        assert_eq!(short_date(0, 1, 1), None);
        assert_eq!(short_date(1, 1, 13), None);
    }

    #[test]
    fn l_heure_garde_deux_chiffres() {
        assert_eq!(clock(9, 5), "09:05");
        assert_eq!(clock(21, 46), "21:46");
    }

    #[test]
    fn un_bureau_sans_nom_garde_son_numero() {
        assert_eq!(top_bar::workspace(1, Some("Code")), "1 · Code");
        assert_eq!(top_bar::workspace(2, None), "2");
        assert_eq!(top_bar::workspace(3, Some("")), "3");
        assert_eq!(percent(55), "55\u{202F}%");
    }
}
