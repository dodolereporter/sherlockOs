//! Control Room, the settings application (lot D-26a): its sizes, motions and texts. The window
//! follows the « Réglages » board (`docs/design/planches/Reglages.dc.html`): a sidebar of 240
//! with the search on top, the page on the right in cards. Pages without a board follow its
//! rules. See `docs/control-room.md`.

use std::time::Duration;

use crate::{metrics, motion};

/// Width of the sidebar (board).
pub const SIDEBAR_WIDTH: f32 = 240.0;
/// Padding of the sidebar: top and bottom, sides (board: 12 and 10).
pub const SIDEBAR_PADDING: (f32, f32) = (12.0, 10.0);
/// Gap between two entries of the sidebar (board), and below the search field.
pub const NAV_GAP: f32 = metrics::GAP_TIGHT;
pub const SEARCH_GAP: f32 = 8.0;
/// Gap between the cards of a page (board), and between the title and its subtitle.
pub const CARD_GAP: f32 = metrics::GAP_SECTION;
pub const TITLE_GAP: f32 = metrics::GAP_TIGHT;
/// Size of the subtitle under the title of a page (board).
pub const SUBTITLE_SIZE: f32 = 14.0;
/// Widest the content of a page grows: on a wide window, its cards stay readable.
pub const CONTENT_MAX_WIDTH: f32 = 760.0;
/// Below this window width the sidebar folds away and a button in the title bar brings it
/// back over the page: 240 of sidebar leave a page too narrow for its controls.
pub const COMPACT_WIDTH: f32 = 720.0;
/// Smallest window: one column of settings with their controls whole.
pub const MIN_WINDOW: (f32, f32) = (420.0, 360.0);
/// Default size of the window: the board's, 1140 by 676.
pub const WINDOW: (f32, f32) = (1140.0, 676.0);
/// Width of a control on the right of a setting (segmented control, menu, slider). In a
/// compact window, it goes under its label, full width.
pub const CONTROL_WIDTH: f32 = 300.0;
pub const SLIDER_WIDTH: f32 = 220.0;
/// Width of the dropdown menus of hours.
pub const HOUR_WIDTH: f32 = 110.0;
/// Below this width of the page, a control goes under its label.
pub const STACK_WIDTH: f32 = 560.0;

/// A wallpaper in the picker: its smallest thumbnail (16:9), the widest it grows to fill a
/// line of the card, its corner radius, the gap between two, the ring of the chosen one and
/// the gap between the ring and the picture.
pub const WALLPAPER_TILE: (f32, f32) = (128.0, 72.0);
pub const WALLPAPER_TILE_MAX: f32 = 176.0;
pub const WALLPAPER_RADIUS: f32 = 10.0;
pub const WALLPAPER_GAP: f32 = 12.0;
pub const WALLPAPER_RING: f32 = 2.0;
pub const WALLPAPER_RING_GAP: f32 = 2.0;
/// Pixels of a thumbnail per logical pixel: sharp on a screen at scale 2.
pub const THUMBNAIL_SCALE: f32 = 2.0;

/// A page that replaces another: it comes up from this far below while it fades in, on the
/// spring of an opening.
pub const PAGE_RISE: f32 = 12.0;
pub const PAGE_SPRING: motion::Spring = motion::OPEN_SPRING;
pub const PAGE_FADE: Duration = motion::FADE;
/// The folded sidebar slides in from the left edge on the same spring; the page darkens.
pub const SIDEBAR_SPRING: motion::Spring = motion::OPEN_SPRING;
pub const SIDEBAR_FADE: Duration = motion::FADE;

/// A setting opened from the search is lit with the selection's blue, which fades out over
/// this long: the eye finds it on its page.
pub const SPOTLIGHT: Duration = Duration::from_millis(1400);

/// A changed setting is written this long after the last change: a slider dragged writes once
/// it rests, not at every step.
pub const SAVE_DELAY: Duration = Duration::from_millis(metrics::EVENT_COALESCE_MS);
/// Step of the hours of the automatic theme, in minutes.
pub const HOUR_STEP: u32 = 30;

/// The interface texts of Control Room, in French.
pub mod texts {
    pub use crate::texts::evidence::{CLOSE, MAXIMIZE, MINIMIZE};

    pub const APP_NAME: &str = "Control Room";
    pub const SEARCH: &str = "Rechercher un réglage";
    pub const NO_RESULT: &str = "Aucun réglage ne correspond.";
    pub const SHOW_SIDEBAR: &str = "Afficher les sections";
    /// Under a setting whose file cannot be read or written: it is left alone.
    pub const LOCKED_FILE: &str = "Ce fichier de réglages ne peut pas être modifié.";

    // Pages: their name in the sidebar, the subtitle under their title.
    pub const APPEARANCE: &str = "Apparence";
    pub const APPEARANCE_ABOUT: &str = "Le thème, le fond d'écran et le dock.";
    pub const TOUCHPAD: &str = "Pavé tactile et souris";
    pub const TOUCHPAD_ABOUT: &str = "Comment tes doigts cliquent, glissent et défilent.";

