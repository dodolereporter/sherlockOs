//! Geometry of the dock: the magnification curve of the Dock design board, the place of each
//! icon, hit testing. Pure computation, in logical pixels of the dock surface, whose origin is
//! its top left corner.
//!
//! Everything is first computed along the dock (`along`, left to right or top to bottom) and
//! away from the screen edge it sits on (`depth`), then placed for that edge: the same code
//! lays out a dock at the bottom, at the top, on the left or on the right.

use dock_config::{DockConfig, Position};
use iced::{Point, Rectangle, Size};
use sherlock_ui::{dock, metrics, typography};

/// The settings that shape the dock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// Side of an icon at rest.
    pub icon: f32,
    /// Magnification factor, 1 without magnification.
    pub factor: f32,
    pub position: Position,
}

impl Default for Geometry {
    /// The Dock board: icons of 52, magnified 1.7 times, at the bottom.
    fn default() -> Self {
        Self {
            icon: metrics::DOCK_ICON,
            factor: metrics::DOCK_MAGNIFICATION,
            position: Position::Bottom,
        }
    }
}

impl Geometry {
    pub fn from_config(config: &DockConfig) -> Self {
        Self {
            icon: config.icon_size,
            factor: if config.magnification {
                config.magnification_factor
            } else {
                1.0
            },
            position: config.position,
        }
    }

    /// Thickness of the bar: the icon and the padding on both sides (76 for icons of 52).
    pub fn bar_thickness(&self) -> f32 {
        self.icon + 2.0 * dock::PADDING.1
    }

    /// Distance from the edge of the bar to the icons: border and padding.
    fn inset(&self) -> f32 {
        metrics::BORDER_WIDTH + dock::PADDING.1
    }

    /// Distance from the screen edge of the bar to the label, past the tallest icon (108 on
    /// the board).
    pub fn label_depth(&self) -> f32 {
        let tallest = self.inset() + self.icon * self.factor;
        tallest.max(self.bar_thickness()) + dock::LABEL_GAP
    }

    /// Height of the label: its text and its padding.
    pub fn label_height() -> f32 {
        2.0 * dock::LABEL_PADDING.0 + dock::LABEL_SIZE * typography::LINE_HEIGHT
    }

    /// Scale of icon `index` among `count` when the pointer is at `p` along the bar, 0 at its
    /// start and 1 at its end. The formula of the board's script:
    /// `d = p × n − (i + 0.5)`, `k = 1 + (max − 1) × exp(−d² / (2 × reach²))`.
    pub fn scale(&self, p: f32, index: usize, count: usize) -> f32 {
        let d = p * count as f32 - (index as f32 + 0.5);
        let reach = metrics::DOCK_MAGNIFICATION_REACH;
        1.0 + (self.factor - 1.0) * (-(d * d) / (2.0 * reach * reach)).exp()
    }

    /// Size each icon heads for: the base size without pointer, otherwise the base size times
    /// its scale, rounded to the pixel as on the board.
    pub fn targets(&self, p: Option<f32>, count: usize) -> Vec<f32> {
        (0..count)
            .map(|i| match p {
                Some(p) => (self.icon * self.scale(p, i, count)).round(),
                None => self.icon,
            })
            .collect()
    }

    /// Length of the bar holding icons of `sizes`, and the separator if there is one: border,
    /// padding, icons and the gaps between them.
    pub fn bar_length(&self, sizes: &[f32], separator: bool) -> f32 {
        let slots = sizes.len() + usize::from(separator);
        let gaps = slots.saturating_sub(1) as f32 * metrics::DOCK_ICON_GAP;
        let separator = if separator { separator_length() } else { 0.0 };
        2.0 * (metrics::BORDER_WIDTH + dock::PADDING.0)
            + sizes.iter().sum::<f32>()
            + gaps
            + separator
    }

