//! Launcher (batch D-22a): the tokens, styles and texts of the application launcher. Values
//! from the launcher of the Panels design board, `docs/design/planches/Panneaux.dc.html`, and
//! of its light version.

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
/// Results shown at most, all sections together. A choice of D-22a: the panel stays clear of
/// the dock on the 900 high reference screen.
pub const MAX_RESULTS: usize = 8;

/// Height of the panel holding `sections` section labels and `rows` result rows.
pub fn panel_height(sections: usize, rows: usize) -> f32 {
    let label = crate::typography::SECTION_LABEL_SIZE * crate::typography::LINE_HEIGHT;
    let (top, _, bottom) = SECTION_PADDING;
    2.0 * PADDING
        + SEARCH_HEIGHT
        + sections as f32 * (top + label + bottom)
        + rows as f32 * ROW_HEIGHT
}

// Colors.

/// The launcher's own colors, for one theme; the rest comes from the palette.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Glass of the panel (`0.88`, denser than the other panels) and its border.
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
}

impl Colors {
    pub const DARK: Self = Self {
        panel: Color::rgba(16, 26, 44, 0.88),
        border: Color::rgba(255, 255, 255, 0.12),
        divider: Color::rgba(255, 255, 255, 0.08),
        key: Color::rgba(255, 255, 255, 0.10),
        key_row: Color::rgba(255, 255, 255, 0.12),
        tile: color::BLUE_BRIGHT,
        tile_letter: color::DEEPEST,
    };

    /// The light board: white glass, night blue veils at the same opacity.
    pub const LIGHT: Self = Self {
        panel: Color::rgba(255, 255, 255, 0.88),
        border: Color::rgba(20, 35, 59, 0.12),
        divider: Color::rgba(20, 35, 59, 0.08),
        key: Color::rgba(20, 35, 59, 0.10),
        key_row: Color::rgba(20, 35, 59, 0.12),
        tile: color::BLUE_BRIGHT,
        tile_letter: color::DEEPEST,
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
            assert_eq!(Colors::of(palette), colors);
        }
    }

    #[test]
    fn the_panel_grows_with_its_rows() {
        assert_eq!(panel_height(0, 0), 68.0);
        let one = panel_height(1, 1);
        assert!((one - (68.0 + 14.0 + 11.0 * 1.4 + 46.0)).abs() < 1e-3);
        assert!(panel_height(1, MAX_RESULTS) < metrics_room());
    }

    /// Room between the launcher's top and the dock on the reference screen.
    fn metrics_room() -> f32 {
        crate::metrics::REFERENCE_SCREEN.1 - TOP - crate::metrics::DOCK_HEIGHT
    }
}
