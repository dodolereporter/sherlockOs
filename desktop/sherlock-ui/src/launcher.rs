//! Launcher (batches D-22a and D-22b): the tokens, styles and texts of the application
//! launcher. Values from the launcher of the Panels design board,
//! `docs/design/planches/Panneaux.dc.html`, and of its light version.

use crate::color::{self, Color, Palette};

// Geometry, in logical pixels.

/// Width of the panel, and its distance from the top of the screen.
pub const WIDTH: f32 = 600.0;
pub const TOP: f32 = 150.0;
/// Inner padding and radius of the panel.
pub const PADDING: f32 = 8.0;
pub const RADIUS: f32 = crate::metrics::PANEL_RADIUS;
/// Search row: height, side padding, gap between its parts, magnifier side, query text size.
pub const SEARCH_HEIGHT: f32 = 52.0;
pub const SEARCH_PADDING: f32 = 10.0;
pub const GAP: f32 = 12.0;
pub const SEARCH_ICON: f32 = 20.0;
pub const QUERY_SIZE: f32 = 18.0;
/// Key cap (« Échap », « Entrée »): vertical and side padding, radius, text size.
pub const KEY_PADDING: (f32, f32) = (3.0, 8.0);
pub const KEY_PADDING_ROW: (f32, f32) = (2.0, 7.0);
pub const KEY_RADIUS: f32 = 6.0;
pub const KEY_SIZE: f32 = 11.5;
/// Section label above the results: top, sides, bottom padding.
pub const SECTION_PADDING: (f32, f32, f32) = (10.0, 10.0, 4.0);
/// Result row: height, side padding, radius; its tile, the tile radius, the initial in it.
pub const ROW_HEIGHT: f32 = 46.0;
pub const ROW_PADDING: f32 = 10.0;
pub const ROW_RADIUS: f32 = 10.0;
pub const TILE: f32 = 30.0;
pub const TILE_RADIUS: f32 = 8.0;
pub const TILE_LETTER_SIZE: f32 = 14.0;
/// Side of an application icon drawn on a tile, and the glyph drawn on a colored tile.
pub const APP_ICON: f32 = TILE;
pub const TILE_GLYPH: f32 = 16.0;
/// Scale application icons are looked up for: sharp on a scale 2 screen, scaled down on the
/// others.
pub const ICON_SCALE: u32 = 2;
/// Results shown at most, all sections together: with the five sections, the panel stays
/// clear of the dock on the 900 high reference screen.
pub const MAX_RESULTS: usize = 9;
/// Results of one section at most.
pub const SECTION_RESULTS: usize = 5;
/// Sections at most: calculator, applications, system, files, Sherlock.
pub const MAX_SECTIONS: usize = 5;
/// Least time between arming a session command (« Se déconnecter », « Redémarrer »,
/// « Éteindre ») and confirming it: longer than a double click and than a key bounce.
pub const CONFIRM_DELAY: std::time::Duration = std::time::Duration::from_millis(400);
/// Width of the menu of an application (« Garder dans le Dock »), drawn inside its row.
pub const MENU_WIDTH: f32 = 200.0;
/// Scale the panel opens from and closes to, around the middle of its top edge, as Spotlight.
pub const OPEN_SCALE: f32 = 0.96;
/// Spring of the opening and the closing: the panel's opacity and scale (lot P-02).
pub const OPEN_SPRING: crate::motion::Spring = crate::motion::OPEN_SPRING;
/// How close to its end the opening or the closing counts as done.
pub const OPEN_PRECISION: f64 = 0.01;
/// The results moving to their new place, the panel's height following them, the selection
/// sliding (lot P-02d).
pub const LIST_SPRING: crate::motion::Spring = crate::motion::LIST_SPRING;
/// How close to their place, in pixels, the moving rows and the panel's edge count as there.
pub const LIST_PRECISION: f64 = 0.5;
/// A new result fades in over this time, coming down this far to its place.
pub const APPEAR: std::time::Duration = crate::motion::LIST_APPEAR;
pub const APPEAR_OFFSET: f32 = 8.0;
/// Height of a section label, its padding included.
pub fn label_height() -> f32 {
    let (top, _, bottom) = SECTION_PADDING;
    top + crate::typography::SECTION_LABEL_SIZE * crate::typography::LINE_HEIGHT + bottom
}

