//! Rayons, espacements et tailles, en pixels logiques. Valeurs de `docs/contexte.md` et des
//! planches Menus et Réglages.

/// Écran de référence des planches.
pub const REFERENCE_SCREEN: (f32, f32) = (1440.0, 900.0);

/// Les trois disques du fond d'écran, sur l'écran de référence : gauche, haut et diamètre de
/// chacun, dans l'ordre de `color::DESKTOP_DISCS` (planche Main). Le dernier passe devant.
pub const DESKTOP_DISC_GEOMETRY: [(f32, f32, f32); 3] = [
    (-260.0, 340.0, 1100.0),
    (860.0, -420.0, 900.0),
    (980.0, 520.0, 520.0),
];

// Barre du haut.

/// Hauteur de la barre du haut.
pub const TOP_BAR_HEIGHT: f32 = 36.0;
/// Marges de la barre du haut sur les côtés.
pub const TOP_BAR_MARGIN_SIDE: f32 = 12.0;
/// Marge de la barre du haut au-dessus.
pub const TOP_BAR_MARGIN_TOP: f32 = 10.0;
/// Rayon de la barre du haut.
pub const TOP_BAR_RADIUS: f32 = 12.0;
/// Inner padding of the top bar: left, right.
pub const TOP_BAR_PADDING: (f32, f32) = (6.0, 10.0);
/// Gap between the Sherlock button and the application name.
pub const TOP_BAR_LEFT_GAP: f32 = 4.0;
/// Sherlock button: width, height, radius, size of the logo symbol in it.
pub const SHERLOCK_BUTTON: (f32, f32) = (30.0, 28.0);
pub const SHERLOCK_BUTTON_RADIUS: f32 = 8.0;
pub const SHERLOCK_GLYPH: f32 = 22.0;
/// Active application name: left and right padding.
pub const APP_NAME_PADDING: (f32, f32) = (8.0, 10.0);
/// Workspace chip: height, side padding, radius, gap between two chips.
pub const CHIP_HEIGHT: f32 = 24.0;
pub const CHIP_PADDING: f32 = 12.0;
pub const CHIP_RADIUS: f32 = 12.0;
pub const CHIP_GAP: f32 = 4.0;
/// Right-hand indicators: gap between two indicators, between an icon and its value.
pub const STATUS_GAP: f32 = 14.0;
pub const STATUS_ICON_GAP: f32 = 6.0;
/// Gap between the bottom of the bar and a menu it opens (Menus board: 46 then 52).
pub const TOP_BAR_MENU_GAP: f32 = 6.0;
/// Sherlock menu header: side of the avatar (the logo in its square), top, side and bottom
/// padding.
pub const MENU_AVATAR: f32 = 40.0;
pub const MENU_HEADER_PADDING: (f32, f32, f32) = (6.0, 8.0, 8.0);

// Fenêtres et panneaux.

/// Rayon d'une fenêtre.
pub const WINDOW_RADIUS: f32 = 16.0;
/// Hauteur de la barre de titre.
pub const TITLE_BAR_HEIGHT: f32 = 40.0;
/// Bouton de fenêtre (réduire, agrandir, fermer) : largeur et hauteur.
pub const WINDOW_BUTTON: (f32, f32) = (28.0, 24.0);
/// Épaisseur des bordures de fenêtre, de panneau, de menu et de carte.
pub const BORDER_WIDTH: f32 = 1.0;
/// Ombre d'une fenêtre ou d'un panneau : décalage vertical et flou.
pub const SHADOW_OFFSET: f32 = 28.0;
pub const SHADOW_BLUR: f32 = 70.0;
/// Rayon d'un panneau flottant.
pub const PANEL_RADIUS: f32 = 20.0;
/// Marge intérieure d'un panneau flottant.
pub const PANEL_PADDING: f32 = 14.0;

// Menus.

/// Rayon d'un menu.
pub const MENU_RADIUS: f32 = 12.0;
/// Marge intérieure d'un menu.
pub const MENU_PADDING: f32 = 6.0;
/// Largeur d'un menu et d'un sous-menu.
pub const MENU_WIDTH: f32 = 256.0;
pub const SUBMENU_WIDTH: f32 = 190.0;
/// Écart entre un menu et son sous-menu ouvert à droite (planche Menus : 674 puis 678).
pub const SUBMENU_GAP: f32 = 4.0;
/// Ombre d'un menu : décalage vertical et flou.
pub const MENU_SHADOW_OFFSET: f32 = 22.0;
pub const MENU_SHADOW_BLUR: f32 = 54.0;
/// Entrée de menu : hauteur, rayon, marge sur les côtés.
pub const MENU_ITEM_HEIGHT: f32 = 30.0;
pub const MENU_ITEM_RADIUS: f32 = 7.0;
pub const MENU_ITEM_PADDING: f32 = 10.0;
/// Séparateur de menu : marges au-dessus et sur les côtés.
pub const MENU_SEPARATOR_MARGIN: (f32, f32) = (4.0, 8.0);

// Contrôles.

