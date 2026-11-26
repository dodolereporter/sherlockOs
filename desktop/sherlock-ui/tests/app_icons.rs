//! The application icons render without error at every size the desktop draws them, with their
//! tile where it should be: through resvg, the SVG renderer iced uses.

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{Options, Tree};
use sherlock_ui::app_icon::AppIcon;
use sherlock_ui::color::{self, Color};

/// Sizes in device pixels: 16 and 32 of the theme, the launcher tile at scale 1 and 2, the dock
/// at rest and magnified, at scale 1 and 2.
const SIZES: [u32; 8] = [16, 30, 32, 52, 60, 88, 104, 176];

fn render(svg: &str, size: u32) -> Pixmap {
    let tree = Tree::from_str(svg, &Options::default()).expect("SVG parsed");
    let mut pixmap = Pixmap::new(size, size).expect("pixmap");
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap
}

/// Whether the pixel at (`x`, `y`) is opaque and `color`, each channel within `tolerance`.
fn is_color(pixmap: &Pixmap, x: u32, y: u32, color: Color, tolerance: u8) -> bool {
    let Some(pixel) = pixmap.pixel(x, y) else {
        return false;
    };
    let close = |a: u8, b: u8| a.abs_diff(b) <= tolerance;
    pixel.alpha() == 255
        && close(pixel.red(), color.r)
        && close(pixel.green(), color.g)
        && close(pixel.blue(), color.b)
}

#[test]
fn every_icon_renders_at_every_size() {
    for icon in AppIcon::ALL {
        for size in SIZES {
            let pixmap = render(icon.at(size as f32, 1.0), size);
            // Near the top middle every drawing leaves its tile bare; the hairline border along
            // the edge lightens it a little.
            let (x, y) = (size / 2, (size / 16).max(1));
            assert!(is_color(&pixmap, x, y, icon.tile(), 28), "{icon:?} at {size}");
            // The corners are rounded off: transparent.
            assert_eq!(pixmap.pixel(0, 0).map(|p| p.alpha()), Some(0), "{icon:?}");
            // Something is drawn on the tile.
            let drawn = pixmap
                .pixels()
                .iter()
                .filter(|p| p.alpha() == 255)
                .any(|p| {
                    let tile = icon.tile();
                    p.red().abs_diff(tile.r) > 60 || p.green().abs_diff(tile.g) > 60
                });
            assert!(drawn, "{icon:?} at {size}: an empty tile");
        }
    }
}

#[test]
fn the_optical_versions_sit_on_whole_pixels() {
    // At 16 px, every pixel of an optical version is the tile or a color of the glyph, flat:
    // nothing smoothed, away from the rounded corners and the hairline border.
    let glyph = [color::CREAM, color::TILE_KNOB];
    for icon in AppIcon::ALL {
        let Some(optical) = icon.optical() else {
            continue;
        };
        let pixmap = render(optical, 16);
        for y in 2..14 {
            for x in 2..14 {
                let flat = std::iter::once(icon.tile())
                    .chain(glyph)
                    .any(|c| is_color(&pixmap, x, y, c, 2));
                assert!(flat, "{icon:?}: pixel ({x}, {y}) is smoothed");
            }
        }
    }
}
