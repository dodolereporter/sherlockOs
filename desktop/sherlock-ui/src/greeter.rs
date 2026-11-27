//! The login screen, `sherlockos-greeter` (lot D-24, ADR 0026): the look of the lock screen
//! (`lock::screen`, whose sizes, colors and motions it takes), with the choice of the user and of
//! the session. macOS-like, as the author asked: no board draws it, the board of the login screen
//! inspires it. Every size is in logical pixels; those that follow the screen use
//! `lock::screen::fit`. See `docs/greeter.md`.

use std::time::Duration;

use crate::motion::{self, Spring};

/// The choice of the user, under Sherlock's bubble, which is the only avatar (lot D-24d, as on
/// macOS): the name of the selected user, then, when there are several, a discreet row of small
/// pills, one per user and « Autre… » last. A single user: only the name.
pub const CHIP_HEIGHT: f32 = 24.0;
pub const CHIP_TEXT_SIZE: f32 = 12.0;
pub const CHIP_PADDING: f32 = 10.0;
/// A pill never grows wider than this: a longer name ends with « … ».
pub const CHIP_MAX_WIDTH: f32 = 120.0;
pub const CHIP_GAP: f32 = 6.0;
/// The gap between the name and the row of pills.
pub const CHIPS_GAP: f32 = 10.0;

/// The name of the user just chosen rises in on this spring; Sherlock looks at the pills for
/// `LOOK_AT_USER`, then back ahead.
pub const SWAP_SPRING: Spring = motion::OPEN_SPRING;
pub const LOOK_AT_USER: Duration = Duration::from_millis(1200);

/// The choice of the session under the pill: a segmented control of glass, discreet.
pub const SEGMENT_WIDTH: f32 = 96.0;
pub const SEGMENT_HEIGHT: f32 = 26.0;
pub const SEGMENT_TEXT_SIZE: f32 = 12.0;
pub const SEGMENT_PADDING: f32 = 2.0;
pub const SESSION_GAP: f32 = 12.0;
/// The thumb slides between the segments on this spring.
pub const SEGMENT_SPRING: Spring = motion::TOGGLE_SPRING;
/// A disabled segment is this opaque.
pub const DISABLED_OPACITY: f32 = 0.4;
/// The line of the other sessions under the control.
pub const OTHERS_TEXT_SIZE: f32 = 12.0;
pub const OTHERS_GAP: f32 = 6.0;
/// The gap between two sessions of that list.
pub const OTHERS_ROW_GAP: f32 = 20.0;

/// Once logged in, Sherlock is happy this long before the screen leaves. The background stays:
/// the compositor of the session starts from the same picture (lot D-24d).
pub const SUCCESS_HOLD: Duration = Duration::from_millis(700);

/// The arrival of the desktop once the session opens (the compositor, `arrival.rs`): the bar
/// slides down, the dock rises `ARRIVAL_STAGGER` later, on `ARRIVAL_SPRING`; only within
/// `ARRIVAL_WINDOW` after the compositor started.
pub const ARRIVAL_SPRING: Spring = motion::OPEN_SPRING;
pub const ARRIVAL_STAGGER: Duration = Duration::from_millis(60);
pub const ARRIVAL_WINDOW: Duration = Duration::from_secs(15);

/// The texts of the screen.
pub const OTHER: &str = "Autre…";
pub const USER_NAME: &str = "Nom d'utilisateur";
pub const DESKTOP: &str = "Bureau";
pub const GAME: &str = "Mode jeu";
pub const GAME_SOON: &str = "Le mode jeu arrive bientôt";
pub const OTHER_SESSIONS: &str = "Autres sessions";
pub const HIDE_SESSIONS: &str = "Masquer les autres sessions";
/// Shown when the session chosen is not ours: « Session : Plasma (Wayland) ».
pub fn session_line(name: &str) -> String {
    format!("Session : {name}")
}
pub const LOG_IN: &str = "Se connecter";
pub const FAILED: &str = "Connexion impossible. Réessaie.";
pub const UNSUPPORTED: &str = "Cette demande n'est pas encore prise en charge ici.";
pub const NO_SESSION: &str = "Aucune session à ouvrir.";
pub const NO_USER: &str = "Tape ton nom d'utilisateur.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::screen;

    #[test]
    fn the_screen_still_fits_with_the_users_and_the_sessions() {
        // The four screens of the brief, by their logical height.
        for height in [768.0, 1080.0, 1067.0, 2160.0] {
            let used = screen::top(height)
                + screen::DATE_SIZE
                + screen::clock_size(height)
                + screen::bubble(height)
                + screen::BUBBLE_GAP
                + screen::NAME_SIZE
                + CHIPS_GAP
                + CHIP_HEIGHT
                + screen::NAME_GAP
                + screen::FIELD_HEIGHT
                + screen::HINT_GAP
                + screen::HINT_SIZE
                + SESSION_GAP
                + SEGMENT_HEIGHT
                + screen::machine_gap(height)
                + screen::MACHINE_BUTTON
                + screen::bottom(height);
            assert!(used < height * 0.9, "{height}: {used}");
        }
    }
}
