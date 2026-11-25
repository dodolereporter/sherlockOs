//! Polices et tailles de texte. Les trois polices sont embarquées, sous licence OFL : voir
//! `polices/` et `docs/sherlock-ui.md`.

/// Famille de l'interface : Inter, version variable (nom interne « Inter Variable »).
pub const UI_FAMILY: &str = "Inter Variable";
/// Famille des titres.
pub const TITLE_FAMILY: &str = "Space Grotesk";
/// Famille du code et du terminal.
pub const CODE_FAMILY: &str = "JetBrains Mono";

/// Fichiers des polices, à charger au démarrage de chaque programme.
pub const FONTS: [&[u8]; 3] = [
    include_bytes!("../polices/InterVariable.ttf"),
    include_bytes!("../polices/SpaceGrotesk-Variable.ttf"),
    include_bytes!("../polices/JetBrainsMono-Regular.ttf"),
];

/// Texte de l'interface.
pub const UI_SIZE: f32 = 13.0;
/// Texte de l'interface sur les écrans système, de 14 à 15.
pub const SYSTEM_SIZE: f32 = 15.0;
/// Raccourcis, descriptions courtes.
pub const SMALL_SIZE: f32 = 12.0;
/// Description sous un titre de carte.
pub const DESCRIPTION_SIZE: f32 = 12.5;
/// Titre de carte, sous-titre de page.
pub const CARD_TITLE_SIZE: f32 = 15.0;
/// Titres en Space Grotesk : petit, page, grand écran système. La gamme va de 17 à 44.
pub const TITLE_SMALL_SIZE: f32 = 17.0;
pub const TITLE_SIZE: f32 = 26.0;
pub const TITLE_LARGE_SIZE: f32 = 44.0;
/// Code et terminal, de 12 à 12,5, sur un interligne de 20.
pub const CODE_SIZE: f32 = 12.5;
pub const CODE_LINE_HEIGHT: f32 = 20.0;
/// Code in a narrow panel: file preview, branch badge.
pub const CODE_SMALL_SIZE: f32 = 12.0;
/// Column headers of a listing and status badges, uppercase for the former.
pub const COLUMN_HEADER_SIZE: f32 = 11.5;
pub const BADGE_SIZE: f32 = 11.5;
/// Étiquette de section, de 11 à 11,5, en majuscules.
pub const SECTION_LABEL_SIZE: f32 = 11.0;
/// Interlettrage de l'étiquette de section, en part de la taille. iced 0.14 ne sait pas
/// l'appliquer : voir l'écart connu dans `docs/sherlock-ui.md`.
pub const SECTION_LABEL_TRACKING: f32 = 0.06;
/// Average advance of a character of Inter at the interface size, to estimate how much of
/// a name fits in a column without shaping it.
pub const UI_CHAR_WIDTH: f32 = 7.2;
/// Interligne du texte courant, en part de la taille.
pub const LINE_HEIGHT: f32 = 1.4;

/// Graisses utilisées : normale, moyenne (libellé de réglage), demi-grasse (titres, étiquettes).
pub const WEIGHT_REGULAR: u16 = 400;
pub const WEIGHT_MEDIUM: u16 = 500;
pub const WEIGHT_SEMIBOLD: u16 = 600;

#[cfg(feature = "iced")]
pub use fonts::*;

#[cfg(feature = "iced")]
mod fonts {
    use iced::Font;
    use iced::font::{Family, Weight};

    const fn font(family: &'static str, weight: Weight) -> Font {
        Font {
            family: Family::Name(family),
            weight,
            ..Font::DEFAULT
        }
    }

    /// Interface, graisse normale.
    pub const UI: Font = font(super::UI_FAMILY, Weight::Normal);
    /// Interface, graisse moyenne.
    pub const UI_MEDIUM: Font = font(super::UI_FAMILY, Weight::Medium);
    /// Interface, demi-grasse.
    pub const UI_SEMIBOLD: Font = font(super::UI_FAMILY, Weight::Semibold);
    /// Titres, demi-gras.
    pub const TITLE: Font = font(super::TITLE_FAMILY, Weight::Semibold);
    /// Code.
    pub const CODE: Font = font(super::CODE_FAMILY, Weight::Normal);
}

const _: () = {
    assert!(TITLE_SMALL_SIZE >= 17.0 && TITLE_LARGE_SIZE <= 44.0);
    assert!(CODE_SIZE >= 12.0 && CODE_SIZE <= 12.5);
    assert!(CODE_SMALL_SIZE >= 12.0 && CODE_SMALL_SIZE <= CODE_SIZE);
    assert!(SECTION_LABEL_SIZE >= 11.0 && SECTION_LABEL_SIZE <= 11.5);
    assert!(SYSTEM_SIZE >= 14.0 && SYSTEM_SIZE <= 15.0);
};