    /// Size of the dock surface for `count` icons: as long as the longest magnified bar plus
    /// room for the label at its ends, as deep as the label (beside the icons on the sides).
    pub fn surface_size(&self, count: usize, separator: bool) -> Size {
        let longest = (0..=1000)
            .map(|step| {
                self.bar_length(&self.targets(Some(step as f32 / 1000.0), count), separator)
            })
            .fold(
                self.bar_length(&self.targets(None, count), separator),
                f32::max,
            );
        let along = (longest + 2.0 * dock::SURFACE_SLACK).ceil();
        let room = if self.position.is_vertical() {
            dock::LABEL_SIDE_ROOM
        } else {
            Self::label_height()
        };
        let depth = (self.label_depth() + room).ceil();
        if self.position.is_vertical() {
            Size::new(depth, along)
        } else {
            Size::new(along, depth)
        }
    }
}

/// Room the separator takes along the bar: its thickness and its margins.
fn separator_length() -> f32 {
    dock::SEPARATOR.0 + 2.0 * dock::SEPARATOR_MARGIN
}

/// The icon whose label shows: the closest one to the pointer, as the board picks it.
pub fn nearest(p: f32, count: usize) -> usize {
    let index = (p * count as f32).floor().max(0.0) as usize;
    index.min(count.saturating_sub(1))
}

/// Where everything is, for icons of the given sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub geometry: Geometry,
    pub surface: Size,
    /// The glass bar, against the screen edge and centered along it.
    pub bar: Rectangle,
    /// Each icon, on the padding nearest the screen edge. A magnified icon goes past the bar.
    pub icons: Vec<Rectangle>,
    /// The separator before the open applications that are not pinned.
    pub separator: Option<Rectangle>,
    /// Start and length of the bar along the dock.
    along: (f32, f32),
}

impl Layout {
    /// Lays out icons of `sizes` in a surface of `surface`, the separator before icon
    /// `separator`. The bar is as long as its icons: when one grows, the bar grows at both
    /// ends and pushes the neighbours away.
    pub fn new(geometry: Geometry, surface: Size, sizes: &[f32], separator: Option<usize>) -> Self {
        let vertical = geometry.position.is_vertical();
        let span = if vertical {
            surface.height
        } else {
            surface.width
        };
        let length = geometry.bar_length(sizes, separator.is_some());
        let start = (span - length) / 2.0;
        let place = |along: f32, depth: f32, length: f32, thickness: f32| {
            to_surface(geometry.position, surface, along, depth, length, thickness)
        };
        let bar = place(start, 0.0, length, geometry.bar_thickness());
        let depth = geometry.inset();
        let mut cursor = start + metrics::BORDER_WIDTH + dock::PADDING.0;
        let mut icons = Vec::with_capacity(sizes.len());
        let mut divider = None;
        for (index, &side) in sizes.iter().enumerate() {
            if separator == Some(index) {
                let (thickness, across) = dock::SEPARATOR;
                let middle = (geometry.bar_thickness() - across) / 2.0;
                divider = Some(place(
                    cursor + dock::SEPARATOR_MARGIN,
                    middle,
                    thickness,
                    across,
                ));
                cursor += separator_length() + metrics::DOCK_ICON_GAP;
            }
            icons.push(place(cursor, depth, side, side));
            cursor += side + metrics::DOCK_ICON_GAP;
        }
        Self {
            geometry,
            surface,
            bar,
            icons,
            separator: divider,
            along: (start, length),
        }
    }

    /// Position of the pointer along the bar, from 0 to 1, as the board measures it on the bar
    /// as it is drawn at that moment.
    pub fn ratio(&self, point: Point) -> f32 {
        let (start, length) = self.along;
        if length <= 0.0 {
            return 0.5;
        }
        let along = if self.geometry.position.is_vertical() {
            point.y
        } else {
            point.x
        };
        ((along - start) / length).clamp(0.0, 1.0)
    }

    /// Whether `point` is over the dock: the bar, or an icon that grew past it.
    pub fn contains(&self, point: Point) -> bool {
        self.bar.contains(point) || self.icon_at(point).is_some()
    }

    /// The icon under `point`. The gap between two icons belongs to neither.
    pub fn icon_at(&self, point: Point) -> Option<usize> {
        self.icons.iter().position(|icon| icon.contains(point))
    }