/// Height of the panel holding `sections` section labels and `rows` result rows: its border
/// and padding, the search row and the line under it, the labels, the rows.
pub fn panel_height(sections: usize, rows: usize) -> f32 {
    let border = crate::metrics::BORDER_WIDTH;
    2.0 * (border + PADDING)
        + SEARCH_HEIGHT
        + border
        + sections as f32 * label_height()
        + rows as f32 * ROW_HEIGHT
}

// Colors.

/// The launcher's own colors, for one theme; the rest comes from the palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Glass of the panel (`0.96`: nearly opaque until the compositor blurs what is behind,
    /// lot D-37, so that a busy wallpaper never makes the results hard to read; UI-03) and
    /// its border.
    pub panel: Color,
    pub border: Color,
    /// Line under the search row.
    pub divider: Color,
    /// Key cap of the search row, then of the selected row.
    pub key: Color,
    pub key_row: Color,
    /// Tile of an application, and its initial (the « Conteneurs » tile of the board).
    pub tile: Color,
    pub tile_letter: Color,
    /// Tile of a system command (the « Réglages » tile of the board) and of a file (the
    /// board's « Fichiers » tiles); their glyph is cream.
    pub system_tile: Color,
    pub file_tile: Color,
    pub tile_glyph: Color,
}

impl Colors {
    pub const DARK: Self = Self {
        panel: Color::rgba(16, 26, 44, 0.96),
        border: Color::rgba(255, 255, 255, 0.12),
        divider: Color::rgba(255, 255, 255, 0.08),
        key: Color::rgba(255, 255, 255, 0.10),
        key_row: Color::rgba(255, 255, 255, 0.12),
        tile: color::BLUE_BRIGHT,
        tile_letter: color::DEEPEST,
        system_tile: color::TILE_SETTINGS,
        file_tile: color::TILE_FILES,
        tile_glyph: color::CREAM,
    };

    /// The light board: white glass, night blue veils at the same opacity.
    pub const LIGHT: Self = Self {
        panel: Color::rgba(255, 255, 255, 0.96),
        border: Color::rgba(20, 35, 59, 0.12),
        divider: Color::rgba(20, 35, 59, 0.08),
        key: Color::rgba(20, 35, 59, 0.10),
        key_row: Color::rgba(20, 35, 59, 0.12),
        tile: color::BLUE_BRIGHT,
        tile_letter: color::DEEPEST,
        system_tile: color::TILE_SETTINGS,
        file_tile: color::TILE_FILES,
        tile_glyph: color::CREAM,
    };

    /// The colors that go with `palette`.
    pub fn of(palette: &Palette) -> &'static Self {
        if *palette == Palette::LIGHT {
            &Self::LIGHT
        } else {
            &Self::DARK
        }
    }
}

/// Launcher texts, in French.
pub mod text {
    /// Placeholder of the search field (the board's hidden label).
    pub const PLACEHOLDER: &str = "Rechercher ou lancer";
    pub const ESCAPE: &str = "Échap";
    pub const ENTER: &str = "Entrée";
    /// Section of the installed applications.
    pub const APPLICATIONS: &str = "Applications";
    /// Shown when nothing matches. The board has no such state.
    pub const NOTHING: &str = "Aucun résultat";

    /// Sections, in the order of the panel. The board has neither the calculator nor the
    /// system commands.
    pub const CALCULATOR: &str = "Calculatrice";
    pub const SYSTEM: &str = "Système";
    pub const FILES: &str = "Fichiers";
    pub const SHERLOCK: &str = "Sherlock";

    /// The calculator's line, then the line once the result is copied.
    pub const COPY: &str = "Entrée pour copier le résultat";
    pub const COPIED: &str = "Copié";

    /// System commands: title, line, and the words that find them (folded, without accents).
    pub const LOCK: &str = "Verrouiller";
    pub const LOCK_LINE: &str = "Verrouille ta session";
    pub const LOCK_WORDS: [&str; 3] = ["verrouiller", "verrou", "lock"];
    pub const LOG_OUT: &str = "Se déconnecter";
    pub const LOG_OUT_LINE: &str = "Ferme ta session";
    pub const LOG_OUT_WORDS: [&str; 4] = ["deconnexion", "deconnecter", "logout", "quitter"];
    pub const REBOOT: &str = "Redémarrer";
    pub const REBOOT_LINE: &str = "Redémarre l'ordinateur";
    pub const REBOOT_WORDS: [&str; 3] = ["redemarrer", "reboot", "restart"];
    pub const POWER_OFF: &str = "Éteindre";
    pub const POWER_OFF_LINE: &str = "Éteins l'ordinateur";
    pub const POWER_OFF_WORDS: [&str; 4] = ["eteindre", "arreter", "shutdown", "poweroff"];
    /// The line of « Se déconnecter », « Redémarrer » and « Éteindre » after a first Enter.
    pub const CONFIRM: &str = "Appuie encore sur Entrée pour confirmer";

