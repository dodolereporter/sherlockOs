//! Geometry of the dock: the magnification curve of the Dock design board, the place of each
//! icon, hit testing. Pure computation, in logical pixels of the dock surface, whose origin is
//! its top left corner.

use iced::{Point, Rectangle, Size};
use sherlock_ui::{dock, metrics};

/// Scale of icon `index` among `count` when the pointer is at `p` along the bar, 0 at its left
/// edge and 1 at its right edge. The formula of the board's script:
/// `d = p × n − (i + 0.5)`, `k = 1 + (max − 1) × exp(−d² / (2 × reach²))`.
pub fn scale(p: f32, index: usize, count: usize) -> f32 {
    let d = p * count as f32 - (index as f32 + 0.5);
    let reach = metrics::DOCK_MAGNIFICATION_REACH;
    1.0 + (metrics::DOCK_MAGNIFICATION - 1.0) * (-(d * d) / (2.0 * reach * reach)).exp()
}

/// Size each icon heads for: the base size without pointer, otherwise the base size times its
/// scale, rounded to the pixel as on the board.
pub fn targets(p: Option<f32>, count: usize) -> Vec<f32> {
    (0..count)
        .map(|i| match p {
            Some(p) => (metrics::DOCK_ICON * scale(p, i, count)).round(),
            None => metrics::DOCK_ICON,
        })
        .collect()
}

/// The icon whose label shows: the closest one to the pointer, as the board picks it.
pub fn nearest(p: f32, count: usize) -> usize {
    let index = (p * count as f32).floor().max(0.0) as usize;
    index.min(count.saturating_sub(1))
}

/// Width of the bar holding icons of `sizes`: border, padding, icons and the gaps between them.
pub fn bar_width(sizes: &[f32]) -> f32 {
    let gaps = sizes.len().saturating_sub(1) as f32 * metrics::DOCK_ICON_GAP;
    2.0 * (metrics::BORDER_WIDTH + dock::PADDING.0) + sizes.iter().sum::<f32>() + gaps
}

/// Size of the dock surface for `count` icons: as wide as the widest magnified bar plus room for
/// the label at the edges, as high as the label above the bar.
pub fn surface_size(count: usize) -> Size {
    let widest = (0..=1000)
        .map(|step| bar_width(&targets(Some(step as f32 / 1000.0), count)))
        .fold(bar_width(&targets(None, count)), f32::max);
    let label =
        2.0 * dock::LABEL_PADDING.0 + dock::LABEL_SIZE * sherlock_ui::typography::LINE_HEIGHT;
    let height = metrics::BORDER_WIDTH + dock::LABEL_BOTTOM + label;
    Size::new((widest + 2.0 * dock::SURFACE_SLACK).ceil(), height.ceil())
}

/// Where everything is, for icons of the given sizes.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// The glass bar, 76 high, at the bottom of the surface and centered.
    pub bar: Rectangle,
    /// Each icon, bottoms aligned on the bottom padding of the bar. A magnified icon goes past
    /// the top of the bar.
    pub icons: Vec<Rectangle>,
}

impl Layout {
    /// Lays out icons of `sizes` in a surface of `surface`. The bar is as wide as its icons:
    /// when one grows, the bar widens on both sides and pushes its neighbours away.
    pub fn new(surface: Size, sizes: &[f32]) -> Self {
        let width = bar_width(sizes);
        let bar = Rectangle {
            x: (surface.width - width) / 2.0,
            y: surface.height - metrics::DOCK_HEIGHT,
            width,
            height: metrics::DOCK_HEIGHT,
        };
        let bottom = surface.height - metrics::BORDER_WIDTH - dock::PADDING.1;
        let mut x = bar.x + metrics::BORDER_WIDTH + dock::PADDING.0;
        let icons = sizes
            .iter()
            .map(|&side| {
                let icon = Rectangle {
                    x,
                    y: bottom - side,
                    width: side,
                    height: side,
                };
                x += side + metrics::DOCK_ICON_GAP;
                icon
            })
            .collect();
        Self { bar, icons }
    }

    /// Position of the pointer along the bar, from 0 to 1, as the board measures it on the bar
    /// as it is drawn at that moment.
    pub fn ratio(&self, x: f32) -> f32 {
        if self.bar.width <= 0.0 {
            return 0.5;
        }
        ((x - self.bar.x) / self.bar.width).clamp(0.0, 1.0)
    }

