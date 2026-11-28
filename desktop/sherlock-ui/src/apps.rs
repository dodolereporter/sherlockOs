//! Applications view (lot D-41): every installed application in a grid of pages, after the
//! Launchpad of macOS. No design board draws it: the values follow the identity rules of
//! `docs/contexte.md` and the dock's icons.

use std::time::Duration;

use crate::color::{self, Color, Palette};
use crate::motion::{self, Spring};

// Geometry, in logical pixels, on the 1440 × 900 reference screen. Everything but the texts,
// the search field and the dots grows and shrinks with the screen (`Layout::fit`).

/// Side of an application icon, and the scale its file is looked up for (lot P-02c: larger,
/// as Launchpad's).
pub const ICON: f32 = 88.0;
pub const ICON_SCALE: u32 = 2;
/// Tile of an application without icon: radius as a share of the icon, initial as a share.
pub const TILE_RADIUS_RATIO: f32 = crate::metrics::DOCK_ICON_RADIUS_RATIO;
pub const LETTER_RATIO: f32 = 0.42;
/// The least width of a cell: how many columns fit. Its least height is the icon, the gap,
/// one line of name and the padding (`Layout::fit`).
pub const CELL_WIDTH: f32 = 148.0;
pub const CELL_PADDING: f32 = 5.0;
pub const CELL_RADIUS: f32 = 16.0;
pub const LABEL_GAP: f32 = 8.0;
/// Height of the line of a name.
pub const LABEL_HEIGHT: f32 = crate::typography::UI_SIZE * crate::typography::LINE_HEIGHT;
/// A name is cut with « … » past this share of the icon's side.
pub const LABEL_WIDTH_RATIO: f32 = 1.5;
/// The soft shadow under a name: a dark copy, this far below.
pub const LABEL_SHADOW_OFFSET: f32 = 1.0;
/// The cells spread over the room they have, up to these shares of their least size: the grid
/// fills the screen, as Launchpad's, without scattering on a very wide one.
pub const SPREAD: (f32, f32) = (1.7, 1.5);
/// Columns and rows of a page: one at least, Launchpad's seven by five at most.
pub const MIN_COLUMNS: usize = 1;
pub const MAX_COLUMNS: usize = 7;
pub const MIN_ROWS: usize = 1;
pub const MAX_ROWS: usize = 5;
/// Room on the sides of the grid.
pub const SIDE_MARGIN: f32 = 80.0;
/// The search field, a pill: width, height, distance from the screen top.
pub const SEARCH_WIDTH: f32 = 240.0;
pub const SEARCH_HEIGHT: f32 = 30.0;
pub const SEARCH_TOP: f32 = 40.0;
/// Between the field and the room of the grid, between that room and the dots.
pub const GRID_GAP: f32 = 18.0;
pub const DOTS_GAP: f32 = 8.0;
/// Page dots: diameter, gap, distance of their row from the screen bottom (clear of the dock,
/// which keeps its size on every screen).
pub const DOT: f32 = 8.0;
pub const DOT_GAP: f32 = 12.0;
pub const DOTS_BOTTOM: f32 = 112.0;
/// The menu of an application: the dock menu's width, and its least distance from the screen
/// edges.
pub const MENU_WIDTH: f32 = crate::dock::MENU_WIDTH;
pub const MENU_MARGIN: f32 = 8.0;
/// How far the sizes follow the screen: from three quarters (a small laptop, a projector) to
/// 1.3 times (1920 × 1080 and larger) the reference ones.
pub const MIN_FACTOR: f32 = 0.75;
pub const MAX_FACTOR: f32 = 1.3;
/// Side the icon files are looked up for: the largest icon.
pub const ICON_LOOKUP: u32 = (ICON * MAX_FACTOR) as u32;

/// The view laid out for a screen: its sizes and its grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// The sizes, as a share of the reference ones.
    pub factor: f32,
    pub icon: f32,
    /// A cell spread over the room of the grid, and the button inside it.
    pub cell: (f32, f32),
    pub button: (f32, f32),
    pub columns: usize,
    pub rows: usize,
    pub search_top: f32,
    /// Top of the grid, centered in its room.
    pub grid_top: f32,
}

