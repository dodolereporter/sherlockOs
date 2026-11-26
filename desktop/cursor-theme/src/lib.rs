//! The cursor theme of the desktop: its shapes and every name they answer to, how a drawing is
//! read and painted, and the Xcursor format the themes are written in.
//!
//! The drawings are the SVGs of `assets/cursors/`, from the author's cursor boards, on a grid
//! of 24 units: one unit is one pixel at the size 24. Each SVG says its hotspot
//! (`data-hotspot="x y"` on the root) and paints its parts with the classes of its
//! `<style id="theme">` block: `body`, `edge`, `ink`, `track`. A group `id="spin"` with a
//! `data-center="x y"` turns, for the waiting cursors. Replacing a drawing is copying its SVG.
//!
//! The generator (`cursor-theme`, feature `render`) writes two themes: `nom-os` for the dark
//! desktop and `nom-os-light` for the light one. See `docs/cursor.md`.

pub mod xcursor;

use std::{fmt, time::Duration};

use sherlock_ui::{
    color::Color,
    cursor::{self, CursorColors},
};

/// Theme of the dark desktop, the default: cream cursors.
pub const THEME: &str = "nom-os";
/// Theme of the light desktop: night blue cursors.
pub const LIGHT_THEME: &str = "nom-os-light";

/// Side of the drawing grid of the SVGs.
pub const GRID: u32 = 24;
/// Nominal sizes written in each file: the size 24 at the scales 1, 1.33, 2, 2.67 and 4.
pub const SIZES: [u32; 5] = [24, 32, 48, 64, 96];

/// Side of the image of the nominal `size`: a quarter more than the grid, for the outlines
/// that go past it and for the badges that go past it at the bottom right, as in the boards.
pub fn canvas(size: u32) -> u32 {
    size * 5 / 4
}

/// Where the grid starts in the image of the nominal `size`: two units in, rounded up to a
/// whole pixel so that the drawing stays on the pixels of its grid.
pub fn margin(size: u32) -> u32 {
    (2 * size).div_ceil(GRID)
}

/// Images in a turn of the waiting cursors, and the time each one is shown.
pub const SPIN_FRAMES: u32 = 30;
pub fn spin_delay() -> Duration {
    cursor::SPIN_TURN / SPIN_FRAMES
}

/// A cursor shape: its drawing, `assets/cursors/<name>.svg`, and the names it answers to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Name of the file: the CSS name `wp_cursor_shape_v1` gives, `sherlock` for ours.
    pub name: &'static str,
    /// Other names, CSS and legacy X11: links to the file in the theme.
    pub aliases: &'static [&'static str],
}

const fn shape_of(name: &'static str, aliases: &'static [&'static str]) -> Shape {
    Shape { name, aliases }
}

/// Every shape of the theme. The boards draw 17 of them: the CSS names without a drawing are
/// aliases of the closest one (a column edge moves as a window edge, a cell is aimed at as a
/// point).
pub const SHAPES: &[Shape] = &[
    shape_of(
        "default",
        &[
            "left_ptr",
            "arrow",
            "top_left_arrow",
            "left_arrow",
            "context-menu",
            "help",
            "question_arrow",
            "whats_this",
            "left_ptr_help",
            "alias",
            "link",
            "dnd-link",
            "dnd-ask",
        ],
    ),
    shape_of("pointer", &["hand2", "hand1", "hand", "pointing_hand"]),
    shape_of("text", &["xterm", "ibeam", "vertical-text"]),
    shape_of("wait", &["watch"]),
    shape_of("progress", &["left_ptr_watch", "half-busy"]),
    shape_of(
        "ew-resize",
        &[
            "e-resize",
            "w-resize",
            "col-resize",
            "right_side",
            "left_side",
            "split_h",
            "h_double_arrow",
            "sb_h_double_arrow",
            "size_hor",
        ],
    ),
    shape_of(
        "ns-resize",
        &[
            "n-resize",
            "s-resize",
            "row-resize",
            "top_side",
            "bottom_side",
            "split_v",
            "v_double_arrow",
            "sb_v_double_arrow",
            "size_ver",
        ],
    ),
    shape_of(
        "nwse-resize",
        &[
            "nw-resize",
            "se-resize",
            "top_left_corner",
            "bottom_right_corner",
            "bd_double_arrow",
            "size_fdiag",
        ],
    ),
    shape_of(
        "nesw-resize",
        &[
            "ne-resize",
            "sw-resize",
            "top_right_corner",
            "bottom_left_corner",
            "fd_double_arrow",
            "size_bdiag",
        ],
    ),
    shape_of(
        "move",
        &["all-resize", "all-scroll", "fleur", "size_all", "dnd-move"],
    ),
    shape_of("grabbing", &["grab", "openhand", "closedhand", "dnd-none"]),
    shape_of(
        "not-allowed",
        &[
            "no-drop",
            "crossed_circle",
            "forbidden",
            "circle",
            "dnd-no-drop",
        ],
    ),
    shape_of("crosshair", &["cross", "tcross", "cell", "plus"]),
    shape_of("zoom-in", &[]),
    shape_of("zoom-out", &[]),
    shape_of("copy", &["dnd-copy"]),
    shape_of("sherlock", &[]),
];

/// The shape that answers to `name`, by its own name or one of its aliases.
pub fn shape(name: &str) -> Option<&'static Shape> {
    SHAPES
        .iter()
        .find(|s| s.name == name || s.aliases.contains(&name))
}