    /// The dot of an open application, between its icon and the screen edge.
    pub fn dot(&self, index: usize) -> Option<Rectangle> {
        let icon = self.icons.get(index)?;
        let depth = self.geometry.inset() - dock::DOT_OFFSET;
        let (along, side) = self.along_of(*icon);
        let start = along + (side - dock::DOT) / 2.0;
        Some(to_surface(
            self.geometry.position,
            self.surface,
            start,
            depth,
            dock::DOT,
            dock::DOT,
        ))
    }

    /// Where the label of icon `index` goes: the middle of the icon along the dock, at the
    /// label depth from the edge. The label extends away from the edge from this point.
    pub fn label_anchor(&self, index: usize) -> Option<Point> {
        let icon = self.icons.get(index)?;
        let (along, side) = self.along_of(*icon);
        let middle = along + side / 2.0;
        let depth = self.geometry.label_depth();
        let Size { width, height } = self.surface;
        Some(match self.geometry.position {
            Position::Bottom => Point::new(middle, height - depth),
            Position::Top => Point::new(middle, depth),
            Position::Left => Point::new(depth, middle),
            Position::Right => Point::new(width - depth, middle),
        })
    }

    /// Start and length of `rectangle` along the dock.
    fn along_of(&self, rectangle: Rectangle) -> (f32, f32) {
        if self.geometry.position.is_vertical() {
            (rectangle.y, rectangle.height)
        } else {
            (rectangle.x, rectangle.width)
        }
    }
}