impl Layout {
    /// The layout for a screen of `width` by `height` logical pixels, whatever its scale: the
    /// sizes follow the smaller of its two ratios to the reference screen, within
    /// `MIN_FACTOR` and `MAX_FACTOR`; then as many cells as fit between the field and the dots,
    /// within the bounds, spread over that room and centered in it.
    pub fn fit(width: f32, height: f32) -> Self {
        let (reference_width, reference_height) = crate::metrics::REFERENCE_SCREEN;
        let factor = (width / reference_width)
            .min(height / reference_height)
            .clamp(MIN_FACTOR, MAX_FACTOR);
        let icon = ICON * factor;
        let button = (
            CELL_WIDTH * factor,
            icon + LABEL_GAP * factor + LABEL_HEIGHT + 2.0 * CELL_PADDING,
        );
        let search_top = SEARCH_TOP * factor;
        let top = search_top + SEARCH_HEIGHT + GRID_GAP * factor;
        let room = (
            (width - 2.0 * SIDE_MARGIN * factor).max(0.0),
            (height - DOTS_BOTTOM - DOTS_GAP - top).max(0.0),
        );
        let fit = |room: f32, side: f32, (least, most): (usize, usize)| {
            ((room / side).floor().max(0.0) as usize).clamp(least, most)
        };
        let columns = fit(room.0, button.0, (MIN_COLUMNS, MAX_COLUMNS));
        let rows = fit(room.1, button.1, (MIN_ROWS, MAX_ROWS));
        let cell = (
            (room.0 / columns as f32).clamp(button.0, button.0 * SPREAD.0),
            (room.1 / rows as f32).clamp(button.1, button.1 * SPREAD.1),
        );
        let grid_top = top + ((room.1 - rows as f32 * cell.1) / 2.0).max(0.0);
        Self {
            factor,
            icon,
            cell,
            button,
            columns,
            rows,
            search_top,
            grid_top,
        }
    }

    /// Applications on a page.
    pub fn per_page(&self) -> usize {
        self.columns * self.rows
    }

    /// Width of the name under an icon.
    pub fn label_width(&self) -> f32 {
        (self.icon * LABEL_WIDTH_RATIO).min(self.button.0 - 2.0 * CELL_PADDING)
    }

    /// Radius of the tile of an application without icon, and the size of its initial.
    pub fn tile_radius(&self) -> f32 {
        self.icon * TILE_RADIUS_RATIO
    }

    pub fn letter_size(&self) -> f32 {
        crate::ellipsis::at_least_a_pixel((self.icon * LETTER_RATIO).round())
    }

    /// Center of the cell of `place` on its page, on a screen `width` wide.
    pub fn center(&self, place: usize, width: f32) -> (f32, f32) {
        let at = place % self.per_page().max(1);
        let (column, line) = (at % self.columns, at / self.columns);
        let left = (width - self.columns as f32 * self.cell.0) / 2.0;
        (
            left + (column as f32 + 0.5) * self.cell.0,
            self.grid_top + (line as f32 + 0.5) * self.cell.1,
        )
    }
}

/// Where a started application goes, as the view fades out: the middle of the dock, at the
/// bottom of a screen `height` high.
pub fn dock_target(width: f32, height: f32) -> (f32, f32) {
    let middle = crate::dock::MARGIN_BOTTOM + crate::metrics::DOCK_HEIGHT / 2.0;
    (width / 2.0, height - middle)
}

// The backdrop: the wallpaper, blurred and darkened (lot P-02c).

/// Width of the blurred copy of the wallpaper, whatever the screen: the GPU stretches it.
pub const BACKDROP_WIDTH: u32 = 480;
/// Blur of that copy, in its pixels (a Gaussian's sigma): about 50 pixels of a 1920 screen.
pub const BACKDROP_BLUR: f32 = 12.0;
/// Every wallpaper is darkened this much at least (its channels multiplied)...
pub const BACKDROP_DIM: f32 = 0.85;
/// ... and more if needed, so that no point is brighter than this relative luminance: the
/// white names read at 4.5 to 1 on it, even under the veil of the selected cell.
pub const BACKDROP_MAX_LUMINANCE: f32 = 0.11;

// Motion.