/// Bouton : hauteur, rayon, marge sur les côtés. La gamme va de 28 à 36 de haut.
pub const BUTTON_HEIGHT: f32 = 34.0;
pub const BUTTON_RADIUS: f32 = 9.0;
pub const BUTTON_PADDING: f32 = 14.0;
/// Bouton compact, dans un panneau ou une notification.
pub const BUTTON_COMPACT_HEIGHT: f32 = 28.0;
pub const BUTTON_COMPACT_RADIUS: f32 = 8.0;
/// Bouton d'icône seule : côté et rayon.
pub const ICON_BUTTON: f32 = 32.0;
pub const ICON_BUTTON_RADIUS: f32 = 8.0;
/// Interrupteur : largeur, hauteur, retrait du bouton rond.
pub const TOGGLE_WIDTH: f32 = 44.0;
pub const TOGGLE_HEIGHT: f32 = 26.0;
pub const TOGGLE_INSET: f32 = 3.0;
/// Champ de texte : hauteur, rayon, marge sur les côtés.
pub const FIELD_HEIGHT: f32 = 32.0;
pub const FIELD_RADIUS: f32 = 9.0;
pub const FIELD_PADDING: f32 = 10.0;
/// Carte : rayon, marges verticale et horizontale.
pub const CARD_RADIUS: f32 = 14.0;
pub const CARD_PADDING: (f32, f32) = (16.0, 18.0);
/// Hauteur minimale d'une ligne de réglage dans une carte.
pub const SETTING_ROW_HEIGHT: f32 = 48.0;
/// Anneau du focus : épaisseur et écart avec l'élément.
pub const FOCUS_RING_WIDTH: f32 = 2.0;
pub const FOCUS_RING_GAP: f32 = 2.0;

// Icônes.

/// Grille de dessin des icônes.
pub const ICON_GRID: f32 = 24.0;
/// Épaisseur du trait des icônes, prise dans la gamme 1,6 à 1,7.
pub const ICON_STROKE: f32 = 1.7;
/// Taille d'affichage courante d'une icône, et celle des chevrons de menu.
pub const ICON_SIZE: f32 = 16.0;
pub const ICON_SIZE_SMALL: f32 = 14.0;

// Dock.

/// Hauteur et rayon de la barre du dock.
pub const DOCK_HEIGHT: f32 = 76.0;
pub const DOCK_RADIUS: f32 = 22.0;
/// Icône du dock : côté, rayon en part du côté, écart entre deux icônes.
pub const DOCK_ICON: f32 = 52.0;
pub const DOCK_ICON_RADIUS_RATIO: f32 = 0.27;
pub const DOCK_ICON_GAP: f32 = 8.0;
/// Grossissement : facteur maximal et portée de la cloche, en icônes.
pub const DOCK_MAGNIFICATION: f32 = 1.7;
pub const DOCK_MAGNIFICATION_REACH: f32 = 1.2;

// Espacements, par rôle.

/// Entre deux entrées d'une navigation.
pub const GAP_TIGHT: f32 = 2.0;
/// Entre une icône et son texte.
pub const GAP_ICON: f32 = 10.0;
/// Entre le texte d'une entrée et son raccourci.
pub const GAP_INLINE: f32 = 12.0;
/// Entre deux blocs d'une page, entre les cartes.
pub const GAP_SECTION: f32 = 14.0;
/// Entre un libellé et son contrôle dans une ligne de réglage.
pub const GAP_ROW: f32 = 16.0;
/// Marges d'une page de fenêtre : verticale et horizontale.
pub const PAGE_PADDING: (f32, f32) = (22.0, 28.0);

// Durées, en millisecondes.

/// Survol : de 110 à 150 ms.
pub const HOVER_MS: (u64, u64) = (110, 150);
/// Grossissement du dock.
pub const DOCK_TRANSITION_MS: u64 = 150;
/// Window in which a burst of system events (volume slider, network changes) is merged into
/// a single refresh of the shell.
pub const EVENT_COALESCE_MS: u64 = 150;
/// Longest such burst: a volume slider dragged for seconds still refreshes this often.
pub const EVENT_COALESCE_MAX_MS: u64 = 500;

// Cohérence des jetons, vérifiée à la compilation : un jeton hors de sa gamme ne compile pas.
const _: () = {
    assert!(BUTTON_COMPACT_HEIGHT >= 28.0 && BUTTON_HEIGHT <= 36.0);
    assert!(BUTTON_COMPACT_HEIGHT < BUTTON_HEIGHT);
    assert!(BUTTON_COMPACT_RADIUS >= 8.0 && BUTTON_RADIUS <= 10.0);
    assert!(ICON_STROKE >= 1.6 && ICON_STROKE <= 1.7);
    assert!(HOVER_MS.0 <= HOVER_MS.1 && DOCK_TRANSITION_MS <= HOVER_MS.1);
    assert!(EVENT_COALESCE_MS < EVENT_COALESCE_MAX_MS);
    // Le bouton rond de l'interrupteur fait 20 et tient deux fois dans sa piste.
    assert!(TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET == 20.0);
    assert!(2.0 * (TOGGLE_HEIGHT - 2.0 * TOGGLE_INSET) < TOGGLE_WIDTH);
    // Une entrée de menu tient dans le menu, un sous-menu est plus étroit.
    assert!(MENU_ITEM_RADIUS < MENU_RADIUS && MENU_ITEM_HEIGHT < BUTTON_HEIGHT);
    assert!(SUBMENU_WIDTH < MENU_WIDTH);
    assert!(ICON_SIZE_SMALL < ICON_SIZE && ICON_SIZE < ICON_GRID);
    // The bar's elements fit in its height.
    assert!(SHERLOCK_BUTTON.1 < TOP_BAR_HEIGHT && CHIP_HEIGHT < TOP_BAR_HEIGHT);
    assert!(SHERLOCK_GLYPH < SHERLOCK_BUTTON.1);
};
