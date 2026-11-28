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
/// The results of the search come in one after the other, this far apart, each fading in over
/// `PAGE_FADE`; `HITS_ENTRANCE` is how long the whole list takes at most.
pub const HIT_STAGGER: Duration = Duration::from_millis(30);
pub const HITS_ENTRANCE: Duration = Duration::from_millis(30 * 12 + 200);
/// The results come up from this far below.
pub const HIT_RISE: f32 = 6.0;
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

/// The « Écrans » page (lot D-26b): the arrangement's area, the margin around the screens in
/// it, the corner radius of a screen, its thickness of the main screen's bar, and the
/// smallest a screen is drawn.
pub const ARRANGEMENT_HEIGHT: f32 = 200.0;
pub const ARRANGEMENT_MARGIN: f32 = 24.0;
pub const SCREEN_RADIUS: f32 = 6.0;
pub const MAIN_BAR: f32 = 5.0;
/// A change of mode, scale, rotation or of a screen on or off is undone after this long
/// without « Garder », as on macOS and Windows: a screen left black comes back by itself.
pub const CONFIRM_DELAY: Duration = Duration::from_secs(15);
/// The scales offered.
pub const SCALES: [f64; 5] = [1.0, 1.25, 1.5, 1.75, 2.0];
/// Width of the menus of resolution, refresh and rotation.
pub const DISPLAY_MENU_WIDTH: f32 = 220.0;
/// Width of the value written on the right of a repeat slider (« 1000 ms », « 50 par seconde »).
pub const VALUE_WIDTH: f32 = 104.0;
/// Width of the menu of the keyboard layouts.
pub const LAYOUT_WIDTH: f32 = CONTROL_WIDTH;
/// Steps of the repeat sliders: 10 ms for the delay, one repeat per second for the rate.
pub const REPEAT_DELAY_STEP: u32 = 10;
/// Most characters kept in the field where the keyboard is tried.
pub const TRIAL_MAX: usize = 200;