/// The icons arrive from this scale, fading in, and leave towards it (macOS Launchpad).
pub const OPEN_SCALE: f32 = 1.1;
/// The tile started goes to the dock (`dock_target`) as the view fades out, shrinking to the
/// dock's icon.
pub const LAUNCH_SIDE: f32 = crate::metrics::DOCK_ICON;
/// Opening and closing of the view: icons, backdrop and field together.
pub const OPEN_SPRING: Spring = motion::OPEN_SPRING;
/// The view is open, or gone, within this share of its run.
pub const OPEN_PRECISION: f64 = 0.01;
/// Pages sliding into place, after a wheel notch, a swipe or a bounce.
pub const PAGE_SPRING: Spring = motion::SNAP_SPRING;
/// Pages settle within this share of a page.
pub const PAGE_PRECISION: f64 = 0.001;
/// Rubber band past the first or the last page: the content follows the fingers less and less,
/// `(1 - 1 / (excess × RUBBER / width + 1)) × width` (the formula of iOS).
pub const RUBBER: f32 = 0.55;
/// A swipe let go past this share of the width turns the page; a faster one turns it whatever
/// the distance (pixels per second).
pub const TURN_SHARE: f32 = 0.18;
pub const FLICK_SPEED: f32 = 500.0;
/// A swipe of the touchpad is over when no scroll event came for this long.
pub const SWIPE_END: Duration = Duration::from_millis(120);
/// Wheel notches closer than this turn one page only: a wheel sends a burst of them.
pub const WHEEL_PAUSE: Duration = Duration::from_millis(250);
/// A notch past the first or the last page throws the pages this fast (pages per second): they
/// stretch and come back, the elastic bounce.
pub const BOUNCE_SPEED: f64 = 2.5;

// Colors.

/// The view's own colors. The backdrop is darkened in both themes, as macOS does: the names and
/// the search field stay light on dark on it. The menu follows the theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colors {
    /// Laid over the desktop when there is no wallpaper to blur: night blue, dense enough for
    /// the names on a white desktop.
    pub veil: Color,
    pub label: Color,
    /// The soft shadow under a name.
    pub label_shadow: Color,
    /// Veil of the hovered cell, of the selected one.
    pub hover: Color,
    pub selected: Color,
    /// Page dots.
    pub dot: Color,
    pub dot_active: Color,
    /// Tile of an application without icon, and its initial.
    pub tile: Color,
    pub tile_letter: Color,
    /// « Aucune application » under the field.
    pub nothing: Color,
    /// The search pill: a dark glass with a light edge, its texts and its magnifier.
    pub field: Color,
    pub field_edge: Color,
    pub field_edge_focused: Color,
    pub field_text: Color,
    pub field_placeholder: Color,
    pub field_selection: Color,
    pub field_icon: Color,
}

impl Colors {
    pub const DARK: Self = Self {
        veil: Color::rgba(6, 12, 24, 0.70),
        label: Color::rgb(255, 255, 255),
        label_shadow: Color::rgba(0, 0, 0, 0.45),
        hover: Color::rgba(255, 255, 255, 0.08),
        selected: Color::rgba(255, 255, 255, 0.14),
        dot: Color::rgba(255, 255, 255, 0.32),
        dot_active: Color::rgba(255, 255, 255, 0.92),
        tile: color::BLUE_BRIGHT,
        tile_letter: color::DEEPEST,
        nothing: Color::rgb(255, 255, 255),
        field: Color::rgba(8, 14, 26, 0.32),
        field_edge: Color::rgba(255, 255, 255, 0.20),
        field_edge_focused: Color::rgba(255, 255, 255, 0.34),
        field_text: Color::rgb(255, 255, 255),
        field_placeholder: Color::rgba(255, 255, 255, 0.75),
        field_selection: Color::rgba(255, 255, 255, 0.30),
        field_icon: Color::rgba(255, 255, 255, 0.75),
    };

    /// The light theme darkens the same way: only the menu changes.
    pub const LIGHT: Self = Self::DARK;

    /// The colors that go with `palette`.
    pub fn of(palette: &Palette) -> &'static Self {
        if *palette == Palette::LIGHT {
            &Self::LIGHT
        } else {
            &Self::DARK
        }
    }
}

/// Texts of the view, in French.
pub mod text {
    /// Placeholder of the search field.
    pub const PLACEHOLDER: &str = "Rechercher";
    /// Nothing matches the search.
    pub const NOTHING: &str = "Aucune application";
    /// The right-click menu of an application: pinning to the dock (`dock::text::KEEP`,
    /// `UNPIN`), then showing its desktop file in the file manager.
    pub const SHOW: &str = "Afficher dans Evidence";
}

#[cfg(feature = "iced")]
pub use styles::*;

#[cfg(feature = "iced")]
mod styles {
    use iced::widget::{button, container, text_input};
    use iced::{Background, Border, Theme, border};