    // « Apparence ».
    pub const THEME: &str = "Thème";
    pub const THEME_HELP: &str = "Sombre, clair, ou selon l'heure.";
    pub const DARK: &str = "Sombre";
    pub const LIGHT: &str = "Clair";
    pub const AUTO: &str = "Auto";
    pub const LIGHT_FROM: &str = "Clair à partir de";
    pub const LIGHT_FROM_HELP: &str = "Le bureau passe en clair à cette heure.";
    pub const DARK_FROM: &str = "Sombre à partir de";
    pub const DARK_FROM_HELP: &str = "Et il repasse en sombre à celle-ci.";
    pub const WALLPAPER: &str = "Fond d'écran";
    pub const FOLLOW_THEME: &str = "Suivre le thème";
    pub const FOLLOW_THEME_HELP: &str = "Prend la version claire ou sombre du fond, s'il en a une.";
    pub const OWN_IMAGE: &str = "Ton image";
    pub const OWN_IMAGE_HELP: &str = "Choisis un fond ci-dessous pour la remplacer.";
    pub const NO_WALLPAPER: &str = "Aucun fond d'écran n'est installé.";
    pub const DOCK: &str = "Dock";
    pub const DOCK_SIZE: &str = "Taille";
    pub const DOCK_SIZE_HELP: &str = "La taille des icônes au repos.";
    pub const MAGNIFICATION: &str = "Grossissement";
    pub const MAGNIFICATION_HELP: &str = "Les icônes grossissent sous le pointeur.";
    pub const MAGNIFICATION_FACTOR: &str = "Intensité";
    pub const MAGNIFICATION_FACTOR_HELP: &str = "Jusqu'où elles grossissent.";
    pub const AUTO_HIDE: &str = "Masquage";
    pub const AUTO_HIDE_HELP: &str = "Quand le dock se cache.";
    pub const HIDE_NEVER: &str = "Jamais";
    pub const HIDE_SMART: &str = "Plein écran";
    pub const HIDE_ALWAYS: &str = "Toujours";
    pub const POSITION: &str = "Position";
    pub const POSITION_HELP: &str = "Le bord de l'écran où il se place.";
    pub const BOTTOM: &str = "En bas";
    pub const LEFT: &str = "À gauche";
    pub const RIGHT: &str = "À droite";
    pub const TOP: &str = "En haut";

    // « Pavé tactile et souris ».
    pub const CLICKS: &str = "Clics";
    pub const RIGHT_CLICK: &str = "Clic droit";
    pub const RIGHT_CLICK_HELP: &str = "Comment tu fais un clic droit sur le pavé.";
    pub const TWO_FINGERS: &str = "Appuyer à deux doigts";
    pub const CORNER: &str = "Coin en bas à droite";
    pub const TAP_TO_CLICK: &str = "Toucher pour cliquer";
    pub const TAP_TO_CLICK_HELP: &str = "Un léger toucher suffit, sans appuyer.";
    pub const SCROLL_AND_SPEED: &str = "Défilement et vitesse";
    pub const NATURAL_SCROLL: &str = "Défilement naturel";
    pub const NATURAL_SCROLL_HELP: &str = "Le contenu suit le mouvement de tes doigts.";
    pub const SPEED: &str = "Vitesse du pointeur";
    pub const SPEED_HELP: &str = "Plus elle est haute, plus il va loin.";
    pub const TYPING: &str = "Désactiver pendant la frappe";
    pub const TYPING_HELP: &str = "Ta paume ne bouge pas le pointeur quand tu écris.";
    pub const POINTER: &str = "Pointeur";
    pub const SHOW_CLICKS: &str = "Montrer les clics";
    pub const SHOW_CLICKS_HELP: &str = "Un anneau vert marque chaque clic.";

    // The search: names of the dock settings out of their card, and the other words each
    // setting is found by.
    pub const DOCK_SIZE_SEARCH: &str = "Taille du dock";
    pub const MAGNIFICATION_SEARCH: &str = "Intensité du grossissement";
    pub const AUTO_HIDE_SEARCH: &str = "Masquage du dock";
    pub const POSITION_SEARCH: &str = "Position du dock";
    pub const THEME_WORDS: &str = "sombre clair nuit jour mode couleurs";
    pub const HOURS_WORDS: &str = "thème heure automatique matin soir";
    pub const WALLPAPER_WORDS: &str = "image arrière-plan bureau";
    pub const DOCK_WORDS: &str = "dock icônes applications";
    pub const CLICK_WORDS: &str = "clic trackpad souris bouton appuyer toucher";
    pub const SCROLL_WORDS: &str = "défiler molette sens inverser trackpad";
    pub const POINTER_WORDS: &str = "pointeur souris curseur trackpad";
    pub const TOUCHPAD_WORDS: &str = "trackpad souris paume clavier";

    /// Hour of the day as the menus show it: « 07:30 ».
    pub fn hour(minutes: u32) -> String {
        format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hours_read_as_a_clock() {
        assert_eq!(texts::hour(7 * 60 + 30), "07:30");
        assert_eq!(texts::hour(0), "00:00");
        assert_eq!(texts::hour(23 * 60 + 59), "23:59");
    }

    #[test]
    fn the_compact_window_holds_a_page() {
        const {
            assert!(MIN_WINDOW.0 < COMPACT_WIDTH);
            assert!(CONTROL_WIDTH < STACK_WIDTH);
            assert!(CONTROL_WIDTH + 2.0 * metrics::CARD_PADDING.1 < MIN_WINDOW.0);
            assert!(WINDOW.0 >= COMPACT_WIDTH);
        }
    }
}
