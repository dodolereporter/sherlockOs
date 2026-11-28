//! Screen sharing (lot D-28d): the picker where the user chooses the screen or the window an
//! application will see, as macOS asks it, and the indicator of the bar while something is
//! shared. No board draws them: the measures below are choices of the lot, taken from the panel
//! and Mission Control tokens, to review with the design. See `docs/portal.md`.

use crate::motion::{self, Spring};

/// Width of the panel on a large screen; it never takes more than `WIDTH_SHARE` of the screen.
pub const WIDTH: f32 = 860.0;
pub const WIDTH_SHARE: f32 = 0.9;
/// Height of the panel at most, as a share of the screen: the grid scrolls beyond.
pub const HEIGHT_SHARE: f32 = 0.86;
/// Space inside the panel, and between its parts.
pub const PADDING: f32 = 24.0;
pub const GAP: f32 = 16.0;
/// A thumbnail: its box, 16 by 9, and its radius; the picture fits in it, centred.
pub const THUMB_WIDTH: f32 = 224.0;
pub const THUMB_HEIGHT: f32 = 126.0;
pub const THUMB_RADIUS: f32 = 10.0;
/// The thumbnails are taken at most this wide, in pixels: sharp at scale 2.
pub const THUMB_PIXELS: u32 = 448;
/// Space around the content of a tile, and between tiles.
pub const TILE_PADDING: f32 = 8.0;
pub const TILE_GAP: f32 = 12.0;
/// The chosen tile's ring.
pub const RING: f32 = 3.0;

/// Opening and closing: the panel grows from `OPEN_SCALE` and fades in on the spring of the
/// panels; it shrinks back and fades out to close.
pub const OPEN_SPRING: Spring = motion::OPEN_SPRING;
pub const OPEN_SCALE: f32 = motion::OPEN_SCALE as f32;
pub const OPEN_PRECISION: f64 = 0.01;

/// The bar's indicator grows from this scale as it comes, and back as it goes.
pub const INDICATOR_SCALE: f32 = 0.9;

/// Texts.
pub mod text {
    /// The title: who asks.
    pub fn title(app: &str) -> String {
        format!("Que veux-tu partager avec {app} ?")
    }
    /// When the application gives no name (an X11 application).
    pub const SOME_APP: &str = "cette application";
    pub const SCREENS: &str = "Écran";
    pub const WINDOWS: &str = "Fenêtre";
    pub const CANCEL: &str = "Annuler";
    pub const SHARE: &str = "Partager";
    pub const NOTHING: &str = "Rien à partager";
    /// A screen without a description.
    pub fn screen(number: usize) -> String {
        format!("Écran {number}")
    }
    /// A window without a title.
    pub const UNTITLED: &str = "Sans titre";
    /// The indicator of the bar: a dot, and what a click on it does.
    pub const DOT: &str = "●";
    pub const STOP: &str = "Arrêter le partage";
}

/// The name shown for the application `app_id`: the last part of a reverse domain name
/// (`com.discordapp.Discord` gives `Discord`), the id itself otherwise; `None` without one.
pub fn app_name(app_id: &str) -> Option<&str> {
    let id = app_id.trim().trim_end_matches(".desktop");
    let name = id.rsplit('.').next().unwrap_or(id);
    (!name.is_empty()).then_some(name)
}

/// The width of the panel on a screen `screen` logical pixels wide.
pub fn width(screen: f32) -> f32 {
    WIDTH
        .min(screen * WIDTH_SHARE)
        .max(THUMB_WIDTH + 2.0 * (PADDING + TILE_PADDING))
}

/// How many tiles a row of the panel `width` wide holds: at least one.
pub fn columns(width: f32) -> usize {
    let inner = width - 2.0 * PADDING + TILE_GAP;
    let tile = THUMB_WIDTH + 2.0 * TILE_PADDING + TILE_GAP;
    ((inner / tile).floor() as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_of_an_application_is_its_last_part() {
        assert_eq!(app_name("com.discordapp.Discord"), Some("Discord"));
        assert_eq!(app_name("firefox"), Some("firefox"));
        assert_eq!(app_name("org.chromium.Chromium.desktop"), Some("Chromium"));
        assert_eq!(app_name(""), None);
    }

    #[test]
    fn the_panel_fits_every_screen() {
        // 1920 wide: the whole panel, three tiles a row.
        assert_eq!(width(1920.0), WIDTH);
        assert_eq!(columns(width(1920.0)), 3);
        // A small screen: narrower, at least one tile.
        let small = width(320.0);
        assert!(small <= 320.0, "{small}");
        assert_eq!(columns(small), 1);
    }
}
