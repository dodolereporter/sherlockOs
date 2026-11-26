//! The cursor theme of the desktop: its shapes, their hotspots and their names, and the Xcursor
//! format the themes are written in.
//!
//! The drawings are the SVGs of `assets/cursors/`, on a grid of 24 units: one unit is one pixel
//! at the size 24. The generator (`cursor-theme`, feature `render`) renders them at every size
//! of `SIZES` and writes two themes, `nom-os` (night blue, cream outline) and `nom-os-inverse`
//! (cream, night blue outline). See `docs/cursor.md`.

pub mod xcursor;

use sherlock_ui::color::{self, Color};

/// Theme the compositor uses by default: night blue filled, cream outline.
pub const THEME: &str = "nom-os";
/// The optional variant: cream filled, night blue outline.
pub const INVERSE_THEME: &str = "nom-os-inverse";

/// Side of the drawing grid of the SVGs.
pub const GRID: u32 = 24;
/// Nominal sizes written in each file: scales 1 to 4 of the size 24.
pub const SIZES: [u32; 5] = [24, 32, 48, 64, 96];

/// A part of the drawing that turns, for the waiting cursors: the group `id="spin"` of the SVG.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spin {
    /// Center of the rotation, in grid units.
    pub center: (f32, f32),
    /// Images in a turn.
    pub frames: u32,
    /// Time each image is shown, in milliseconds.
    pub delay_ms: u32,
}

/// A cursor shape: one drawing, its hotspot and every name it answers to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    /// Name of the file and of the SVG: the CSS name, as `wp_cursor_shape_v1` gives it.
    pub name: &'static str,
    /// The point of the cursor, in grid units. Multiples of 3, so that it falls on a whole
    /// pixel at each size of `SIZES`.
    pub hotspot: (u32, u32),
    /// Other names, CSS and legacy X11: links to the file in the theme.
    pub aliases: &'static [&'static str],
    /// Turns, for the waiting cursors.
    pub spin: Option<Spin>,
}

impl Shape {
    /// The hotspot in pixels for the nominal `size`.
    pub fn hotspot_at(&self, size: u32) -> (u32, u32) {
        let scale = |v: u32| (v * size + GRID / 2) / GRID;
        (scale(self.hotspot.0), scale(self.hotspot.1))
    }
}

/// One turn per second, 30 images: smooth at 60 Hz, two vertical blanks per image.
const SPIN_FRAMES: u32 = 30;
const SPIN_DELAY_MS: u32 = 33;