/// A rectangle given along the dock and away from its screen edge, in surface coordinates.
fn to_surface(
    position: Position,
    surface: Size,
    along: f32,
    depth: f32,
    length: f32,
    thickness: f32,
) -> Rectangle {
    let (x, y, width, height) = match position {
        Position::Bottom => (along, surface.height - depth - thickness, length, thickness),
        Position::Top => (along, depth, length, thickness),
        Position::Left => (depth, along, thickness, length),
        Position::Right => (surface.width - depth - thickness, along, thickness, length),
    };
    Rectangle {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The board's script, transcribed as it is (`Dock.dc.html`, `renderVals`).
    fn board(p: f64, i: usize, n: usize) -> f64 {
        let (base, max, reach) = (52.0, 1.7, 1.2);
        let d = p * n as f64 - (i as f64 + 0.5);
        let k = 1.0 + (max - 1.0) * (-(d * d) / (2.0 * reach * reach)).exp();
        (base * k).round()
    }

    fn at(position: Position) -> Geometry {
        Geometry {
            position,
            ..Geometry::default()
        }
    }

    #[test]
    fn the_curve_is_the_board_formula() {
        // The nine icons of the board, at the key points: edges, over an icon, between two.
        let geometry = Geometry::default();
        for p in [0.0, 0.05, 0.1, 0.25, 4.5 / 9.0, 0.5, 0.61, 0.75, 0.95, 1.0] {
            let sizes = geometry.targets(Some(p as f32), 9);
            for (i, size) in sizes.iter().enumerate() {
                assert_eq!(f64::from(*size), board(p, i, 9), "p = {p}, icon {i}");
            }
        }
    }

    #[test]
    fn the_bell_peaks_under_the_pointer() {
        // Pointer right over the middle of the fifth icon (Sherlock on the board).
        let sizes = Geometry::default().targets(Some(4.5 / 9.0), 9);
        assert_eq!(sizes[4], 88.0, "52 × 1.7, rounded");
        // One icon away: 52 × (1 + 0.7 × e^(−1/2.88)) = 77.7; two away: 61.1; three: 54.4.
        assert_eq!((sizes[3], sizes[5]), (78.0, 78.0));
        assert_eq!((sizes[2], sizes[6]), (61.0, 61.0));
        assert_eq!((sizes[1], sizes[7]), (54.0, 54.0));
        assert_eq!((sizes[0], sizes[8]), (52.0, 52.0));
        assert_eq!(Geometry::default().targets(None, 9), vec![52.0; 9]);
    }

    #[test]
    fn the_settings_change_the_curve() {
        // Factor 2, icons of 40: 80 under the pointer.
        let big = Geometry {
            icon: 40.0,
            factor: 2.0,
            ..Geometry::default()
        };
        assert_eq!(big.targets(Some(0.5), 5)[2], 80.0);
        assert_eq!(big.bar_thickness(), 64.0);
        // Without magnification, nothing grows.
        let flat = Geometry {
            factor: 1.0,
            ..Geometry::default()
        };
        assert_eq!(flat.targets(Some(0.5), 5), vec![52.0; 5]);
        assert_eq!(flat.label_depth(), 76.0 + 7.0, "just past the bar");
        assert_eq!(
            Geometry::default().label_depth().round(),
            108.0,
            "the board's 108"
        );
    }

    #[test]
    fn the_label_goes_to_the_closest_icon() {
        assert_eq!(nearest(0.0, 9), 0);
        assert_eq!(nearest(4.5 / 9.0, 9), 4);
        assert_eq!(nearest(0.999, 9), 8);
        assert_eq!(nearest(1.0, 9), 8);
    }

    #[test]
    fn a_grown_icon_pushes_its_neighbours() {
        let geometry = Geometry::default();
        let surface = Size::new(900.0, 132.0);
        let rest = Layout::new(geometry, surface, &[52.0; 5], None);
        // At rest: 2 × (1 + 12) + 5 × 52 + 4 × 8 = 318, centered, icons 60 apart.
        assert_eq!(rest.bar.width, 318.0);
        assert_eq!(rest.bar.x, 291.0);
        assert_eq!(rest.icons[0].x, 304.0);
        assert_eq!(rest.icons[1].x - rest.icons[0].x, 60.0);
        // The bottom of each icon sits 13 above the bottom of the surface.
        assert_eq!(rest.icons[2].y + rest.icons[2].height, 119.0);

        let sizes = geometry.targets(Some(0.5), 5);
        let grown = Layout::new(geometry, surface, &sizes, None);
        let growth: f32 = sizes.iter().map(|s| s - 52.0).sum();
        assert_eq!(grown.bar.width, rest.bar.width + growth);
        // The bar widens equally on both sides: the middle icon keeps its center.
        assert_eq!(grown.icons[2].center_x(), rest.icons[2].center_x());
        // Neighbours keep their gap of 8 and move outwards.
        for pair in grown.icons.windows(2) {
            assert_eq!(pair[1].x - (pair[0].x + pair[0].width), 8.0);
        }
        assert!(grown.icons[0].x < rest.icons[0].x);
        assert!(grown.icons[4].x > rest.icons[4].x);
        // The grown icon goes past the top of the bar, bottoms stay aligned.
        assert!(grown.icons[2].y < grown.bar.y);
        assert_eq!(grown.icons[2].y + grown.icons[2].height, 119.0);
    }

    #[test]
    fn the_separator_takes_its_place_in_the_bar() {
        let geometry = Geometry::default();
        let surface = Size::new(900.0, 132.0);
        let layout = Layout::new(geometry, surface, &[52.0; 4], Some(3));
        // One more slot: 5 for the separator and a gap of 8.
        assert_eq!(layout.bar.width, 26.0 + 4.0 * 52.0 + 3.0 * 8.0 + 5.0 + 8.0);
        let separator = layout.separator.expect("separator");
        assert_eq!((separator.width, separator.height), (1.0, 40.0));
        assert_eq!(separator.x, layout.icons[2].x + 52.0 + 8.0 + 2.0);
        assert_eq!(layout.icons[3].x, separator.x + 1.0 + 2.0 + 8.0);
        // Centered across the bar.
        assert_eq!(separator.center_y(), layout.bar.center_y());
    }

    #[test]
    fn hit_testing_follows_the_drawn_icons() {
        let geometry = Geometry::default();
        let surface = Size::new(900.0, 132.0);
        let sizes = geometry.targets(Some(0.5), 5);
        let layout = Layout::new(geometry, surface, &sizes, None);
        let top = layout.icons[2];
        // Above the bar, on the grown icon.
        let point = Point::new(top.center_x(), top.y + 2.0);
        assert!(point.y < layout.bar.y);
        assert_eq!(layout.icon_at(point), Some(2));
        assert!(layout.contains(point));
        // Above the bar, beside the grown icon: outside the dock.
        assert!(!layout.contains(Point::new(layout.icons[0].x, top.y + 2.0)));
        // In the gap between two icons: on the bar, on no icon.
        let gap = Point::new(layout.icons[1].x - 4.0, 100.0);
        assert_eq!(layout.icon_at(gap), None);
        assert!(layout.contains(gap));
        assert_eq!(layout.ratio(Point::new(layout.bar.x - 10.0, 100.0)), 0.0);
        assert_eq!(layout.ratio(layout.bar.center()), 0.5);
    }

    #[test]
    fn each_edge_mirrors_the_bottom_dock() {
        let count = 5;
        let bottom = at(Position::Bottom);
        let surface = bottom.surface_size(count, false);
        assert_eq!(surface, Size::new(529.0, 132.0));
        let reference = Layout::new(bottom, surface, &bottom.targets(Some(0.3), count), None);
        for position in [Position::Top, Position::Left, Position::Right] {
            let geometry = at(position);
            let size = geometry.surface_size(count, false);
            let sizes = geometry.targets(Some(0.3), count);
            let layout = Layout::new(geometry, size, &sizes, None);
            for (icon, expected) in layout.icons.iter().zip(&reference.icons) {
                // Along the dock: the same places; away from the edge: the same distances.
                let (along, length, depth) = match position {
                    Position::Top => (icon.x, icon.width, icon.y),
                    Position::Left => (icon.y, icon.height, icon.x),
                    _ => (icon.y, icon.height, size.width - icon.x - icon.width),
                };
                let expected_depth = surface.height - expected.y - expected.height;
                assert_eq!(
                    (along, length),
                    (expected.x, expected.width),
                    "{position:?}"
                );
                assert_eq!(depth, expected_depth, "{position:?}");
            }
            // The bar against the edge, the exclusive zone its thickness.
            match position {
                Position::Top => assert_eq!((layout.bar.y, layout.bar.height), (0.0, 76.0)),
                Position::Left => assert_eq!((layout.bar.x, layout.bar.width), (0.0, 76.0)),
                _ => assert_eq!(layout.bar.x + layout.bar.width, size.width),
            }
        }
        // On the sides the surface is tall, with room for the label beside the icons.
        let left = at(Position::Left).surface_size(count, false);
        assert_eq!(left.height, 529.0);
        assert!(left.width >= 108.0 + dock::LABEL_SIDE_ROOM);
    }

    #[test]
    fn on_the_sides_the_magnification_follows_the_vertical_axis() {
        let geometry = at(Position::Left);
        let surface = geometry.surface_size(5, false);
        let rest = Layout::new(geometry, surface, &[52.0; 5], None);
        // The pointer over the fourth icon: its height along the dock decides, not its x.
        let point = Point::new(30.0, rest.icons[3].center_y());
        let ratio = rest.ratio(point);
        assert_eq!(nearest(ratio, 5), 3);
        assert_eq!(rest.ratio(Point::new(500.0, point.y)), ratio);
        let grown = Layout::new(geometry, surface, &geometry.targets(Some(ratio), 5), None);
        assert!(grown.icons[3].width > 80.0);
        // Grown away from the left edge, towards the label on the right.
        assert_eq!(grown.icons[3].x, 13.0);
        let label = grown.label_anchor(3).expect("label");
        assert_eq!(label.y, grown.icons[3].center_y());
        assert!(label.x > grown.icons[3].x + grown.icons[3].width);
        // The dot between the icon and the edge.
        let dot = grown.dot(3).expect("dot");
        assert_eq!((dot.x, dot.width), (4.0, 5.0));
    }

    #[test]
    fn the_surface_holds_the_widest_dock_and_the_label() {
        let geometry = Geometry::default();
        let size = geometry.surface_size(5, false);
        let widest = (0..=100)
            .map(|s| geometry.bar_length(&geometry.targets(Some(s as f32 / 100.0), 5), false))
            .fold(0.0, f32::max);
        assert!(size.width >= widest + 2.0 * dock::SURFACE_SLACK);
        // Label: 108 above the bar bottom, plus its own height; above the tallest icon.
        assert!(size.height > 108.0 + 2.0 * dock::LABEL_PADDING.0);
        assert!(size.height > 13.0 + 52.0 * 1.7 * 1.08);
    }
}
