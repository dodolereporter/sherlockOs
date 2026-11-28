//! Tokens of Evidence, the file manager, added with its multiple selection and its fuller
//! right-click menu (lot E-01): the lasso dragged on the background of the listing, and the
//! new menu entries. No board draws it: its look is the
//! selection's blue, lighter, with a border. The other tokens of Evidence are in `texts`
//! and `metrics`. See `docs/evidence.md`.

use crate::color::{self, Color};
use crate::{metrics, typography};

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

/// Smallest window: the right-click menu (about 380 high) and the application chooser keep
/// their rows whole in it.
pub const MIN_WINDOW: (f32, f32) = (720.0, 460.0);

/// Longest wait for a read on D-Bus (UDisks2), as the bar's; then the devices are left out.
pub const BUS_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Longest wait for an action on D-Bus (mount, unmount, eject), which may wait for the
/// password asked by polkit.
pub const BUS_ACTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// The application chooser: what its panel holds besides the list (title, search field,
/// the switch's two lines, button, the gaps between them, its padding) and a margin above and below it. The
/// list takes the rest of the window's height, up to `metrics::CHOOSER_LIST_HEIGHT`, never
/// less than `CHOOSER_MIN_ROWS` rows.
pub const CHOOSER_CHROME: f32 = typography::CARD_TITLE_SIZE * 1.3
    + metrics::FIELD_HEIGHT
    // The switch's sentence takes two lines.
    + 2.0 * typography::UI_SIZE * 1.3
    + metrics::BUTTON_HEIGHT
    + 4.0 * metrics::PREVIEW_GAP
    + 2.0 * metrics::PANEL_PADDING
    + 2.0 * CHOOSER_MARGIN;
pub const CHOOSER_MARGIN: f32 = 24.0;
pub const CHOOSER_MIN_ROWS: f32 = 4.0;

/// The name of a file always keeps this much room in the listing: the columns, the preview
/// and then the sidebar fold before it shrinks further (lot E-05).
pub const NAME_MIN_WIDTH: f32 = 220.0;

/// The path bar (lot E-04). A segment of the breadcrumb is never wider than this: a longer
/// name keeps its start and its end around « … ».
pub const CRUMB_MAX_WIDTH: f32 = 180.0;
/// The breadcrumb and the text field cross-fade in this time, each way.
pub const PATH_SWITCH: std::time::Duration = std::time::Duration::from_millis(160);
/// A path that leads nowhere: the field shakes three times, as macOS says no.
pub const PATH_SHAKE: std::time::Duration = std::time::Duration::from_millis(450);
pub const PATH_SHAKES: f32 = 3.0;
pub const PATH_SHAKE_AMPLITUDE: f32 = 8.0;
/// The completion list: rows shown at most before it scrolls, and how far it slides in.
pub const COMPLETION_ROWS: f32 = 8.0;
pub const COMPLETION_SLIDE: f32 = 6.0;
/// The completion reads at most this many entries of a folder, and gives up after this time:
/// a huge or slow folder (a network share) never holds the field.
pub const COMPLETION_MAX_ENTRIES: usize = 20_000;
pub const COMPLETION_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);
/// Folders offered at most in the list.
pub const COMPLETION_MAX_SHOWN: usize = 200;
/// Width of the « Aller au dossier » sheet.
pub const GO_TO_WIDTH: f32 = 460.0;

/// The preview slides in and out in this time when its button or Ctrl+Alt+P toggles it.
pub const PREVIEW_SLIDE: std::time::Duration = std::time::Duration::from_millis(220);

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
    // The chooser keeps more than its fewest rows in the smallest window.
    assert!(MIN_WINDOW.1 - CHOOSER_CHROME > CHOOSER_MIN_ROWS * metrics::MENU_ITEM_HEIGHT);
    assert!(LASSO_THRESHOLD < crate::metrics::LIST_ROW_HEIGHT / 2.0);
    assert!(LASSO_SCROLL / 60.0 < LASSO_EDGE);
};
