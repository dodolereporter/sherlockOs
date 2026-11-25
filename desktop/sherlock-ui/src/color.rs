//! Couleurs du thème, nommées par rôle. Valeurs de `docs/contexte.md` et des planches.

/// Une couleur sRGB, avec son opacité entre 0 et 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: f32,
}

impl Color {
    /// Couleur opaque, comme `#RRGGBB`.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Couleur translucide, comme `rgba(r, g, b, a)` en CSS.
    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Composantes entre 0 et 1, multipliées par l'opacité : la forme qu'attendent les moteurs
    /// de rendu pour mélanger les couleurs.
    pub fn premultiplied(self) -> [f32; 4] {
        let channel = |c: u8| f32::from(c) / 255.0 * self.a;
        [channel(self.r), channel(self.g), channel(self.b), self.a]
    }

    /// Mélange opaque : `t` vaut 0 pour `self`, 1 pour `other`. Sert aux nuances de survol.
    pub const fn mix(self, other: Self, t: f32) -> Self {
        const fn channel(a: u8, b: u8, t: f32) -> u8 {
            (a as f32 + (b as f32 - a as f32) * t + 0.5) as u8
        }
        Self::rgb(
            channel(self.r, other.r, t),
            channel(self.g, other.g, t),
            channel(self.b, other.b, t),
        )
    }

    /// `self` posée sur `below`, mélangée en sRGB comme le font le navigateur et tiny-skia.
    pub fn over(self, below: Self) -> Self {
        let a = self.a + below.a * (1.0 - self.a);
        if a <= 0.0 {
            return Self::rgba(0, 0, 0, 0.0);
        }
        let channel = |top: u8, bottom: u8| {
            let mixed =
                (f32::from(top) * self.a + f32::from(bottom) * below.a * (1.0 - self.a)) / a;
            mixed.round().clamp(0.0, 255.0) as u8
        };
        Self::rgba(
            channel(self.r, below.r),
            channel(self.g, below.g),
            channel(self.b, below.b),
            a,
        )
    }

