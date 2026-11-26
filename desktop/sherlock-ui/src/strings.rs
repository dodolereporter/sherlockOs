//! Shared interface texts, in French and using « tu ». A program writes no interface text
//! itself: it takes it from here.

/// Abbreviated days of the week, Monday to Sunday.
pub const WEEKDAYS_SHORT: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];

/// Abbreviated months, January to December.
pub const MONTHS_SHORT: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Short date of the top bar, such as « mar. 24 nov. ». `weekday` goes from 1 (Monday) to 7,
/// `month` from 1 to 12. Returns `None` outside these bounds.
pub fn short_date(weekday: u8, day: u8, month: u8) -> Option<String> {
    let weekday = WEEKDAYS_SHORT.get(usize::from(weekday).checked_sub(1)?)?;
    let month = MONTHS_SHORT.get(usize::from(month).checked_sub(1)?)?;
    Some(format!("{weekday} {day} {month}"))
}

/// 24-hour time, such as « 09:05 ».
pub fn clock(hour: u8, minute: u8) -> String {
    format!("{hour:02}:{minute:02}")
}

/// Percentage, with the French narrow no-break space: « 55 % ».
pub fn percent(value: u8) -> String {
    format!("{value}\u{202F}%")
}

/// Top bar texts.
pub mod top_bar {
    /// Workspace label: its number, then its name if it has one (« 1 · Code »).
    pub fn workspace(number: usize, name: Option<&str>) -> String {
        match name {
            Some(name) if !name.is_empty() => format!("{number} · {name}"),
            _ => number.to_string(),
        }
    }

    /// Below the user name, at the top of the Sherlock menu.
    pub const SESSION_OPEN: &str = "Session ouverte";
    pub const ABOUT: &str = "À propos de cet ordinateur";
    pub const SETTINGS: &str = "Réglages";
    pub const UPDATES: &str = "Mises à jour";
    pub const GAME_MODE: &str = "Passer en mode jeu";
    pub const LOCK: &str = "Verrouiller";
    pub const LOG_OUT: &str = "Fermer la session";
    pub const POWER_OFF: &str = "Éteindre";
}

/// Names of the color themes given to GTK, Qt and KDE applications, as their theme pickers
/// show them.
pub mod toolkit_theme {
    /// The dark variant.
    pub const DARK: &str = "SherlockOS sombre";
    /// The light variant.
    pub const LIGHT: &str = "SherlockOS clair";
    /// The description of the GTK theme.
    pub const COMMENT: &str = "Couleurs du bureau SherlockOS";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_short_date_follows_the_design_board() {
        assert_eq!(short_date(2, 24, 11).as_deref(), Some("mar. 24 nov."));
        assert_eq!(short_date(7, 1, 8).as_deref(), Some("dim. 1 août"));
        assert_eq!(short_date(0, 1, 1), None);
        assert_eq!(short_date(1, 1, 13), None);
    }

    #[test]
    fn the_time_keeps_two_digits() {
        assert_eq!(clock(9, 5), "09:05");
        assert_eq!(clock(21, 46), "21:46");
    }

    #[test]
    fn a_workspace_without_name_keeps_its_number() {
        assert_eq!(top_bar::workspace(1, Some("Code")), "1 · Code");
        assert_eq!(top_bar::workspace(2, None), "2");
        assert_eq!(top_bar::workspace(3, Some("")), "3");
        assert_eq!(percent(55), "55\u{202F}%");
    }
}

/// Texts of Mission Control (lot D-33).
pub mod mission_control {
    /// Under the thumbnail of a workspace: « Bureau 2 ».
    pub fn workspace(number: usize) -> String {
        format!("Bureau {number}")
    }
}
