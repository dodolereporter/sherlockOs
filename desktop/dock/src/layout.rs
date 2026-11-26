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
        let along = (self.longest(count, separator) + 2.0 * dock::SURFACE_SLACK).ceil();
        let depth = self.reach().ceil();
        if self.position.is_vertical() {
            Size::new(depth, along)
        } else {
            Size::new(along, depth)
        }
    }

    /// The longest the bar gets, over every pointer position.
    fn longest(&self, count: usize, separator: bool) -> f32 {
        (0..=1000)
            .map(|step| {
                self.bar_length(&self.targets(Some(step as f32 / 1000.0), count), separator)
            })
            .fold(
                self.bar_length(&self.targets(None, count), separator),
                f32::max,
            )
    }

    /// How far from the edge the dock may draw: the label past the magnified icons, or beside
    /// them on the left and right edges.
    fn reach(&self) -> f32 {
        let room = if self.position.is_vertical() {
            dock::LABEL_SIDE_ROOM
        } else {
            Self::label_height()
        };
        self.label_depth() + room
    }

    /// Where the surface takes the pointer (`wl_surface.set_input_region`). At rest, the bar
    /// only: a click in the empty room past it (56 px above a bottom dock) goes to the window
    /// below. While hovered, the area the dock may draw in: the longest magnified bar, up to
    /// the label, so that a grown icon stays reachable and the dock sees the pointer leave.
    /// The room at both ends of the surface, kept for the label of an edge icon, never takes
    /// the pointer.
    pub fn input_region(
        &self,
        surface: Size,
        count: usize,
        separator: Option<usize>,
        hovering: bool,
    ) -> Rectangle {
        let rest = Layout::new(*self, surface, &self.targets(None, count), separator).bar;
        if !hovering {
            return rest;
        }
        let span = if self.position.is_vertical() {
            surface.height
        } else {
            surface.width
        };
        let length = self.longest(count, separator.is_some());
        let depth = self.reach().min(if self.position.is_vertical() {
            surface.width
        } else {
            surface.height
        });
        to_surface(
            self.position,
            surface,
            (span - length) / 2.0,
            0.0,
            length,
            depth,
        )
    }

    /// Where a hidden dock takes the pointer: the band of `HIDE_STRIP` the screen still shows
    /// of the surface, along its side away from the edge, over its whole length.
    pub fn strip(&self, surface: Size) -> Rectangle {
        let (span, deep) = if self.position.is_vertical() {
            (surface.height, surface.width)
        } else {
            (surface.width, surface.height)
        };
        let strip = dock::HIDE_STRIP.min(deep);
        to_surface(self.position, surface, 0.0, deep - strip, span, strip)
    }
}

/// `icon`, drawn in a surface of `surface` at `distance` from the `position` edge of a screen
/// of `screen` (logical width and height), relative to the top left corner of the screen. The
/// surface is centered along its edge as the compositor centers it, halves rounded down.
pub fn on_screen(
    position: Position,
    surface: Size,
    screen: (i32, i32),
    distance: i32,
    icon: Rectangle,
) -> (i32, i32, i32, i32) {
    let (width, height) = (surface.width.round() as i32, surface.height.round() as i32);
    let (x, y) = match position {
        Position::Bottom => (screen.0 / 2 - width / 2, screen.1 - distance - height),
        Position::Top => (screen.0 / 2 - width / 2, distance),
        Position::Left => (distance, screen.1 / 2 - height / 2),
        Position::Right => (screen.0 - distance - width, screen.1 / 2 - height / 2),
    };
    let (left, top, w, h) = pixels(icon);
    (x + left, y + top, w, h)
}

/// `rectangle` in whole pixels, as `x, y, width, height`, covering all of it.
pub fn pixels(rectangle: Rectangle) -> (i32, i32, i32, i32) {
    let (left, top) = (rectangle.x.floor(), rectangle.y.floor());
    let right = (rectangle.x + rectangle.width).ceil();
    let bottom = (rectangle.y + rectangle.height).ceil();
    (
        left as i32,
        top as i32,
        (right - left) as i32,
        (bottom - top) as i32,
    )
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

/// An icon in the flow of the dock, as its animations have it at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// Side of the icon.
    pub side: f32,
    /// Share of its room it takes along the dock: 0 while it arrives or once it left, 1 in
    /// place. Its neighbours move aside as it grows.
    pub room: f32,
    /// Offset of the icon from its place: along the dock, and away from the screen edge.
    pub shift: (f32, f32),
}

impl Slot {
    /// An icon of `side` in its place.
    pub fn at_rest(side: f32) -> Self {
        Self {
            side,
            room: 1.0,
            shift: (0.0, 0.0),
        }
    }
}

