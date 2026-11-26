//! Applications view (lot D-41): every installed application in a grid of pages, after the
//! Launchpad of macOS. No design board draws it: the values follow the identity rules of
//! `docs/contexte.md` and the dock's icons.

use std::time::Duration;

use crate::color::{self, Color, Palette};
use crate::motion::{self, Spring};

// Geometry, in logical pixels, on the 1440 × 900 reference screen. Everything but the texts,
// the search field and the dots grows and shrinks with the screen (`Layout::fit`).

/// Side of an application icon, and the scale its file is looked up for.
pub const ICON: f32 = 80.0;
pub const ICON_SCALE: u32 = 2;
/// Tile of an application without icon: radius as a share of the icon, initial as a share.
pub const TILE_RADIUS_RATIO: f32 = crate::metrics::DOCK_ICON_RADIUS_RATIO;
pub const LETTER_RATIO: f32 = 0.42;
/// A cell of the grid: the icon, its name under it, the veil of the selected one.
pub const CELL_WIDTH: f32 = 148.0;
pub const CELL_HEIGHT: f32 = 120.0;
pub const CELL_PADDING: f32 = 6.0;
pub const CELL_RADIUS: f32 = 16.0;
pub const LABEL_GAP: f32 = 8.0;
/// Columns and rows of a page: one at least, Launchpad's seven by five at most.
pub const MIN_COLUMNS: usize = 1;
pub const MAX_COLUMNS: usize = 7;
pub const MIN_ROWS: usize = 1;
pub const MAX_ROWS: usize = 5;
/// Room around the grid: on the sides, above (the search field).
pub const SIDE_MARGIN: f32 = 80.0;
pub const GRID_TOP: f32 = 132.0;
/// The search field: width, distance from the screen top.
pub const SEARCH_WIDTH: f32 = 280.0;
pub const SEARCH_TOP: f32 = 52.0;
/// Below the grid: the dots and the dock, which keep their size on every screen.
pub const GRID_BOTTOM: f32 = 140.0;
/// Page dots: diameter, gap, distance of their row from the screen bottom.
pub const DOT: f32 = 8.0;
pub const DOT_GAP: f32 = 12.0;
pub const DOTS_BOTTOM: f32 = 112.0;
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
    pub cell: (f32, f32),
    pub columns: usize,
    pub rows: usize,
    pub search_top: f32,
    pub grid_top: f32,
}

impl Layout {
    /// The layout for a screen of `width` by `height` logical pixels, whatever its scale: the
    /// sizes follow the smaller of its two ratios to the reference screen, within
    /// `MIN_FACTOR` and `MAX_FACTOR`; then as many cells as fit, within the bounds.
    pub fn fit(width: f32, height: f32) -> Self {
        let (reference_width, reference_height) = crate::metrics::REFERENCE_SCREEN;
        let factor = (width / reference_width)
            .min(height / reference_height)
            .clamp(MIN_FACTOR, MAX_FACTOR);
        let cell = (CELL_WIDTH * factor, CELL_HEIGHT * factor);
        let grid_top = GRID_TOP * factor;
        let fit = |room: f32, side: f32, (least, most): (usize, usize)| {
            ((room / side).floor().max(0.0) as usize).clamp(least, most)
        };
        Self {
            factor,
            icon: ICON * factor,
            cell,
            columns: fit(
                width - 2.0 * SIDE_MARGIN * factor,
                cell.0,
                (MIN_COLUMNS, MAX_COLUMNS),
            ),
            rows: fit(
                height - grid_top - GRID_BOTTOM,
                cell.1,
                (MIN_ROWS, MAX_ROWS),
            ),
            search_top: SEARCH_TOP * factor,
            grid_top,
        }
    }

    /// Applications on a page.
    pub fn per_page(&self) -> usize {
        self.columns * self.rows
    }

    /// Width of the name under an icon.
    pub fn label_width(&self) -> f32 {
        self.cell.0 - 2.0 * CELL_PADDING
    }

    /// Radius of the tile of an application without icon, and the size of its initial.
    pub fn tile_radius(&self) -> f32 {
        self.icon * TILE_RADIUS_RATIO
    }

    pub fn letter_size(&self) -> f32 {
        (self.icon * LETTER_RATIO).round()
    }
}

// Motion.

/// The icons arrive from this scale, fading in, and leave towards it (macOS Launchpad).
pub const OPEN_SCALE: f32 = 1.1;
/// The tile started grows to this scale as the view fades out.
pub const LAUNCH_SCALE: f32 = 1.2;
/// Opening and closing of the view: icons, veil and field together.
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
    /// Laid over the wallpaper: night blue, dense enough for the names on a white wallpaper.
    pub veil: Color,
    pub label: Color,
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
    /// The magnifier of the search field, which stays dark in both themes.
    pub field_icon: Color,
}

