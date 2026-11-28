//! Fonts and text sizes. The three fonts are embedded, under the OFL license: see `fonts/`
//! and `docs/sherlock-ui.md`.

/// Interface family: Inter, variable version (internal name "Inter Variable").
pub const UI_FAMILY: &str = "Inter Variable";
/// Title family.
pub const TITLE_FAMILY: &str = "Space Grotesk";
/// Family of code and the terminal.
pub const CODE_FAMILY: &str = "JetBrains Mono";

/// Font files, to load at the start of each program.
pub const FONTS: [&[u8]; 3] = [
    include_bytes!("../fonts/InterVariable.ttf"),
    include_bytes!("../fonts/SpaceGrotesk-Variable.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
];

/// Interface text.
pub const UI_SIZE: f32 = 13.0;
/// Interface text on the system screens, from 14 to 15.
pub const SYSTEM_SIZE: f32 = 15.0;
/// Shortcuts, short descriptions.
pub const SMALL_SIZE: f32 = 12.0;
/// Description under a card title.
pub const DESCRIPTION_SIZE: f32 = 12.5;
/// Card title, page subtitle.
pub const CARD_TITLE_SIZE: f32 = 15.0;
/// Titles in Space Grotesk: small, page, large system screen. The range goes from 17 to 44.
pub const TITLE_SMALL_SIZE: f32 = 17.0;
pub const TITLE_SIZE: f32 = 26.0;
pub const TITLE_LARGE_SIZE: f32 = 44.0;
/// Code and terminal, from 12 to 12.5, on a line height of 20.
pub const CODE_SIZE: f32 = 12.5;
pub const CODE_LINE_HEIGHT: f32 = 20.0;
/// Code in a narrow panel: file preview, branch badge.
pub const CODE_SMALL_SIZE: f32 = 12.0;
/// Column headers of a listing and status badges, uppercase for the former.
pub const COLUMN_HEADER_SIZE: f32 = 11.5;
pub const BADGE_SIZE: f32 = 11.5;
/// Section label, from 11 to 11.5, in uppercase.
pub const SECTION_LABEL_SIZE: f32 = 11.0;
/// Letter spacing of the section label, as a share of the size. iced 0.14 cannot apply it:
/// see the known gap in `docs/sherlock-ui.md`.
pub const SECTION_LABEL_TRACKING: f32 = 0.06;
/// Average advance of a character of Inter at the interface size, to estimate how much of
/// a name fits in a column without shaping it.
pub const UI_CHAR_WIDTH: f32 = 7.2;
/// Line height of body text, as a share of the size.
pub const LINE_HEIGHT: f32 = 1.4;

/// Weights in use: regular, medium (setting label), semibold (titles, labels).
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

    /// Interface, regular weight.
    pub const UI: Font = font(super::UI_FAMILY, Weight::Normal);
    /// Interface, medium weight.
    pub const UI_MEDIUM: Font = font(super::UI_FAMILY, Weight::Medium);
    /// Interface, semibold.
    pub const UI_SEMIBOLD: Font = font(super::UI_FAMILY, Weight::Semibold);
    /// Titles, semibold.
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

/// A text size the text engine takes: cosmic-text panics on a line height of 0, so on a size
/// of 0 (lot Q-32). Not a number gives a pixel too. Here, outside the `iced` feature, for the
/// tokens the compositor reads too (`apps`, lot Q-53).
pub fn at_least_a_pixel(size: f32) -> f32 {
    size.max(1.0)
}
