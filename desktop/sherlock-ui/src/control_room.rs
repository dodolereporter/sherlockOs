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
/// The delays before the lock the menu offers, in minutes; 0, never, last.
pub const LOCK_DELAYS: [u32; 8] = [1, 2, 5, 10, 15, 30, 60, 0];
/// Width of the menus of the lock delay and of the default applications.
pub const DELAY_WIDTH: f32 = 160.0;
pub const APP_MENU_WIDTH: f32 = CONTROL_WIDTH;

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
    pub const SOUND: &str = "Son";
    pub const SOUND_ABOUT: &str = "Où le son sort, d'où il entre, et à quel volume.";
    pub const LOCK: &str = "Verrouillage";
    pub const LOCK_ABOUT: &str = "Quand ta session se verrouille toute seule.";
    pub const DEFAULT_APPS: &str = "Applications par défaut";
    pub const DEFAULT_APPS_ABOUT: &str = "Ce qui s'ouvre pour chaque sorte de fichier ou de lien.";

    pub const NETWORK: &str = "Réseau";
    pub const NETWORK_ABOUT: &str = "Le Wi-Fi, les réseaux autour de toi et ceux que tu gardes.";

    // « Réseau ».
    pub const WIFI: &str = "Wi-Fi";
    pub const WIFI_HELP: &str = "Coupé, l'ordinateur ne cherche plus de réseau.";
    pub const WIFI_HARDWARE_OFF: &str =
        "Coupé par l'interrupteur ou le mode avion de l'ordinateur.";
    pub const NETWORKS: &str = "Réseaux";
    pub const NETWORK_LOADING: &str = "Lecture du Wi-Fi…";
    pub const NETWORK_UNAVAILABLE: &str = "Le gestionnaire de réseau ne répond pas.";
    pub const NO_WIFI: &str = "Aucune carte Wi-Fi trouvée.";
    pub const NO_NETWORK: &str = "Aucun réseau trouvé.";
    pub const CONNECTED: &str = "Connecté";
    pub const OPEN_NETWORK: &str = "Ouvert";
    pub const PROTECTED_NETWORK: &str = "Protégé par un mot de passe";
    pub const ENTERPRISE_NETWORK: &str = "Entreprise ou WEP";
    pub const FORGET: &str = "Oublier";
    pub const SCAN: &str = "Rechercher";
    pub const OTHER_NETWORK: &str = "Autre réseau…";
    pub const OTHER_NETWORK_SEARCH: &str = "Rejoindre un réseau caché";
    pub const NETWORK_NAME: &str = "Nom du réseau";
    pub const SECURITY_NONE: &str = "Aucune";
    pub const SECURITY_PASSWORD: &str = "Mot de passe";
    pub const SECURITY_WEP: &str = "WEP";
    pub const SECURITY_ENTERPRISE: &str = "Entreprise";
    pub const PEAP: &str = "PEAP";
    pub const TTLS: &str = "TTLS";
    pub const IDENTITY: &str = "Identifiant";
    pub const PASSWORD: &str = "Mot de passe";
    pub const WEP_KEY: &str = "Clé WEP";
    pub const DOMAIN: &str = "Domaine du serveur (facultatif)";
    pub const DOMAIN_HELP: &str =
        "Ton établissement le donne : il prouve que c'est bien son serveur qui répond.";
    pub const CANCEL: &str = "Annuler";
    pub const JOIN: &str = "Rejoindre";
    pub const NETWORK_REFUSED: &str = "Refusé : tu n'as pas le droit de changer ce réglage.";
    pub const NETWORK_FAILED: &str = "Échec : le réseau n'a pas pu être rejoint.";

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

    // « Son ».
    pub const OUTPUT: &str = "Sortie";
    pub const OUTPUT_DEVICE: &str = "Sortie du son";
    pub const OUTPUT_DEVICE_HELP: &str = "Le son passe par l'appareil choisi.";
    pub const OUTPUT_VOLUME: &str = "Volume de sortie";
    pub const INPUT: &str = "Entrée";
    pub const INPUT_DEVICE: &str = "Micro";
    pub const INPUT_DEVICE_HELP: &str = "Le micro que les applications entendent.";
    pub const INPUT_VOLUME: &str = "Volume d'entrée";
    pub const VOLUME_HELP: &str = "Glisse pour monter ou baisser.";
    pub const NO_DEVICE: &str = "Aucun appareil trouvé.";
    pub const SOUND_UNAVAILABLE: &str = "Le son ne répond pas pour l'instant.";
    pub const SOUND_LOADING: &str = "Recherche des appareils…";

    // « Verrouillage ».
    pub const AUTO_LOCK: &str = "Verrouillage automatique";
    pub const LOCK_AFTER: &str = "Verrouiller après";
    pub const LOCK_AFTER_HELP: &str =
        "Sans clavier ni souris pendant ce temps, la session se verrouille.";
    pub const NEVER: &str = "Jamais";
    pub const ONE_HOUR: &str = "1 heure";

    /// A delay before the lock: « 1 minute », « 5 minutes ».
    pub fn minutes(count: u32) -> String {
        match count {
            1 => String::from("1 minute"),
            n => format!("{n} minutes"),
        }
    }

    // « Applications par défaut ».
    pub const APPS_FOR_LINKS: &str = "Internet";
    pub const APPS_FOR_WORK: &str = "Travail";
    pub const APPS_FOR_MEDIA: &str = "Médias";
    pub const BROWSER: &str = "Navigateur";
    pub const BROWSER_HELP: &str = "Ouvre les liens et les pages web.";
    pub const MAIL: &str = "Mail";
    pub const MAIL_HELP: &str = "Écrit quand tu cliques sur une adresse.";
    pub const EDITOR: &str = "Éditeur de texte";
    pub const EDITOR_HELP: &str = "Ouvre les fichiers texte et le code.";
    pub const TERMINAL: &str = "Terminal";
    pub const TERMINAL_HELP: &str = "S'ouvre pour les programmes en ligne de commande.";
    pub const FILES: &str = "Fichiers";
    pub const FILES_HELP: &str = "Ouvre les dossiers.";
    pub const IMAGES: &str = "Images";
    pub const IMAGES_HELP: &str = "Ouvre les photos et les captures.";
    pub const VIDEO: &str = "Vidéo";
    pub const VIDEO_HELP: &str = "Lit les films et les vidéos.";
    pub const MUSIC: &str = "Musique";
    pub const MUSIC_HELP: &str = "Lit les morceaux.";
    pub const NO_APP: &str = "Aucune application installée.";
    pub const APPS_LOADING: &str = "Lecture des applications…";

    // The search: names of the dock settings out of their card, and the other words each
    // setting is found by.
    pub const DOCK_SIZE_SEARCH: &str = "Taille du dock";
    pub const MAGNIFICATION_SEARCH: &str = "Intensité du grossissement";
    pub const AUTO_HIDE_SEARCH: &str = "Masquage du dock";
    pub const POSITION_SEARCH: &str = "Position du dock";
    pub const NETWORK_WORDS: &str = "wifi internet réseau sans fil eduroam mot de passe";
    pub const THEME_WORDS: &str = "sombre clair nuit jour mode couleurs";
    pub const HOURS_WORDS: &str = "thème heure automatique matin soir";
    pub const WALLPAPER_WORDS: &str = "image arrière-plan bureau";
    pub const DOCK_WORDS: &str = "dock icônes applications";
    pub const CLICK_WORDS: &str = "clic trackpad souris bouton appuyer toucher";
    pub const SCROLL_WORDS: &str = "défiler molette sens inverser trackpad";
    pub const POINTER_WORDS: &str = "pointeur souris curseur trackpad";
    pub const TOUCHPAD_WORDS: &str = "trackpad souris paume clavier";
    pub const SOUND_WORDS: &str = "son audio haut-parleurs casque enceinte hdmi";
    pub const MICROPHONE_WORDS: &str = "son audio micro microphone entrée casque";
    pub const LOCK_WORDS: &str = "verrouillage écran veille inactivité délai sécurité";
    pub const APPS_WORDS: &str = "applications par défaut ouvrir avec programme";

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