/// What an SVG says of itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drawing {
    /// The point of the cursor, in grid units.
    pub hotspot: (f32, f32),
    /// Center of the group that turns, in grid units.
    pub spin: Option<(f32, f32)>,
}

/// An SVG that lacks what the generator needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingError(String);

impl fmt::Display for DrawingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DrawingError {}

/// The two numbers of the attribute `name` in the first tag that holds `marker`.
fn point(svg: &str, marker: &str, name: &str) -> Result<Option<(f32, f32)>, DrawingError> {
    let Some(tag_start) = svg.find(marker) else {
        return Ok(None);
    };
    let tag = &svg[tag_start
        ..svg[tag_start..]
            .find('>')
            .map_or(svg.len(), |e| tag_start + e)];
    let key = format!("{name}=\"");
    let value = tag
        .find(&key)
        .map(|at| &tag[at + key.len()..])
        .and_then(|rest| rest.split('"').next())
        .ok_or_else(|| DrawingError(format!("no {name} next to {marker}")))?;
    let numbers: Vec<f32> = value
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| DrawingError(format!("{name}=\"{value}\" is not two numbers")))?;
    match numbers.as_slice() {
        [x, y] => Ok(Some((*x, *y))),
        _ => Err(DrawingError(format!(
            "{name}=\"{value}\" is not two numbers"
        ))),
    }
}

impl Drawing {
    pub fn read(svg: &str) -> Result<Self, DrawingError> {
        let hotspot = point(svg, "<svg", "data-hotspot")?
            .ok_or_else(|| DrawingError("no <svg> root".into()))?;
        let spin = point(svg, SPIN, "data-center")?;
        if !svg.contains(STYLE) {
            return Err(DrawingError(format!("no {STYLE} block")));
        }
        Ok(Self { hotspot, spin })
    }

    /// The hotspot in the image of the nominal `size`: the pixel under the point.
    pub fn hotspot_at(&self, size: u32) -> (u32, u32) {
        let scale = |v: f32| margin(size) + (v * size as f32 / GRID as f32).floor().max(0.0) as u32;
        (scale(self.hotspot.0), scale(self.hotspot.1))
    }
}

const SPIN: &str = "id=\"spin\"";
const STYLE: &str = "<style id=\"theme\">";

/// The two themes written by the generator, one per desktop theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Dark,
    Light,
}

impl Variant {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Light];

    /// Name of the theme folder.
    pub fn theme(self) -> &'static str {
        match self {
            Self::Dark => THEME,
            Self::Light => LIGHT_THEME,
        }
    }

    pub fn colors(self) -> CursorColors {
        match self {
            Self::Dark => cursor::DARK,
            Self::Light => cursor::LIGHT,
        }
    }

    /// The rules of the `<style id="theme">` block, the drawing's colors in this theme.
    pub fn stylesheet(self) -> String {
        let c = self.colors();
        format!(
            "\n    .body {{ fill: {} }}\n    .edge {{ stroke: {} }}\n    .ink {{ stroke: {} }}\n    \
             .track {{ stroke: {}; stroke-opacity: {} }}\n  ",
            hex(c.body),
            hex(c.edge),
            hex(c.body),
            hex(c.track),
            c.track.a
        )
    }

    /// The SVG of a drawing in this theme, its spinning group turned by `angle` degrees.
    pub fn paint(self, svg: &str, drawing: &Drawing, angle: f32) -> String {
        let mut svg = match svg.find(STYLE).map(|at| at + STYLE.len()) {
            Some(start) => {
                let end = svg[start..].find("</style>").map_or(start, |e| start + e);
                format!("{}{}{}", &svg[..start], self.stylesheet(), &svg[end..])
            }
            None => svg.to_owned(),
        };
        if let Some((x, y)) = drawing.spin {
            svg = svg.replacen(
                SPIN,
                &format!("{SPIN} transform=\"rotate({angle} {x} {y})\""),
                1,
            );
        }
        svg
    }
}