    /// Whether `point` is over the dock: the bar, or an icon that grew past it.
    pub fn contains(&self, point: Point) -> bool {
        self.bar.contains(point) || self.icon_at(point).is_some()
    }

    /// The icon under `point`. The gap between two icons belongs to neither.
    pub fn icon_at(&self, point: Point) -> Option<usize> {
        self.icons.iter().position(|icon| icon.contains(point))
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

    #[test]
    fn the_curve_is_the_board_formula() {
        // The nine icons of the board, at the key points: edges, over an icon, between two.
        for p in [0.0, 0.05, 0.1, 0.25, 4.5 / 9.0, 0.5, 0.61, 0.75, 0.95, 1.0] {
            let sizes = targets(Some(p as f32), 9);
            for (i, size) in sizes.iter().enumerate() {
                assert_eq!(f64::from(*size), board(p, i, 9), "p = {p}, icon {i}");
            }
        }
    }

    #[test]
    fn the_bell_peaks_under_the_pointer() {
        // Pointer right over the middle of the fifth icon (Sherlock on the board).
        let sizes = targets(Some(4.5 / 9.0), 9);
        assert_eq!(sizes[4], 88.0, "52 × 1.7, rounded");
        // One icon away: 52 × (1 + 0.7 × e^(−1/2.88)) = 77.7; two away: 61.1; three: 54.4.
        assert_eq!((sizes[3], sizes[5]), (78.0, 78.0));
        assert_eq!((sizes[2], sizes[6]), (61.0, 61.0));
        assert_eq!((sizes[1], sizes[7]), (54.0, 54.0));
        assert_eq!((sizes[0], sizes[8]), (52.0, 52.0));
        assert_eq!(targets(None, 9), vec![52.0; 9]);
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
        let surface = Size::new(900.0, 132.0);
        let rest = Layout::new(surface, &[52.0; 5]);
        // At rest: 2 × (1 + 12) + 5 × 52 + 4 × 8 = 318, centered, icons 60 apart.
        assert_eq!(rest.bar.width, 318.0);
        assert_eq!(rest.bar.x, 291.0);
        assert_eq!(rest.icons[0].x, 304.0);
        assert_eq!(rest.icons[1].x - rest.icons[0].x, 60.0);
        // The bottom of each icon sits 13 above the bottom of the surface.
        assert_eq!(rest.icons[2].y + rest.icons[2].height, 119.0);

        let sizes = targets(Some(0.5), 5);
        let grown = Layout::new(surface, &sizes);
        let growth: f32 = sizes.iter().map(|s| s - 52.0).sum();
        assert_eq!(grown.bar.width, rest.bar.width + growth);
        // The bar widens equally on both sides: the middle icon keeps its center.
        let center = |r: Rectangle| r.x + r.width / 2.0;
        assert_eq!(center(grown.icons[2]), center(rest.icons[2]));
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
    fn hit_testing_follows_the_drawn_icons() {
        let surface = Size::new(900.0, 132.0);
        let sizes = targets(Some(0.5), 5);
        let layout = Layout::new(surface, &sizes);
        let top = layout.icons[2];
        // Above the bar, on the grown icon.
        let point = Point::new(top.x + top.width / 2.0, top.y + 2.0);
        assert!(point.y < layout.bar.y);
        assert_eq!(layout.icon_at(point), Some(2));
        assert!(layout.contains(point));
        // Above the bar, beside the grown icon: outside the dock.
        assert!(!layout.contains(Point::new(layout.icons[0].x, top.y + 2.0)));
        // In the gap between two icons: on the bar, on no icon.
        let gap = Point::new(layout.icons[1].x - 4.0, 100.0);
        assert_eq!(layout.icon_at(gap), None);
        assert!(layout.contains(gap));
        assert_eq!(layout.ratio(layout.bar.x - 10.0), 0.0);
        assert_eq!(layout.ratio(layout.bar.x + layout.bar.width / 2.0), 0.5);
    }

    #[test]
    fn the_surface_holds_the_widest_dock_and_the_label() {
        let size = surface_size(5);
        let widest = (0..=100)
            .map(|s| bar_width(&targets(Some(s as f32 / 100.0), 5)))
            .fold(0.0, f32::max);
        assert!(size.width >= widest + 2.0 * dock::SURFACE_SLACK);
        // Label: 108 above the bar bottom, plus its own height; above the tallest icon.
        assert!(size.height > dock::LABEL_BOTTOM + 2.0 * dock::LABEL_PADDING.0);
        assert!(size.height > 13.0 + 52.0 * 1.7 * 1.08);
    }
}