/// The keyboard layouts the menu offers: xkb layout, xkb variant, name shown. Any other layout
/// written in `keyboard.conf` still works, and shows as « Autre disposition ».
pub const KEYBOARD_LAYOUTS: [(&str, &str, &str); 15] = [
    ("fr", "", "Français (AZERTY)"),
    ("fr", "afnor", "Français (AZERTY AFNOR)"),
    ("fr", "bepo", "Français (BÉPO)"),
    ("fr", "mac", "Français (Mac)"),
    ("be", "", "Belge"),
    ("ch", "fr", "Suisse romand"),
    ("ca", "", "Français (Canada)"),
    ("us", "", "Anglais (États-Unis)"),
    ("us", "intl", "Anglais (international, touches mortes)"),
    ("gb", "", "Anglais (Royaume-Uni)"),
    ("de", "", "Allemand"),
    ("es", "", "Espagnol"),
    ("it", "", "Italien"),
    ("pt", "", "Portugais"),
    ("nl", "", "Néerlandais"),
];

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
    pub const BLUETOOTH: &str = "Bluetooth";
    pub const BLUETOOTH_ABOUT: &str = "Tes casques, souris et claviers sans fil.";
    // The Bluetooth page (D-26o).
    pub const BLUETOOTH_SWITCH: &str = "Bluetooth";
    pub const BLUETOOTH_SWITCH_HELP: &str = "Les appareils appairés se reconnectent seuls.";
    pub const BLUETOOTH_WORDS: &str = "sans fil casque écouteurs souris clavier manette appairer";
    pub const MY_DEVICES: &str = "Mes appareils";
    pub const NEARBY: &str = "Appareils à proximité";
    pub const NEARBY_SEARCH: &str = "Appareils à proximité Bluetooth";
    pub const SEARCHING: &str = "Recherche…";
    pub const NO_PAIRED: &str = "Aucun appareil appairé.";
    pub const NOTHING_NEARBY: &str = "Aucun appareil à proximité pour l'instant.";
    pub const BLUETOOTH_OFF_NOTE: &str = "Le Bluetooth est coupé.";
    pub const BLUETOOTH_LOADING: &str = "Lecture du Bluetooth…";
    pub const BLUETOOTH_UNAVAILABLE: &str = "Pas de Bluetooth sur cet ordinateur.";
    pub const CONNECT: &str = "Connecter";
    pub const DISCONNECT: &str = "Déconnecter";
    pub const PAIR: &str = "Appairer";
    pub const RENAME: &str = "Renommer";
    pub const CONFIRM: &str = "Confirmer";
    pub const OK: &str = "OK";
    pub const DEVICE_NAME: &str = "Nom de l'appareil";
    pub const BLUETOOTH_REFUSED: &str = "Refusé.";
    pub const BLUETOOTH_FAILED: &str = "Le Bluetooth n'a pas répondu.";
    pub const SOUND: &str = "Son";
    pub const SOUND_ABOUT: &str = "Où le son sort, d'où il entre, et à quel volume.";
    pub const LOCK: &str = "Verrouillage";
    pub const LOCK_ABOUT: &str = "Quand ta session se verrouille toute seule.";
    pub const KEYBOARD: &str = "Clavier";
    pub const KEYBOARD_ABOUT: &str =
        "La disposition des touches, leur répétition et les raccourcis.";
    pub const CLIPBOARD: &str = "Presse-papiers";
    pub const CLIPBOARD_ABOUT: &str = "Ce que Super + V garde de ce que tu copies.";
    pub const ACCESSIBILITY: &str = "Accessibilité";
    pub const ACCESSIBILITY_ABOUT: &str =
        "Des mouvements, de la transparence et du contraste à ta mesure.";
    pub const DEFAULT_APPS: &str = "Applications par défaut";
    pub const DEFAULT_APPS_ABOUT: &str = "Ce qui s'ouvre pour chaque sorte de fichier ou de lien.";

    pub const DISPLAYS: &str = "Écrans";
    pub const DISPLAYS_ABOUT: &str = "La définition, la taille et la place de chaque écran.";

    // « Écrans ».
    pub const ARRANGEMENT: &str = "Disposition des écrans";
    pub const ARRANGEMENT_HELP: &str = "Glisse les écrans pour les placer comme sur ton bureau.";
    pub const RESOLUTION: &str = "Résolution";
    pub const RESOLUTION_HELP: &str = "Plus elle est haute, plus l'image est fine.";
    pub const REFRESH: &str = "Fréquence";
    pub const REFRESH_HELP: &str = "Les images que l'écran affiche chaque seconde.";
    pub const SCALE: &str = "Échelle";
    pub const SCALE_HELP: &str = "La taille du texte et des fenêtres.";
    pub const ROTATION: &str = "Rotation";
    pub const ROTATION_HELP: &str = "Pour un écran tourné sur le côté.";
    pub const ROTATIONS: [&str; 4] = ["Standard", "90°", "180°", "270°"];
    pub const MAIN_DISPLAY: &str = "Écran principal";
    pub const MAIN_DISPLAY_HELP: &str = "La barre et les notifications s'y affichent.";
    pub const MAKE_MAIN: &str = "En faire l'écran principal";
    pub const IS_MAIN: &str = "C'est l'écran principal.";
    pub const DISPLAY_ON: &str = "Utiliser cet écran";
    pub const DISPLAY_ON_HELP: &str = "Éteint, il ne montre plus rien.";
    pub const KEEP_DISPLAYS: &str = "Garder ces réglages d'affichage ?";
    pub const KEEP: &str = "Garder";
    pub const REVERT: &str = "Revenir";
    pub const DISPLAYS_LOADING: &str = "Recherche des écrans…";
    pub const DISPLAYS_UNAVAILABLE: &str = "Les écrans ne se règlent pas ici pour l'instant.";
    pub const DISPLAY_REFUSED: &str = "L'écran a refusé ce réglage : rien n'a changé.";
    pub const DISPLAY_OFF: &str = "Éteint";

    /// What the confirmation says while it waits: « Retour aux réglages précédents dans 12 s. »
    pub fn reverting_in(seconds: u64) -> String {
        format!("Retour aux réglages précédents dans {seconds} s.")
    }

    /// A resolution in a menu: « 1920 × 1080 », « 2560 × 1600 (recommandée) ».
    pub fn resolution(width: i32, height: i32, preferred: bool) -> String {
        match preferred {
            true => format!("{width} × {height} (recommandée)"),
            false => format!("{width} × {height}"),
        }
    }

    /// A refresh rate in millihertz, as a menu shows it: « 60 Hz », « 143,98 Hz ».
    pub fn hertz(millihertz: i32) -> String {
        let hundredths = (f64::from(millihertz) / 10.0).round() as i64;
        match hundredths % 100 {
            0 => format!("{} Hz", hundredths / 100),
            rest => format!("{},{rest:02} Hz", hundredths / 100),
        }
    }

    /// The scales of `SCALES`, as the segmented control shows them.
    pub const SCALE_LABELS: [&str; 5] = ["100 %", "125 %", "150 %", "175 %", "200 %"];

    /// A scale as a percentage: « 125 % ».
    pub fn percent(scale: f64) -> String {
        format!("{} %", (scale * 100.0).round())
    }

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
    pub const NETWORK_WRONG_PASSWORD: &str = "Mot de passe incorrect : le réseau l'a refusé.";

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
    pub const GESTURES: &str = "Gestes";
    pub const TAP_AND_DRAG: &str = "Toucher puis glisser";
    pub const TAP_AND_DRAG_HELP: &str = "Touche deux fois et glisse pour déplacer un objet.";
    pub const DRAG_LOCK: &str = "Glissement prolongé";
    pub const DRAG_LOCK_HELP: &str = "Le glissement continue un instant après avoir levé le doigt.";
    pub const SWIPE_DOWN: &str = "Trois doigts vers le bas";
    pub const SWIPE_DOWN_HELP: &str = "Sur le bureau, hors de Mission Control.";
    pub const SWIPE_NOTHING: &str = "Rien";
    pub const SWIPE_MINIMIZE: &str = "Réduire la fenêtre";
    /// The gestures that do not change: the fingers, what they do.
    pub const FIXED_GESTURES: [(&str, &str); 3] = [
        ("Trois doigts vers le haut", "Ouvrir Mission Control"),
        ("Trois doigts à gauche ou à droite", "Changer de bureau"),
        ("Deux doigts écartés ou pincés", "Zoomer dans l'application"),
    ];

    // « Clavier ».
    pub const TYPING_CARD: &str = "Saisie";
    pub const LAYOUT: &str = "Disposition";
    pub const LAYOUT_HELP: &str = "Ce que tape chaque touche.";
    pub const SYSTEM_LAYOUT: &str = "Celle du système";
    pub const OTHER_LAYOUT: &str = "Autre disposition";
    pub const REPEAT_DELAY: &str = "Délai avant répétition";
    pub const REPEAT_DELAY_HELP: &str =
        "Combien de temps tenir une touche avant qu'elle se répète.";
    pub const REPEAT_RATE: &str = "Vitesse de répétition";
    pub const REPEAT_RATE_HELP: &str = "À quel rythme elle se répète ensuite.";
    pub const TRY_KEYS: &str = "Essai";
    pub const TRY_KEYS_HELP: &str = "Tape ici pour essayer la disposition et la répétition.";
    pub const TRY_KEYS_PLACEHOLDER: &str = "Tape quelques lettres…";
    pub const SHORTCUTS: &str = "Raccourcis du bureau";
    pub const SHORTCUTS_HELP: &str = "Dans une fenêtre d'essai, Alt remplace Super.";

    /// The shortcuts of the desktop, as the compositor takes them: the keys, what they do.
    pub const SHORTCUT_LIST: [(&str, &str); 16] = [
        ("Super + Espace", "Ouvrir le lanceur"),
        ("F4", "Afficher les applications"),
        ("Super + Tab", "Ouvrir Mission Control"),
        ("Super + L", "Verrouiller la session"),
        ("Super + ↑", "Agrandir ou rétablir la fenêtre"),
        (
            "Super + ← ou →",
            "Placer la fenêtre sur une moitié de l'écran",
        ),
        ("Super + ↓", "Remettre la fenêtre à sa place"),
        ("Super + glisser", "Déplacer une fenêtre"),
        ("Super + clic droit glissé", "Redimensionner une fenêtre"),
        ("Super + 1 à 9", "Aller à ce bureau"),
        ("Super + Maj + 1 à 9", "Envoyer la fenêtre sur ce bureau"),
        ("Super + Ctrl + ← ou →", "Aller au bureau voisin"),
        ("Impr. écran", "Capturer l'écran"),
        ("Maj + Impr. écran", "Capturer une zone"),
        ("Alt + Impr. écran", "Capturer la fenêtre"),
        ("Ctrl + Alt + F1 à F12", "Passer à une console"),
    ];

    /// A delay in milliseconds, a rate in repeats per second, as the sliders say them.
    pub fn milliseconds(count: u32) -> String {
        format!("{count} ms")
    }

    pub fn per_second(count: u32) -> String {
        format!("{count} par seconde")
    }

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

    // « Presse-papiers ».
    pub const CLIPBOARD_HISTORY_CARD: &str = "Historique";
    pub const CLIPBOARD_HISTORY: &str = "Historique du presse-papiers";
    pub const CLIPBOARD_HISTORY_HELP: &str = "Garde en mémoire tes 50 dernières copies, pour les retrouver avec Super + V. Jamais les mots de passe, rien pendant le verrouillage.";
    pub const CLIPBOARD_KEEP: &str = "Garder après la déconnexion";
    pub const CLIPBOARD_KEEP_HELP: &str = "Écrit l'historique sur le disque, avec les éléments épinglés. Sinon, il s'efface à la fin de la session.";
    // Accessibility (lot D-26p).
    pub const DISPLAY_CARD: &str = "Affichage";
    pub const REDUCE_MOTION: &str = "Réduire les animations";
    pub const REDUCE_MOTION_HELP: &str =
        "Les fenêtres, les panneaux et les vues apparaissent sans glisser ni rebondir.";
    pub const REDUCE_TRANSPARENCY: &str = "Réduire la transparence";
    pub const REDUCE_TRANSPARENCY_HELP: &str =
        "La barre, le dock, les menus et les bulles prennent un fond opaque.";
    pub const INCREASE_CONTRAST: &str = "Augmenter le contraste";
    pub const INCREASE_CONTRAST_HELP: &str =
        "Des bordures plus nettes et des textes secondaires plus foncés, sur des fonds opaques.";
    pub const EFFECTS: &str = "Effets visuels";
    pub const EFFECTS_HELP: &str = "Réduits : un fondu au lieu du génie, Sherlock en image fixe et Mission Control sans fenêtres animées dans ses vignettes. Pour un ordinateur lent.";
    pub const EFFECTS_FULL: &str = "Complets";
    pub const EFFECTS_REDUCED: &str = "Réduits";
    pub const EFFECTS_NONE: &str = "Aucun";

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
    pub const DISPLAY_WORDS: &str = "écran moniteur affichage définition taille hdmi projecteur";
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
    pub const KEYBOARD_WORDS: &str = "clavier azerty qwerty bépo langue touches";
    pub const REPEAT_WORDS: &str = "clavier touches répétition maintenir tenir";
    pub const SHORTCUT_WORDS: &str = "clavier raccourcis touches super combinaisons";
    pub const GESTURE_WORDS: &str = "gestes doigts trackpad glisser balayer";
    pub const CLIPBOARD_WORDS: &str = "presse-papiers copier coller historique super v vie privée";
    pub const MOTION_WORDS: &str = "animations mouvement vertige accessibilité";
    pub const TRANSPARENCY_WORDS: &str = "transparence flou verre opaque accessibilité";
    pub const CONTRAST_WORDS: &str = "contraste lisibilité bordures accessibilité";
    pub const EFFECTS_WORDS: &str =
        "effets visuels mode léger génie lent performances accessibilité";
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
    fn display_values_read_as_in_the_menus() {
        assert_eq!(texts::hertz(60_000), "60 Hz");
        assert_eq!(texts::hertz(59_951), "59,95 Hz");
        assert_eq!(texts::hertz(143_981), "143,98 Hz");
        assert_eq!(texts::percent(1.25), "125 %");
        for (scale, label) in SCALES.iter().zip(texts::SCALE_LABELS) {
            assert_eq!(texts::percent(*scale), label);
        }
        assert_eq!(
            texts::resolution(1920, 1080, true),
            "1920 × 1080 (recommandée)"
        );
        assert_eq!(
            texts::reverting_in(15),
            "Retour aux réglages précédents dans 15 s."
        );
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