/// `#RRGGBB`, as the SVGs write colors.
pub fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// The `index.theme` of a theme: the name and description a cursor setting shows.
pub fn index_theme(variant: Variant) -> String {
    let (name, comment) = match variant {
        Variant::Dark => (cursor::DARK_NAME, cursor::DARK_COMMENT),
        Variant::Light => (cursor::LIGHT_NAME, cursor::LIGHT_COMMENT),
    };
    format!("[Icon Theme]\nName={name}\nComment={comment}\nInherits=default\n")
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use super::*;

    fn svgs() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/cursors")
    }

    fn read(shape: &Shape) -> (String, Drawing) {
        let path = svgs().join(format!("{}.svg", shape.name));
        let svg = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let drawing = Drawing::read(&svg).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        (svg, drawing)
    }

    #[test]
    fn every_name_is_given_once() {
        let mut seen = HashSet::new();
        for shape in SHAPES {
            for name in std::iter::once(&shape.name).chain(shape.aliases) {
                assert!(seen.insert(*name), "{name} given twice");
            }
        }
    }

    #[test]
    fn every_css_name_of_the_protocol_has_a_shape() {
        // The names of `wp_cursor_shape_v1`, versions 1 and 2.
        let css = "default context-menu help pointer progress wait cell crosshair text \
                   vertical-text alias copy move no-drop not-allowed grab grabbing e-resize \
                   n-resize ne-resize nw-resize s-resize se-resize sw-resize w-resize ew-resize \
                   ns-resize nesw-resize nwse-resize col-resize row-resize all-scroll zoom-in \
                   zoom-out dnd-ask all-resize";
        for name in css.split_whitespace() {
            assert!(shape(name).is_some(), "{name}");
        }
    }

    #[test]
    fn every_shape_has_a_drawing_with_its_hotspot_inside() {
        for shape in SHAPES {
            let (_, drawing) = read(shape);
            for size in SIZES {
                let (x, y) = drawing.hotspot_at(size);
                assert!(
                    x < canvas(size) && y < canvas(size),
                    "{} at {size}",
                    shape.name
                );
            }
        }
        let arrow = read(&SHAPES[0]).1;
        assert_eq!(arrow.hotspot, (5.0, 2.5));
        assert_eq!(arrow.hotspot_at(24), (7, 4));
        assert_eq!(arrow.hotspot_at(96), (28, 18));
        assert_eq!(
            read(shape("wait").unwrap_or(&SHAPES[0])).1.spin,
            Some((12.0, 12.0))
        );
    }

    #[test]
    fn the_drawings_show_the_dark_theme_as_the_generator_paints_it() {
        for shape in SHAPES {
            let (svg, drawing) = read(shape);
            // Without turning the spinning group, the dark theme is the file itself.
            let still = Drawing {
                spin: None,
                ..drawing
            };
            assert_eq!(
                Variant::Dark.paint(&svg, &still, 0.0),
                svg,
                "{}",
                shape.name
            );
        }
    }

    #[test]
    fn the_light_theme_changes_only_the_style_block() {
        let (svg, drawing) = read(&SHAPES[0]);
        let light = Variant::Light.paint(&svg, &drawing, 0.0);
        assert!(light.contains(".body { fill: #14233B }"));
        assert!(light.contains(".edge { stroke: #FFFFFF }"));
        let outside = |s: &str| s[s.find("</style>").unwrap_or(0)..].to_owned();
        assert_eq!(outside(&light), outside(&svg));
    }

    #[test]
    fn the_spinning_group_turns_around_its_center() {
        let svg = "<svg data-hotspot=\"12 12\"><style id=\"theme\"></style>\
                   <g id=\"spin\" data-center=\"20.5 19.5\"></g></svg>";
        let drawing = Drawing::read(svg).unwrap_or_else(|e| panic!("{e}"));
        let turned = Variant::Dark.paint(svg, &drawing, 90.0);
        assert!(turned.contains("id=\"spin\" transform=\"rotate(90 20.5 19.5)\""));
        assert!(Drawing::read("<svg><style id=\"theme\"></style></svg>").is_err());
        assert!(Drawing::read("<svg data-hotspot=\"1\"><style id=\"theme\">").is_err());
    }
}