    /// Luminance relative au sens des WCAG 2, opacité ignorée.
    pub fn luminance(self) -> f32 {
        let linear = |c: u8| {
            let c = f32::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    /// Rapport de contraste WCAG entre deux couleurs opaques, de 1 à 21.
    pub fn contrast(self, other: Self) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

#[cfg(feature = "iced")]
impl From<Color> for iced::Color {
    fn from(c: Color) -> Self {
        Self::from_rgba8(c.r, c.g, c.b, c.a)
    }
}

#[cfg(feature = "iced")]
impl From<iced::Color> for Color {
    fn from(c: iced::Color) -> Self {
        let [r, g, b, _] = c.into_rgba8();
        Self::rgba(r, g, b, c.a)
    }
}

// Fonds.

/// Fond d'écran du bureau.
pub const DESKTOP: Color = Color::rgb(0x0E, 0x1A, 0x2E);
/// Les trois grands disques du fond d'écran, du plus sombre au plus clair.
pub const DESKTOP_DISCS: [Color; 3] = [
    Color::rgb(0x13, 0x2A, 0x49),
    Color::rgb(0x16, 0x31, 0x4F),
    Color::rgb(0x1B, 0x3F, 0x63),
];
/// Fond des écrans système : connexion, démarrage, installeur.
pub const SYSTEM_SCREEN: Color = Color::rgb(0x0A, 0x14, 0x24);
/// Fond le plus sombre : terminal, zones de code. Sert aussi de texte sur le vert.
pub const DEEPEST: Color = Color::rgb(0x0B, 0x12, 0x20);
/// Corps des fenêtres opaques.
pub const WINDOW: Color = Color::rgb(0x0F, 0x18, 0x28);
/// Barre de titre des fenêtres.
pub const TITLE_BAR: Color = Color::rgb(0x13, 0x1E, 0x32);
/// Panneau latéral : navigation à gauche, aperçu à droite.
pub const SIDEBAR: Color = Color::rgb(0x11, 0x1B, 0x2E);
/// Carte dans une fenêtre.
pub const CARD: Color = Color::rgb(0x15, 0x22, 0x38);
/// Bloc de commande du terminal.
pub const TERMINAL_BLOCK: Color = Color::rgb(0x10, 0x1B, 0x2E);

// Verres. Le flou est l'affaire du compositeur : ici, seulement la teinte.

/// Panneaux flottants (flou de 24 px).
pub const GLASS: Color = Color::rgba(16, 26, 44, 0.82);
/// Menus et sous-menus (flou de 28 px).
pub const MENU_GLASS: Color = Color::rgba(22, 34, 56, 0.96);
/// Dock (flou de 30 px).
pub const DOCK_GLASS: Color = Color::rgba(244, 235, 227, 0.17);

// Traits et voiles.

/// Bordure d'un panneau de verre et d'une fenêtre, séparateur de menu.
pub const BORDER: Color = Color::rgba(255, 255, 255, 0.10);
/// Bordure d'un menu (planche Menus).
pub const MENU_BORDER: Color = Color::rgba(255, 255, 255, 0.14);
/// Bordure d'une carte (planche Réglages).
pub const CARD_BORDER: Color = Color::rgba(255, 255, 255, 0.09);
/// Bordure du dock.
pub const DOCK_BORDER: Color = Color::rgba(255, 255, 255, 0.26);
/// Bordure d'un bouton secondaire.
pub const SECONDARY_BORDER: Color = Color::rgba(255, 255, 255, 0.18);
/// Trait entre deux lignes d'une carte, bas de la barre de titre.
pub const DIVIDER: Color = Color::rgba(255, 255, 255, 0.07);
/// Fond d'un champ de texte.
pub const FIELD: Color = Color::rgba(255, 255, 255, 0.07);
/// Voile de survol sur un élément sans fond.
pub const HOVER: Color = Color::rgba(255, 255, 255, 0.07);
/// Voile d'appui sur un élément sans fond.
pub const PRESSED: Color = Color::rgba(255, 255, 255, 0.12);
/// Piste d'un interrupteur éteint, au repos et survolée. Les planches ne montrent que des
/// interrupteurs allumés : la piste éteinte reprend la bordure d'un bouton secondaire.
pub const TOGGLE_OFF: Color = Color::rgba(255, 255, 255, 0.18);
pub const TOGGLE_OFF_HOVER: Color = Color::rgba(255, 255, 255, 0.26);
/// Inset area of a toolbar: the breadcrumb (Evidence mock-up).
pub const INSET: Color = Color::rgba(255, 255, 255, 0.05);
/// Pill behind the window buttons.
pub const CONTROLS: Color = Color::rgba(255, 255, 255, 0.06);
/// Veil laid over a window behind a question awaiting an answer.
pub const SCRIM: Color = Color::rgba(3, 8, 18, 0.45);
/// Background of a status badge: warning, success, neutral (git status in the Evidence
/// mock-up), and border of a green badge without background (git branch).
pub const AMBER_SOFT: Color = Color::rgba(240, 196, 138, 0.18);
pub const ACCENT_SOFT: Color = Color::rgba(133, 197, 149, 0.18);
pub const NEUTRAL_SOFT: Color = Color::rgba(255, 255, 255, 0.08);
pub const ACCENT_BORDER: Color = Color::rgba(133, 197, 149, 0.5);
/// Fond d'un élément désactivé.
pub const DISABLED: Color = Color::rgba(255, 255, 255, 0.05);
/// Colour of the crisp shadows of floating surfaces (`surface::Shadowed`), the same blue
/// black as `SHADOW` and `MENU_SHADOW`; each step of the shadow sets its own opacity.
pub const SHADOW_BASE: Color = Color::rgb(3, 8, 18);
/// Ombre des fenêtres et des panneaux.
pub const SHADOW: Color = Color::rgba(3, 8, 18, 0.55);
/// Ombre des menus.
pub const MENU_SHADOW: Color = Color::rgba(3, 8, 18, 0.6);

// Textes.

/// Titres et contenu.
pub const TEXT_PRIMARY: Color = Color::rgb(0xEE, 0xF2, 0xF7);
/// Descriptions, valeurs.
pub const TEXT_SECONDARY: Color = Color::rgb(0xC5, 0xCE, 0xDB);
/// Étiquettes, raccourcis, texte d'un élément désactivé.
pub const TEXT_TERTIARY: Color = Color::rgb(0xA9, 0xB4, 0xC6);
/// Text of a code snippet or of a file preview (Evidence mock-up).
pub const TEXT_CODE: Color = Color::rgb(0xDC, 0xE4, 0xEE);
/// Numéros de ligne.
pub const TEXT_LINE_NUMBER: Color = Color::rgb(0x84, 0x92, 0xA8);
/// Texte posé sur le vert.
pub const TEXT_ON_ACCENT: Color = DEEPEST;
/// Texte posé sur le crème.
pub const TEXT_ON_CREAM: Color = Color::rgb(0x14, 0x23, 0x3B);

// Accents.

/// Vert de l'écharpe : action principale, état actif, réussite.
pub const ACCENT: Color = Color::rgb(0x85, 0xC5, 0x95);
/// Vert survolé.
pub const ACCENT_HOVER: Color = ACCENT.mix(TEXT_PRIMARY, 0.18);
/// Vert appuyé.
pub const ACCENT_PRESSED: Color = ACCENT.mix(DEEPEST, 0.14);
/// Bleu des icônes et des dossiers, le plus sombre.
pub const BLUE_DARK: Color = Color::rgb(0x2B, 0x5F, 0x86);
/// Bleu des icônes et des dossiers.
pub const BLUE: Color = Color::rgb(0x32, 0x6D, 0x93);
/// Bleu de la sélection et des mots-clés du code.
pub const BLUE_BRIGHT: Color = Color::rgb(0x5B, 0xA3, 0xD0);
/// Bleu clair des mots-clés du code.
pub const BLUE_LIGHT: Color = Color::rgb(0x8F, 0xC1, 0xE8);
/// Ce qui vient de Sherlock ou du système de confiance.
pub const CREAM: Color = Color::rgb(0xF4, 0xEB, 0xE3);
/// Attente d'un accord, avertissement.
pub const AMBER: Color = Color::rgb(0xF0, 0xC4, 0x8A);
/// Erreur, action destructive.
pub const SALMON: Color = Color::rgb(0xF6, 0xB5, 0xA6);

/// Ligne sélectionnée d'une liste ou d'un menu, entrée de menu survolée.
pub const SELECTED_ROW: Color = Color::rgba(91, 163, 208, 0.26);
/// Fond de l'entrée active d'une navigation.
pub const ACTIVE_ENTRY: Color = Color::rgba(133, 197, 149, 0.16);
/// Anneau du focus clavier. Les planches n'en montrent pas : bleu clair, visible sur tous les
/// fonds et distinct du vert de l'état actif.
pub const FOCUS_RING: Color = BLUE_LIGHT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_couleur_opaque_garde_ses_composantes() {
        assert_eq!(Color::rgb(255, 0, 51).premultiplied(), [1.0, 0.0, 0.2, 1.0]);
    }

    #[test]
    fn une_couleur_translucide_est_multipliee_par_son_opacite() {
        let [r, g, b, a] = Color::rgba(255, 255, 0, 0.5).premultiplied();
        assert_eq!((r, g, b, a), (0.5, 0.5, 0.0, 0.5));
    }

    #[test]
    fn le_contraste_suit_les_wcag() {
        let noir = Color::rgb(0, 0, 0);
        let blanc = Color::rgb(255, 255, 255);
        assert!((noir.contrast(blanc) - 21.0).abs() < 0.01);
        assert!((blanc.contrast(blanc) - 1.0).abs() < f32::EPSILON);
        // Valeur de référence : #777777 sur blanc donne 4,48.
        assert!((Color::rgb(0x77, 0x77, 0x77).contrast(blanc) - 4.48).abs() < 0.01);
    }

    #[test]
    fn un_voile_se_pose_comme_dans_le_navigateur() {
        let posee = Color::rgba(255, 255, 255, 0.5).over(Color::rgb(0, 0, 0));
        assert_eq!(posee, Color::rgb(128, 128, 128));
        assert_eq!(GLASS.over(DESKTOP).a, 1.0);
    }

    #[test]
    fn les_nuances_restent_proches_du_vert() {
        assert_eq!(ACCENT.mix(DEEPEST, 0.0), ACCENT);
        assert!(ACCENT_HOVER.luminance() > ACCENT.luminance());
        assert!(ACCENT_PRESSED.luminance() < ACCENT.luminance());
    }
}