    use super::*;
    use crate::theme;

    fn colors(theme: &Theme) -> &'static Colors {
        Colors::of(theme::palette(theme))
    }

    fn fill(c: Color, edge: Border) -> container::Style {
        container::Style {
            background: Some(Background::Color(c.into())),
            border: edge,
            ..container::Style::default()
        }
    }

    /// The search pill, the same in both themes, on the dark backdrop.
    pub fn field(theme: &Theme, status: text_input::Status) -> text_input::Style {
        let c = colors(theme);
        let edge = match status {
            text_input::Status::Focused { .. } => c.field_edge_focused,
            _ => c.field_edge,
        };
        text_input::Style {
            background: Background::Color(c.field.into()),
            border: border::rounded(SEARCH_HEIGHT / 2.0)
                .width(crate::metrics::BORDER_WIDTH)
                .color(edge),
            icon: c.field_icon.into(),
            placeholder: c.field_placeholder.into(),
            value: c.field_text.into(),
            selection: c.field_selection.into(),
        }
    }

    /// The veil over the whole screen.
    pub fn veil(theme: &Theme) -> container::Style {
        fill(colors(theme).veil, Border::default())
    }

    /// A cell: a light veil under the pointer, a denser one when the keyboard selects it.
    pub fn cell(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |theme, status| {
            let c = colors(theme);
            let tint = match status {
                _ if selected => Some(c.selected),
                button::Status::Hovered | button::Status::Pressed => Some(c.hover),
                _ => None,
            };
            button::Style {
                background: tint.map(|t| Background::Color(t.into())),
                text_color: c.label.into(),
                border: border::rounded(CELL_RADIUS),
                ..button::Style::default()
            }
        }
    }

    /// A page dot, brighter for the page shown.
    pub fn dot(active: bool) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let c = colors(theme);
            let tint = if active { c.dot_active } else { c.dot };
            fill(tint, border::rounded(DOT / 2.0))
        }
    }

    /// Tile of an application without icon, with its initial, rounded by `radius`.
    pub fn tile(radius: f32) -> impl Fn(&Theme) -> container::Style {
        move |theme| {
            let c = colors(theme);
            container::Style {
                text_color: Some(c.tile_letter.into()),
                ..fill(c.tile, border::rounded(radius))
            }
        }
    }

    /// The name under an icon, or « Aucune application ».
    pub fn label(theme: &Theme) -> iced::widget::text::Style {
        iced::widget::text::Style {
            color: Some(colors(theme).label.into()),
        }
    }

    /// The shadow of a name.
    pub fn label_shadow(theme: &Theme) -> iced::widget::text::Style {
        iced::widget::text::Style {
            color: Some(colors(theme).label_shadow.into()),
        }
    }

    pub fn nothing(theme: &Theme) -> iced::widget::text::Style {
        iced::widget::text::Style {
            color: Some(colors(theme).nothing.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The brightest grey the backdrop may hold (`BACKDROP_MAX_LUMINANCE`).
    fn brightest_backdrop() -> Color {
        (0..=255u8)
            .rev()
            .map(|v| Color::rgb(v, v, v))
            .find(|c| c.luminance() <= BACKDROP_MAX_LUMINANCE)
            .unwrap()
    }

    #[test]
    fn the_names_read_on_any_wallpaper() {
        for (palette, colors) in [
            (&Palette::DARK, &Colors::DARK),
            (&Palette::LIGHT, &Colors::LIGHT),
        ] {
            // The blurred wallpaper at its brightest, and without one, the veil over the
            // discs of the desktop and over a white one: the worst backdrops.
            let backdrops = [
                brightest_backdrop(),
                colors.veil.over(palette.desktop_discs[2]),
                colors.veil.over(Color::rgb(255, 255, 255)),
            ];
            for below in backdrops {
                for (text, on) in [
                    (colors.label, below),
                    (colors.label, colors.selected.over(below)),
                    (colors.nothing, below),
                ] {
                    assert!(text.contrast(on) >= 4.5, "{text:?} on {on:?}");
                }
                let field = colors.field.over(below);
                for text in [colors.field_text, colors.field_placeholder.over(field)] {
                    assert!(text.contrast(field) >= 4.5, "{text:?} on {field:?}");
                }
                // The dots are not text: 3 to 1 for the page shown.
                assert!(colors.dot_active.over(below).contrast(below) >= 3.0);
            }
            assert!(colors.tile_letter.contrast(colors.tile) >= 4.5);
            assert_eq!(Colors::of(palette), colors);
        }
    }

    #[test]
    fn the_grid_fills_the_screen_within_its_bounds() {
        // Logical sizes: 2560 × 1600 at scale 1.5 is 1707 × 1067, at 1.25 2048 × 1280; 3840 ×
        // 2160 at scale 2 is 1920 × 1080.
        let screens = [
            (1366.0, 768.0),
            (1440.0, 900.0),
            (1920.0, 1080.0),
            (2560.0 / 1.5, 1600.0 / 1.5),
            (2560.0 / 1.25, 1600.0 / 1.25),
            (1280.0, 720.0),
            (1024.0, 768.0),
            (800.0, 600.0),
            (3840.0, 2160.0),
            (5120.0, 1440.0),
        ];
        for (width, height) in screens {
            let layout = Layout::fit(width, height);
            assert!((MIN_FACTOR..=MAX_FACTOR).contains(&layout.factor));
            assert!((MIN_COLUMNS..=MAX_COLUMNS).contains(&layout.columns));
            assert!((MIN_ROWS..=MAX_ROWS).contains(&layout.rows));
            assert!(layout.cell.0 >= layout.button.0 && layout.cell.1 >= layout.button.1);
            // The grid fits beside the margins, under the field, above the dots, the dots
            // above the dock.
            let grid = layout.columns as f32 * layout.cell.0;
            assert!(grid <= width, "{width}: {layout:?}");
            let bottom = layout.grid_top + layout.rows as f32 * layout.cell.1;
            let room_bottom = height - DOTS_BOTTOM - DOTS_GAP;
            assert!(
                bottom <= room_bottom + 0.01,
                "{width} × {height}: {layout:?}"
            );
            let top = layout.search_top + SEARCH_HEIGHT + GRID_GAP * layout.factor;
            assert!(layout.grid_top >= top - 0.01);
            // Centered in its room.
            assert!(
                ((layout.grid_top - top) - (room_bottom - bottom)).abs() < 0.5,
                "{width} × {height}: {layout:?}"
            );
            assert!(layout.icon <= ICON_LOOKUP as f32 + 1.0);
            assert!(layout.label_width() <= layout.button.0);
        }
        const { assert!(DOTS_BOTTOM - DOT > crate::metrics::DOCK_HEIGHT) };
        let full = |w, h| {
            let l = Layout::fit(w, h);
            (l.columns, l.rows)
        };
        // Launchpad's grid on the author's screen, the reference one, a small laptop, and the
        // larger ones, with larger icons.
        for (width, height) in [
            (1366.0, 768.0),
            (1440.0, 900.0),
            (1920.0, 1080.0),
            (2560.0 / 1.25, 1600.0 / 1.25),
        ] {
            assert_eq!(full(width, height), (MAX_COLUMNS, MAX_ROWS), "{width}");
        }
        assert_eq!(Layout::fit(1440.0, 900.0).icon, ICON);
        assert!(Layout::fit(1920.0, 1080.0).icon > Layout::fit(1366.0, 768.0).icon);
        assert_eq!(full(100.0, 100.0), (MIN_COLUMNS, MIN_ROWS));
        // On 1920 × 1080 the grid takes most of the screen, not its top.
        let layout = Layout::fit(1920.0, 1080.0);
        assert!(layout.columns as f32 * layout.cell.0 > 0.75 * 1920.0);
        assert!(layout.rows as f32 * layout.cell.1 > 0.7 * 1080.0);
    }

    #[test]
    fn a_cell_center_and_the_dock() {
        let layout = Layout::fit(1920.0, 1080.0);
        let (x, y) = layout.center(0, 1920.0);
        let (x2, _) = layout.center(layout.columns - 1, 1920.0);
        assert!((x + x2 - 1920.0).abs() < 0.01, "the grid is centered");
        assert!((y - layout.grid_top - layout.cell.1 / 2.0).abs() < 0.01);
        // The next page starts at the same place.
        assert_eq!(layout.center(layout.per_page(), 1920.0), (x, y));
        let (dx, dy) = dock_target(1920.0, 1080.0);
        assert_eq!(dx, 960.0);
        assert!(dy > 1080.0 - crate::metrics::DOCK_HEIGHT && dy < 1080.0);
    }
}