impl Colors {
    pub const DARK: Self = Self {
        veil: Color::rgba(6, 12, 24, 0.70),
        label: color::TEXT_PRIMARY,
        hover: Color::rgba(255, 255, 255, 0.08),
        selected: Color::rgba(255, 255, 255, 0.14),
        dot: Color::rgba(255, 255, 255, 0.32),
        dot_active: Color::rgba(255, 255, 255, 0.92),
        tile: color::BLUE_BRIGHT,
        tile_letter: color::DEEPEST,
        nothing: Color::rgb(0xC5, 0xCE, 0xDB),
        field_icon: color::TEXT_TERTIARY,
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

    /// The search field: the dark theme's, whatever the theme, on the dark backdrop.
    pub fn field(_: &Theme, status: text_input::Status) -> text_input::Style {
        theme::text_field(&theme::theme_for(color::Theme::Dark), status)
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

    pub fn nothing(theme: &Theme) -> iced::widget::text::Style {
        iced::widget::text::Style {
            color: Some(colors(theme).nothing.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_read_on_any_wallpaper() {
        for (palette, colors) in [
            (&Palette::DARK, &Colors::DARK),
            (&Palette::LIGHT, &Colors::LIGHT),
        ] {
            // The discs of the default wallpaper, and a white one: the worst backdrop.
            let wallpapers = [palette.desktop_discs[2], Color::rgb(255, 255, 255)];
            for below in wallpapers {
                let veil = colors.veil.over(below);
                for (text, on) in [
                    (colors.label, veil),
                    (colors.label, colors.selected.over(veil)),
                    (colors.nothing, veil),
                ] {
                    assert!(text.contrast(on) >= 4.5, "{text:?} on {on:?}");
                }
                // The dots are not text: 3 to 1 for the page shown.
                assert!(colors.dot_active.over(veil).contrast(veil) >= 3.0);
            }
            assert!(colors.tile_letter.contrast(colors.tile) >= 4.5);
            assert_eq!(Colors::of(palette), colors);
        }
    }

    #[test]
    fn the_grid_follows_the_screen_within_its_bounds() {
        // Logical sizes: 2560 × 1600 at scale 1.5 is 1707 × 1067, 3840 × 2160 at scale 2 is
        // 1920 × 1080.
        let screens = [
            (1366.0, 768.0),
            (1440.0, 900.0),
            (1920.0, 1080.0),
            (2560.0 / 1.5, 1600.0 / 1.5),
            (1280.0, 720.0),
            (1024.0, 768.0),
            (800.0, 600.0),
            (3840.0, 2160.0),
        ];
        for (width, height) in screens {
            let layout = Layout::fit(width, height);
            assert!((MIN_FACTOR..=MAX_FACTOR).contains(&layout.factor));
            assert!((MIN_COLUMNS..=MAX_COLUMNS).contains(&layout.columns));
            assert!((MIN_ROWS..=MAX_ROWS).contains(&layout.rows));
            // The grid fits beside the margins, above the dots, the dots above the dock.
            let grid = layout.columns as f32 * layout.cell.0;
            assert!(grid <= width, "{width}: {layout:?}");
            let bottom = layout.grid_top + layout.rows as f32 * layout.cell.1;
            assert!(
                bottom <= height - DOTS_BOTTOM,
                "{width} × {height}: {layout:?}"
            );
            assert!(layout.search_top + crate::metrics::FIELD_HEIGHT < layout.grid_top);
            assert!(layout.icon <= ICON_LOOKUP as f32 + 1.0);
        }
        const { assert!(DOTS_BOTTOM - DOT > crate::metrics::DOCK_HEIGHT) };
        let full = |w, h| {
            let l = Layout::fit(w, h);
            (l.columns, l.rows)
        };
        // Launchpad's grid on the author's screen, the reference one, a small laptop, and the
        // larger ones, with larger icons.
        for (width, height) in [(1366.0, 768.0), (1440.0, 900.0), (1920.0, 1080.0)] {
            assert_eq!(full(width, height), (MAX_COLUMNS, MAX_ROWS));
        }
        assert_eq!(Layout::fit(1440.0, 900.0).icon, ICON);
        assert!(Layout::fit(1920.0, 1080.0).icon > Layout::fit(1366.0, 768.0).icon);
        assert_eq!(full(100.0, 100.0), (MIN_COLUMNS, MIN_ROWS));
    }
}
