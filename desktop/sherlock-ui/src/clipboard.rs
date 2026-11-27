//! Clipboard history panel (lot D-42): the tokens and texts of the Super + V panel. It is the
//! launcher's glass, search row and rows (`crate::launcher`), with taller rows for two lines of
//! preview and a footer.

use crate::launcher;

// Geometry, in logical pixels.

/// Width of the panel.
pub const WIDTH: f32 = 560.0;
/// Rows shown at once; the list scrolls past them.
pub const VISIBLE_ROWS: usize = 5;
/// A row: its height, the side of its tile or thumbnail, the gap between its two lines.
pub const ROW_HEIGHT: f32 = 64.0;
pub const TILE: f32 = 44.0;
pub const THUMBNAIL_RADIUS: f32 = 6.0;
/// The footer: its height and text size.
pub const FOOTER_HEIGHT: f32 = 36.0;
pub const FOOTER_SIZE: f32 = 11.5;
/// The pin and remove buttons of the selected row: their side and icon.
pub const ACTION: f32 = 26.0;
pub const ACTION_ICON: f32 = 14.0;
/// Room kept between the panel and the edges of a small screen.
pub const SCREEN_MARGIN: f32 = 24.0;
/// Lines of the preview at most.
pub const PREVIEW_LINES: usize = 2;
/// Scale the panel opens from and closes to, around its middle.
pub const OPEN_SCALE: f32 = launcher::OPEN_SCALE;
pub const OPEN_SPRING: crate::motion::Spring = launcher::OPEN_SPRING;
pub const OPEN_PRECISION: f64 = launcher::OPEN_PRECISION;

/// Height of the panel showing `rows` rows (`VISIBLE_ROWS` at most, one for a message): its
/// border and padding, the search row and the line under it, the rows, the footer.
pub fn panel_height(rows: usize) -> f32 {
    let border = crate::metrics::BORDER_WIDTH;
    2.0 * (border + launcher::PADDING)
        + launcher::SEARCH_HEIGHT
        + border
        + rows.clamp(1, VISIBLE_ROWS) as f32 * ROW_HEIGHT
        + FOOTER_HEIGHT
}

/// Rows that fit on a screen `height` high, at most `VISIBLE_ROWS`, at least one.
pub fn rows_for_screen(height: f32) -> usize {
    let fixed = panel_height(1) - ROW_HEIGHT + 2.0 * SCREEN_MARGIN;
    let rows = ((height - fixed) / ROW_HEIGHT).floor();
    (rows.max(1.0) as usize).min(VISIBLE_ROWS)
}

/// What an item is, for its tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Text,
    Link,
    Files,
    Image,
    Pin,
    Remove,
}

impl Glyph {
    pub const ALL: [Self; 6] = [
        Self::Text,
        Self::Link,
        Self::Files,
        Self::Image,
        Self::Pin,
        Self::Remove,
    ];

    /// Path on the 24 grid.
    pub const fn path(self) -> &'static str {
        match self {
            Self::Text => "M5 6h14M5 10h14M5 14h10M5 18h7",
            Self::Link => {
                "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"
            }
            Self::Files => crate::icon::Icon::Folder.path(),
            Self::Image => crate::icon::Icon::Image.path(),
            Self::Pin => "M9 4h6M10 4v6l-3 4h10l-3-4V4M12 14v6",
            Self::Remove => crate::icon::Icon::Close.path(),
        }
    }

    /// Full SVG document, stroked in black: the display recolors it.
    pub fn svg(self) -> String {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {grid} {grid}\" fill=\"none\" \
             stroke=\"#000\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>",
            grid = crate::metrics::ICON_GRID,
            stroke = crate::metrics::ICON_STROKE,
            path = self.path(),
        )
    }
}

/// When an item was copied, `seconds` ago, as the panel says it.
pub fn age(seconds: u64) -> String {
    match seconds {
        0..60 => String::from(text::NOW),
        60..3600 => format!("il y a {} min", seconds / 60),
        3600..86400 => format!("il y a {} h", seconds / 3600),
        _ => format!("il y a {} j", seconds / 86400),
    }
}

/// Panel texts, in French.
pub mod text {
    pub const PLACEHOLDER: &str = "Rechercher dans le presse-papiers";
    /// Nothing copied yet, nothing found, the history turned off.
    pub const EMPTY: &str = "Rien n'a encore été copié";
    pub const NOTHING: &str = "Aucun résultat";
    pub const OFF: &str = "L'historique du presse-papiers est désactivé";
    pub const OFF_LINE: &str = "Active-le dans Control Room, page Presse-papiers";
    /// The compositor does not offer the history: another compositor.
    pub const UNAVAILABLE: &str = "Historique indisponible dans cette session";
    pub const CLEAR: &str = "Tout effacer";
    /// The footer: what Enter does (ADR 0030).
    pub const FOOTER: &str = "Entrée pour copier, puis Ctrl + V pour coller";
    pub const PIN: &str = "Épingler";
    pub const UNPIN: &str = "Désépingler";
    pub const REMOVE: &str = "Retirer (Suppr)";
    /// The origin of a screenshot, which no application copied.
    pub const SCREENSHOT: &str = "Capture d'écran";
    pub const NOW: &str = "à l'instant";
    /// Between the application and the time.
    pub const SEPARATOR: &str = " · ";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_fits_small_screens() {
        assert_eq!(rows_for_screen(1080.0), VISIBLE_ROWS);
        // A 1366 × 768 screen at scale 1.5 is 512 high.
        let rows = rows_for_screen(512.0);
        assert!(panel_height(rows) + 2.0 * SCREEN_MARGIN <= 512.0);
        assert_eq!(rows_for_screen(100.0), 1);
        assert_eq!(panel_height(0), panel_height(1));
    }

    #[test]
    fn ages_read_naturally() {
        assert_eq!(age(5), "à l'instant");
        assert_eq!(age(150), "il y a 2 min");
        assert_eq!(age(7200), "il y a 2 h");
        assert_eq!(age(3 * 86400), "il y a 3 j");
    }

    #[test]
    fn the_glyphs_are_svg() {
        for glyph in Glyph::ALL {
            assert!(glyph.svg().contains(glyph.path()));
        }
    }
}