const fn arrow(name: &'static str, aliases: &'static [&'static str]) -> Shape {
    Shape {
        name,
        hotspot: (3, 3),
        aliases,
        spin: None,
    }
}

const fn centered(name: &'static str, aliases: &'static [&'static str]) -> Shape {
    Shape {
        name,
        hotspot: (12, 12),
        aliases,
        spin: None,
    }
}

/// Every shape of the theme. The CSS names of `wp_cursor_shape_v1` that share a drawing are
/// aliases: a window edge shows the double arrow of its axis, as the compositor's resizing.
pub const SHAPES: &[Shape] = &[
    arrow(
        "default",
        &["left_ptr", "arrow", "top_left_arrow", "left_arrow"],
    ),
    arrow("context-menu", &["dnd-ask"]),
    arrow("help", &["question_arrow", "whats_this", "left_ptr_help"]),
    Shape {
        name: "pointer",
        hotspot: (9, 3),
        aliases: &["hand2", "hand1", "hand", "pointing_hand"],
        spin: None,
    },
    Shape {
        name: "progress",
        hotspot: (3, 3),
        aliases: &["left_ptr_watch", "half-busy"],
        spin: Some(Spin {
            center: (17.0, 17.0),
            frames: SPIN_FRAMES,
            delay_ms: SPIN_DELAY_MS,
        }),
    },
    Shape {
        name: "wait",
        hotspot: (12, 12),
        aliases: &["watch"],
        spin: Some(Spin {
            center: (12.0, 12.0),
            frames: SPIN_FRAMES,
            delay_ms: SPIN_DELAY_MS,
        }),
    },
    centered("cell", &["plus"]),
    centered("crosshair", &["cross", "tcross"]),
    centered("text", &["xterm", "ibeam"]),
    centered("vertical-text", &[]),
    arrow("alias", &["link", "dnd-link"]),
    arrow("copy", &["dnd-copy"]),
    centered("move", &["all-resize", "fleur", "dnd-move"]),
    arrow(
        "not-allowed",
        &[
            "no-drop",
            "crossed_circle",
            "forbidden",
            "circle",
            "dnd-no-drop",
        ],
    ),
    centered("grab", &["openhand"]),
    centered("grabbing", &["closedhand", "dnd-none"]),
    centered(
        "ew-resize",
        &[
            "e-resize",
            "w-resize",
            "right_side",
            "left_side",
            "h_double_arrow",
            "sb_h_double_arrow",
            "size_hor",
        ],
    ),
    centered(
        "ns-resize",
        &[
            "n-resize",
            "s-resize",
            "top_side",
            "bottom_side",
            "v_double_arrow",
            "sb_v_double_arrow",
            "size_ver",
        ],
    ),
    centered(
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
    centered(
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
    centered("col-resize", &["split_h"]),
    centered("row-resize", &["split_v"]),
    centered("all-scroll", &["size_all"]),
    Shape {
        name: "zoom-in",
        hotspot: (9, 9),
        aliases: &[],
        spin: None,
    },
    Shape {
        name: "zoom-out",
        hotspot: (9, 9),
        aliases: &[],
        spin: None,
    },
];

/// The shape that answers to `name`, by its own name or one of its aliases.
pub fn shape(name: &str) -> Option<&'static Shape> {
    SHAPES
        .iter()
        .find(|s| s.name == name || s.aliases.contains(&name))
}

/// The two themes written by the generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Night blue filled, cream outline: the default.
    Night,
    /// Cream filled, night blue outline.
    Inverse,
}

impl Variant {
    pub const ALL: [Self; 2] = [Self::Night, Self::Inverse];

    /// Name of the theme folder.
    pub fn theme(self) -> &'static str {
        match self {
            Self::Night => THEME,
            Self::Inverse => INVERSE_THEME,
        }
    }

    /// The SVG of a drawing in this variant. The drawings are written in the night variant:
    /// the inverse one swaps the fill and the outline.
    pub fn paint(self, svg: &str) -> String {
        match self {
            Self::Night => svg.to_owned(),
            Self::Inverse => {
                let (fill, outline) = (hex(FILL), hex(OUTLINE));
                svg.replace(&fill, "\u{0}")
                    .replace(&outline, &fill)
                    .replace('\u{0}', &outline)
            }
        }
    }
}

/// Fill of the drawings: the night blue of the text on cream.
pub const FILL: Color = color::TEXT_ON_CREAM;
/// Outline of the drawings.
pub const OUTLINE: Color = color::CREAM;
/// Colors a drawing may use: fill, outline, the shadow and the green of the waiting ring.
pub const DRAWING_COLORS: [Color; 4] = [FILL, OUTLINE, color::SHADOW_BASE, color::ACCENT];

/// `#RRGGBB`, as the SVGs write colors.
pub fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// The `index.theme` of a theme.
pub fn index_theme(variant: Variant) -> String {
    let comment = match variant {
        Variant::Night => "Night blue cursors of NOM_OS",
        Variant::Inverse => "Cream cursors of NOM_OS",
    };
    format!(
        "[Icon Theme]\nName={}\nComment={comment}\nInherits=default\n",
        variant.theme()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

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
    fn hotspots_fall_on_whole_pixels_inside_the_image() {
        for shape in SHAPES {
            let (x, y) = shape.hotspot;
            assert!(x < GRID && y < GRID, "{}", shape.name);
            for size in SIZES {
                assert_eq!((x * size) % GRID, 0, "{} at {size}", shape.name);
                assert_eq!((y * size) % GRID, 0, "{} at {size}", shape.name);
                let (hx, hy) = shape.hotspot_at(size);
                assert!(hx < size && hy < size);
            }
        }
        assert_eq!(SHAPES[0].hotspot_at(96), (12, 12));
    }

    #[test]
    fn every_css_name_of_the_protocol_has_a_shape() {
        // The names of `wp_cursor_shape_v1`, versions 1 and 2.
        let css = [
            "default",
            "context-menu",
            "help",
            "pointer",
            "progress",
            "wait",
            "cell",
            "crosshair",
            "text",
            "vertical-text",
            "alias",
            "copy",
            "move",
            "no-drop",
            "not-allowed",
            "grab",
            "grabbing",
            "e-resize",
            "n-resize",
            "ne-resize",
            "nw-resize",
            "s-resize",
            "se-resize",
            "sw-resize",
            "w-resize",
            "ew-resize",
            "ns-resize",
            "nesw-resize",
            "nwse-resize",
            "col-resize",
            "row-resize",
            "all-scroll",
            "zoom-in",
            "zoom-out",
            "dnd-ask",
            "all-resize",
        ];
        for name in css {
            assert!(shape(name).is_some(), "{name}");
        }
    }

    #[test]
    fn the_inverse_variant_swaps_fill_and_outline() {
        let svg = format!("<path fill=\"{}\" stroke=\"{}\"/>", hex(FILL), hex(OUTLINE));
        let inverse = Variant::Inverse.paint(&svg);
        assert_eq!(
            inverse,
            format!("<path fill=\"{}\" stroke=\"{}\"/>", hex(OUTLINE), hex(FILL))
        );
        assert_eq!(Variant::Night.paint(&svg), svg);
    }
}