/// What the dock holds at one instant.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flow {
    /// The icons, in order.
    pub slots: Vec<Slot>,
    /// Icons leaving the dock, still closing their room: the icon each comes before
    /// (`slots.len()` at the end), whether it was after the separator, and its slot.
    pub leaving: Vec<(usize, bool, Slot)>,
    /// The separator before icon `.0`, taking `.1` of its room.
    pub separator: Option<(usize, f32)>,
    /// The icon the pointer drags, and the pointer: the icon is drawn under it, its room stays
    /// in the bar.
    pub held: Option<(usize, Point)>,
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
    /// The icons leaving the dock, in the order of `Flow::leaving`.
    pub leaving: Vec<Rectangle>,
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
        let flow = Flow {
            slots: sizes.iter().map(|&side| Slot::at_rest(side)).collect(),
            separator: separator.map(|index| (index, 1.0)),
            ..Flow::default()
        };
        Self::flow(geometry, surface, &flow)
    }

    /// Lays out `flow`: each icon takes its share of its room, leaving icons included, and is
    /// drawn at its offset; the held icon is drawn under the pointer, inside the surface.
    pub fn flow(geometry: Geometry, surface: Size, flow: &Flow) -> Self {
        let vertical = geometry.position.is_vertical();
        let (span, deep) = if vertical {
            (surface.height, surface.width)
        } else {
            (surface.width, surface.height)
        };
        let gap = metrics::DOCK_ICON_GAP;
        let weight = |slot: &Slot| (slot.side + gap) * slot.room;
        let (index, room) = flow.separator.unwrap_or((usize::MAX, 0.0));
        let icons_length: f32 = flow
            .slots
            .iter()
            .chain(flow.leaving.iter().map(|(_, _, slot)| slot))
            .map(weight)
            .sum();
        let ends = 2.0 * (metrics::BORDER_WIDTH + dock::PADDING.0);
        let length = ends + (icons_length - gap).max(0.0) + room * (separator_length() + gap);
        let start = (span - length) / 2.0;
        let place = |along: f32, depth: f32, length: f32, thickness: f32| {
            to_surface(geometry.position, surface, along, depth, length, thickness)
        };
        let bar = place(start, 0.0, length, geometry.bar_thickness());
        let inset = geometry.inset();
        let mut cursor = start + metrics::BORDER_WIDTH + dock::PADDING.0;
        let put = |cursor: &mut f32, slot: &Slot| {
            let middle = *cursor + slot.side * slot.room / 2.0;
            *cursor += weight(slot);
            let depth = (inset + slot.shift.1).clamp(0.0, (deep - slot.side).max(0.0));
            place(
                middle - slot.side / 2.0 + slot.shift.0,
                depth,
                slot.side,
                slot.side,
            )
        };
        let mut icons = Vec::with_capacity(flow.slots.len());
        let mut leaving = vec![Rectangle::default(); flow.leaving.len()];
        let mut divider = None;
        for at in 0..=flow.slots.len() {
            // The icons that leave before this one: those of the pinned part before the
            // separator, the open ones after it.
            for after in [false, true] {
                if after && index == at {
                    let (thickness, across) = dock::SEPARATOR;
                    let middle = (geometry.bar_thickness() - across) / 2.0;
                    let margin = dock::SEPARATOR_MARGIN * room;
                    divider = Some(place(cursor + margin, middle, thickness, across));
                    cursor += room * (separator_length() + gap);
                }
                for (rank, (_, _, slot)) in flow
                    .leaving
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| l.0 == at && l.1 == after)
                {
                    leaving[rank] = put(&mut cursor, slot);
                }
            }
            if let Some(slot) = flow.slots.get(at) {
                icons.push(put(&mut cursor, slot));
            }
        }
        if let Some((held, point)) = flow.held
            && let Some(icon) = icons.get_mut(held)
        {
            let (along, depth) = along_depth(geometry.position, surface, point);
            let side = if vertical { icon.height } else { icon.width };
            let along = (along - side / 2.0).clamp(0.0, (span - side).max(0.0));
            let depth = (depth - side / 2.0).clamp(inset, (deep - side).max(inset));
            *icon = place(along, depth, side, side);
        }
        Self {
            geometry,
            surface,
            bar,
            icons,
            leaving,
            separator: divider,
            along: (start, length),
        }
    }

    /// Where `offset` (`Layout::offset`) is in this surface, for an icon of `side`.
    pub fn at_offset(&self, offset: (f32, f32), side: f32) -> Rectangle {
        let span = if self.geometry.position.is_vertical() {
            self.surface.height
        } else {
            self.surface.width
        };
        let (along, depth) = (offset.0 + span / 2.0, offset.1);
        to_surface(
            self.geometry.position,
            self.surface,
            along,
            depth,
            side,
            side,
        )
    }

    /// The middle of icon `index` along the dock.
    pub fn middle(&self, index: usize) -> Option<f32> {
        let (along, side) = self.along_of(*self.icons.get(index)?);
        Some(along + side / 2.0)
    }

    /// `point` along the dock.
    pub fn along(&self, point: Point) -> f32 {
        along_depth(self.geometry.position, self.surface, point).0
    }

    /// Distance of `point` from the screen edge.
    pub fn depth(&self, point: Point) -> f32 {
        along_depth(self.geometry.position, self.surface, point).1
    }

    /// Place of `rectangle` relative to the middle of the dock, which stays put on the screen
    /// when the surface changes size: along the dock, and away from the edge.
    pub fn offset(&self, rectangle: Rectangle) -> (f32, f32) {
        let corner = Point::new(rectangle.x, rectangle.y);
        let (along, depth) = along_depth(self.geometry.position, self.surface, corner);
        let (span, side) = if self.geometry.position.is_vertical() {
            (self.surface.height, rectangle.width)
        } else {
            (self.surface.width, rectangle.height)
        };
        // The corner of a rectangle at the bottom or right is its far side from the edge.
        let depth = match self.geometry.position {
            Position::Bottom | Position::Right => depth - side,
            Position::Top | Position::Left => depth,
        };
        (along - span / 2.0, depth)
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

    /// The band a hovered dock keeps the pointer in: along the bar as it is drawn, from the
    /// screen edge to the top of the tallest icon. The gaps between grown icons are in it, so
    /// the magnification does not drop when the pointer crosses one.
    pub fn hover_band(&self) -> Rectangle {
        let (start, length) = self.along;
        let tallest = self
            .icons
            .iter()
            .map(|icon| icon.width.max(icon.height))
            .fold(0.0, f32::max);
        let depth = self
            .geometry
            .bar_thickness()
            .max(self.geometry.inset() + tallest);
        to_surface(
            self.geometry.position,
            self.surface,
            start,
            0.0,
            length,
            depth,
        )
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

/// `point` along the dock, and its distance from the screen edge.
fn along_depth(position: Position, surface: Size, point: Point) -> (f32, f32) {
    match position {
        Position::Bottom => (point.x, surface.height - point.y),
        Position::Top => (point.x, point.y),
        Position::Left => (point.y, point.x),
        Position::Right => (point.y, surface.width - point.x),
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
    fn the_gap_between_grown_icons_keeps_the_hover() {
        let geometry = Geometry::default();
        let surface = Size::new(900.0, 132.0);
        let sizes = geometry.targets(Some(0.5), 5);
        let layout = Layout::new(geometry, surface, &sizes, None);
        // Above the bar, in the gap between the grown middle icon and its left neighbour.
        let (left, middle) = (layout.icons[1], layout.icons[2]);
        let gap = Point::new(left.x + left.width + 4.0, middle.y + 20.0);
        assert!(
            gap.y < layout.bar.y && gap.y > left.y,
            "above the bar, beside both icons"
        );
        assert!(!layout.contains(gap), "on neither icon nor the bar");
        assert_eq!(layout.icon_at(gap), None, "a click there picks nothing");
        assert!(layout.hover_band().contains(gap), "but the hover holds");
        // Above the tallest icon, or past the ends of the bar: out.
        assert!(
            !layout
                .hover_band()
                .contains(Point::new(gap.x, middle.y - 2.0))
        );
        assert!(
            !layout
                .hover_band()
                .contains(Point::new(layout.bar.x - 2.0, 100.0))
        );
    }

    #[test]
    fn the_input_region_lets_clicks_through_around_the_dock() {
        let geometry = Geometry::default();
        let surface = geometry.surface_size(5, false);
        // At rest: the bar only, 318 by 76 at the bottom; the 56 px above it let clicks go
        // through to the windows, and so do the ends of the surface.
        let rest = geometry.input_region(surface, 5, None, false);
        assert_eq!(
            rest,
            Rectangle::new(Point::new(105.5, 56.0), Size::new(318.0, 76.0))
        );
        // Whole pixels for the protocol, never smaller than the bar.
        assert_eq!(pixels(rest), (105, 56, 319, 76));
        assert!(!rest.contains(Point::new(264.0, 40.0)), "above the bar");
        // Hovered: the longest magnified bar, up to the top of the label.
        let hovered = geometry.input_region(surface, 5, None, true);
        let (_, top, _, height) = pixels(hovered);
        assert_eq!((top, height), (0, 132), "up to the top of the label");
        assert_eq!(hovered.width, 425.0);
        assert_eq!(hovered.center_x(), surface.width / 2.0);
        assert!(
            hovered.x >= dock::SURFACE_SLACK,
            "the label room at the ends stays out"
        );
        for step in 0..=20 {
            let sizes = geometry.targets(Some(step as f32 / 20.0), 5);
            let layout = Layout::new(geometry, surface, &sizes, None);
            for icon in &layout.icons {
                assert!(
                    hovered.contains(icon.position()),
                    "magnified icon reachable"
                );
            }
            assert!(hovered.contains(layout.label_anchor(2).expect("label")));
        }
        // On the right edge: the bar against the edge at rest, the side room while hovered.
        let right = Geometry {
            position: Position::Right,
            ..geometry
        };
        let surface = right.surface_size(5, false);
        let rest = right.input_region(surface, 5, None, false);
        assert_eq!((rest.x + rest.width, rest.width), (surface.width, 76.0));
        let hovered = right.input_region(surface, 5, None, true);
        let (left, _, width, _) = pixels(hovered);
        assert_eq!((left as f32, width as f32), (0.0, surface.width));
        assert_eq!(hovered.height, 425.0);
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