    /// A file right in the personal folder: the line under its name.
    pub const HOME: &str = "Dossier personnel";

    /// The last entry, its line from the board, and its line while sherlockd cannot take a
    /// request from the shell.
    pub const ASK: &str = "Demander à Sherlock";
    pub const ASK_LINE: &str = "Tab pour lui passer la recherche";
    pub const SOON: &str = "Bientôt";
}

/// Line glyphs of the result tiles, on the grid and stroke of `crate::icon`. The board draws
/// the file one; the others follow its stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Calculator,
    Lock,
    LogOut,
    Reboot,
    PowerOff,
    File,
    Folder,
}

impl Glyph {
    pub const ALL: [Self; 7] = [
        Self::Calculator,
        Self::Lock,
        Self::LogOut,
        Self::Reboot,
        Self::PowerOff,
        Self::File,
        Self::Folder,
    ];

    /// Path on the 24 grid.
    pub const fn path(self) -> &'static str {
        match self {
            Self::Calculator => {
                "M6 3h12v18H6zM9 7h6M9 12h.01M12 12h.01M15 12h.01M9 16h.01M12 16h.01M15 16h.01"
            }
            Self::Lock => crate::icon::Icon::Lock.path(),
            Self::LogOut => "M10 4H5v16h5M14 8l4 4-4 4M18 12H9",
            Self::Reboot => "M19 12a7 7 0 1 1-2.05-4.95M19 4v4h-4",
            // The « Éteindre » button of the board's quick settings.
            Self::PowerOff => "M12 4v8M7.5 7a7 7 0 1 0 9 0",
            Self::File => crate::icon::Icon::Document.path(),
            Self::Folder => crate::icon::Icon::Folder.path(),
        }
    }

    /// Whether it sits on the system tile (else on the file tile, or the application tile for
    /// the calculator).
    pub const fn is_system(self) -> bool {
        matches!(
            self,
            Self::Lock | Self::LogOut | Self::Reboot | Self::PowerOff
        )
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

impl Colors {
    /// Tile behind a glyph, and the glyph's color.
    pub fn glyph_tile(&self, glyph: Glyph) -> (Color, Color) {
        match glyph {
            Glyph::Calculator => (self.tile, self.tile_letter),
            g if g.is_system() => (self.system_tile, self.tile_glyph),
            _ => (self.file_tile, self.tile_glyph),
        }
    }
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::{button, container};
    use iced::{Background, Border, Theme, border};

    use super::*;
    use crate::{metrics, theme};

    fn fill(c: Color, edge: Border) -> container::Style {
        container::Style {
            background: Some(Background::Color(c.into())),
            border: edge,
            ..container::Style::default()
        }
    }

    fn colors(theme: &Theme) -> &'static Colors {
        Colors::of(theme::palette(theme))
    }

    /// The panel: dense glass, light border, radius 20. Blur and shadow are the compositor's
    /// job, as for the dock.
    pub fn panel(theme: &Theme) -> container::Style {
        let c = colors(theme);
        let edge = border::width(metrics::BORDER_WIDTH)
            .color(c.border)
            .rounded(RADIUS);
        container::Style {
            text_color: Some(theme::palette(theme).text_primary.into()),
            ..fill(c.panel, edge)
        }
    }

    /// The line under the search row.
    pub fn divider(theme: &Theme) -> container::Style {
        fill(colors(theme).divider, Border::default())
    }

    /// Key cap: on the search row, or on the selected result (`in_row`).
    pub fn key(in_row: bool) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let c = colors(theme);
            let tint = if in_row { c.key_row } else { c.key };
            container::Style {
                text_color: Some(theme::palette(theme).text_secondary.into()),
                ..fill(tint, border::rounded(KEY_RADIUS))
            }
        }
    }

    /// Tile of an application, with its initial.
    pub fn tile(theme: &Theme) -> container::Style {
        let c = colors(theme);
        container::Style {
            text_color: Some(c.tile_letter.into()),
            ..fill(c.tile, border::rounded(TILE_RADIUS))
        }
    }

    /// A result row: the selection blue when it is selected, a veil under the pointer.
    /// The selection, a veil that slides under the rows (lot P-02d).
    pub fn selection(theme: &Theme) -> container::Style {
        let p = theme::palette(theme);
        container::Style {
            background: Some(Background::Color(p.selected_row.into())),
            border: border::rounded(ROW_RADIUS),
            ..container::Style::default()
        }
    }

    pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |theme, status| {
            let p = theme::palette(theme);
            let tint = match status {
                _ if selected => Some(p.selected_row),
                button::Status::Hovered => Some(p.hover),
                button::Status::Pressed => Some(p.pressed),
                _ => None,
            };
            button::Style {
                background: tint.map(|c| Background::Color(c.into())),
                text_color: p.text_primary.into(),
                border: border::rounded(ROW_RADIUS),
                ..button::Style::default()
            }
        }
    }

    /// Tile of a glyph: its color by kind.
    pub fn glyph_tile(glyph: Glyph) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            fill(
                colors(theme).glyph_tile(glyph).0,
                border::rounded(TILE_RADIUS),
            )
        }
    }

    /// The search field itself: no box of its own, the panel is its frame.
    pub fn query(
        theme: &Theme,
        _: iced::widget::text_input::Status,
    ) -> iced::widget::text_input::Style {
        let p = theme::palette(theme);
        iced::widget::text_input::Style {
            background: Background::Color(iced::Color::TRANSPARENT),
            border: Border::default(),
            icon: p.text_tertiary.into(),
            placeholder: p.text_secondary.into(),
            value: p.text_primary.into(),
            selection: p.selected_row.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_read_on_the_launcher_in_both_themes() {
        for (palette, colors) in [
            (&Palette::DARK, &Colors::DARK),
            (&Palette::LIGHT, &Colors::LIGHT),
        ] {
            // The glass laid over the lightest desktop disc, the worst background.
            let below = palette.desktop_discs[2];
            let panel = colors.panel.over(below);
            let selected = palette.selected_row.over(panel);
            for text in [palette.text_primary, palette.text_secondary] {
                assert!(text.contrast(panel) >= 4.5, "{text:?} on {panel:?}");
                assert!(text.contrast(selected) >= 4.5, "{text:?} on {selected:?}");
            }
            let key = colors.key_row.over(selected);
            assert!(palette.text_secondary.contrast(key) >= 4.5);
            assert!(colors.tile_letter.contrast(colors.tile) >= 4.5);
            // The glyphs are not text: 3 to 1 is enough for them.
            for glyph in Glyph::ALL {
                let (tile, stroke) = colors.glyph_tile(glyph);
                assert!(stroke.contrast(tile) >= 3.0, "{glyph:?}");
            }
            assert_eq!(Colors::of(palette), colors);
        }
    }

    #[test]
    fn the_panel_grows_with_its_rows() {
        assert_eq!(panel_height(0, 0), 71.0);
        let one = panel_height(1, 1);
        assert!((one - (71.0 + 14.0 + 11.0 * 1.4 + 46.0)).abs() < 1e-3);
        assert!(panel_height(MAX_SECTIONS, MAX_RESULTS) < metrics_room());
        const { assert!(SECTION_RESULTS < MAX_RESULTS) };
    }

    #[test]
    fn the_glyphs_stay_in_the_grid() {
        for glyph in Glyph::ALL {
            let numbers = glyph
                .path()
                .split(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == ',')
                .flat_map(|chunk| chunk.split_inclusive('-'))
                .filter_map(|n| n.trim_end_matches('-').parse::<f32>().ok());
            for n in numbers {
                assert!(n.abs() <= crate::metrics::ICON_GRID, "{glyph:?}: {n}");
            }
            assert!(glyph.svg().contains("stroke-linecap=\"round\""));
        }
    }

    /// Room between the launcher's top and the dock on the reference screen.
    fn metrics_room() -> f32 {
        crate::metrics::REFERENCE_SCREEN.1 - TOP - crate::metrics::DOCK_HEIGHT
    }
}
