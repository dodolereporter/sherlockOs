//! The projection chooser (lot D-39): Super + P, a small panel in the middle of the screen with
//! four choices, as on Windows and macOS. Drawn by the compositor. No board draws it yet: the
//! measures are choices of the lot, to review with the design.

use std::time::Duration;

use crate::color::{Color, Palette};

/// A choice: its tile, the picture of two screens in it, the gap between tiles.
pub const TILE_WIDTH: f32 = 132.0;
pub const TILE_HEIGHT: f32 = 112.0;
pub const TILE_GAP: f32 = 12.0;
pub const TILE_RADIUS: f32 = 12.0;
/// The two screens drawn in a tile, and the label under them.
pub const PICTURE_WIDTH: f32 = 40.0;
pub const PICTURE_HEIGHT: f32 = 28.0;
pub const PICTURE_GAP: f32 = 6.0;
pub const PICTURE_RADIUS: f32 = 4.0;
pub const LABEL_HEIGHT: f32 = 36.0;
/// The panel: its padding, radius, title and the line of Do not disturb under the tiles.
pub const PADDING: f32 = 20.0;
pub const RADIUS: f32 = 18.0;
pub const TITLE_HEIGHT: f32 = 24.0;
pub const OPTION_HEIGHT: f32 = 24.0;
pub const GAP: f32 = 14.0;
/// The panel fades and grows in this long; the highlight slides from tile to tile this long.
pub const APPEAR: Duration = Duration::from_millis(160);
pub const SLIDE: Duration = Duration::from_millis(140);
/// The scale the panel grows from.
pub const APPEAR_SCALE: f32 = 0.94;

pub mod text {
    pub const TITLE: &str = "Projeter";
    pub const LAPTOP: &str = "Écran du portable seul";
    pub const DUPLICATE: &str = "Dupliquer";
    pub const EXTEND: &str = "Étendre";
    pub const SECOND: &str = "Second écran seul";
    pub const DO_NOT_DISTURB_ON: &str = "Ne pas déranger pendant la projection : oui (Espace)";
    pub const DO_NOT_DISTURB_OFF: &str = "Ne pas déranger pendant la projection : non (Espace)";
}

/// The colors of the panel in a palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    pub panel: Color,
    pub border: Color,
    pub text: Color,
    pub secondary: Color,
    pub tile: Color,
    pub selected: Color,
    pub on_selected: Color,
    /// A screen lit in a picture, and one off.
    pub lit: Color,
    pub off: Color,
}

impl Colors {
    pub const fn of(palette: &Palette) -> Self {
        Self {
            panel: palette.window,
            border: palette.menu_border,
            text: palette.text_primary,
            secondary: palette.text_secondary,
            tile: palette.card,
            selected: palette.accent,
            on_selected: palette.text_on_accent,
            lit: palette.text_secondary,
            off: palette.divider,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_reads_in_both_themes() {
        for p in [&Palette::DARK, &Palette::LIGHT] {
            let c = Colors::of(p);
            assert!(c.text.contrast(c.panel) >= 4.5, "{p:?}");
            assert!(c.text.contrast(c.tile) >= 4.5, "{p:?}");
            assert!(c.on_selected.contrast(c.selected) >= 4.5, "{p:?}");
            assert!(c.secondary.contrast(c.panel) >= 4.5, "{p:?}");
        }
    }
}
