//! Tokens of Evidence, the file manager, added with its multiple selection and its fuller
//! right-click menu (lot E-01): the lasso dragged on the background of the listing, and the
//! new menu entries. No board draws it: its look is the
//! selection's blue, lighter, with a border. The other tokens of Evidence are in `texts`
//! and `metrics`. See `docs/evidence.md`.

use crate::color::{self, Color};

/// How far the pointer goes from the press, in logical pixels, before a press on the background
/// becomes a lasso: a click with a slight slip stays a click.
pub const LASSO_THRESHOLD: f32 = 4.0;
/// Band along the top and bottom edges of the listing where a lasso scrolls it: one row.
pub const LASSO_EDGE: f32 = crate::metrics::LIST_ROW_HEIGHT;
/// Fastest scroll under a lasso held past an edge, in logical pixels per second: 25 rows.
pub const LASSO_SCROLL: f32 = 1000.0;
/// Longest time one frame of that scroll accounts for: a frame late by more scrolls no further.
pub const LASSO_LONGEST_STEP: std::time::Duration = std::time::Duration::from_millis(100);
/// Width of the lasso's border, and its corner radius.
pub const LASSO_BORDER_WIDTH: f32 = 1.0;
pub const LASSO_RADIUS: f32 = 3.0;
/// Opacity of the selection's blue inside the lasso.
const LASSO_FILL_ALPHA: f32 = 0.14;

/// Inside of the lasso.
pub fn lasso_fill() -> Color {
    Color {
        a: LASSO_FILL_ALPHA,
        ..color::palette().blue
    }
}

/// Border of the lasso.
pub fn lasso_border() -> Color {
    color::palette().blue
}

/// Entries of the right-click menu that Evidence does not offer yet: shown greyed.
pub const COMPRESS: &str = "Compresser";
pub const PROPERTIES: &str = "Propriétés";
/// The submenus of the menu on the background: the sort, and the view with its three kinds
/// (only the list exists so far).
pub const SORT_BY: &str = "Trier par";
pub const VIEW: &str = "Vue";
pub const VIEW_LIST: &str = "Liste";
pub const VIEW_GRID: &str = "Grille";
pub const VIEW_COLUMNS: &str = "Colonnes";

#[cfg(feature = "iced")]
pub use style::lasso;

#[cfg(feature = "iced")]
mod style {
    use iced::widget::container;
    use iced::{Background, Theme, border};

    /// The lasso: a light fill of the selection's blue, with a border.
    pub fn lasso(_: &Theme) -> container::Style {
        container::Style {
            background: Some(Background::Color(super::lasso_fill().into())),
            border: border::width(super::LASSO_BORDER_WIDTH)
                .color(super::lasso_border())
                .rounded(super::LASSO_RADIUS),
            ..container::Style::default()
        }
    }
}

// The lasso starts before a row is crossed, and scrolls from a band narrower than the
// listing.
const _: () = {
    assert!(LASSO_THRESHOLD < crate::metrics::LIST_ROW_HEIGHT / 2.0);
    assert!(LASSO_SCROLL / 60.0 < LASSO_EDGE);
};
