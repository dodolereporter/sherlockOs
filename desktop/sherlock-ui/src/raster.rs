//! Application icons drawn smooth on the GPU (UI-05).
//!
//! The author saw the icons of the dock, the launcher and the Applications view with rough,
//! « pixelisés » edges, on the GPU at scale 1. iced 0.14's wgpu renderer draws an SVG through
//! three steps that each harm a full-color icon:
//!
//! - it rasterizes it with resvg into a premultiplied pixmap and uploads those pixels as they
//!   are, then blends them as straight alpha (`SrcAlpha`, `OneMinusSrcAlpha`): an edge pixel at
//!   half alpha is darkened twice, and every antialiased edge gets a dark fringe;
//! - it rasterizes at the drawn size rounded up (`ceil`), then samples that raster with its
//!   nearest-neighbour sampler: at a size that is not a whole number of pixels (the dock
//!   magnifying, a fractional scale) rows and columns are doubled or dropped;
//! - it rasterizes again at every new size, so a magnifying dock rasterizes every frame.
//!
//! (A single-color line icon escapes the first: its color replaces the premultiplied color.)
//!
//! `icon` draws the SVG as an image instead: rasterized here by the same resvg, the alpha taken
//! out of the colors (straight alpha, what the image pipeline blends), at twice the logical size
//! rounded up to a step of `STEP` pixels, so that it is never enlarged on a screen up to scale 2
//! and every size of a magnification between two steps shares one raster; drawn with the linear
//! sampler. Each raster is kept, by document and size: no frame rasterizes what an earlier one
//! did.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};

use iced::widget::image::{self, FilterMethod, Image};
use iced::widget::svg;

/// How much finer than its logical size an icon is rasterized: exact at scale 2, downsampled
/// by the linear sampler below.
pub const OVERSAMPLE: f32 = 2.0;
/// Raster sizes are whole multiples of this many pixels.
pub const STEP: u32 = 16;
/// The largest raster: 1.7 times the dock's largest icon, at scale 2, fits.
pub const MAX_SIDE: u32 = 512;
/// Rasters kept before the cache starts again from nothing.
const MAX_KEPT: usize = 1024;

/// The side of the raster of an icon drawn `side` logical pixels wide.
pub fn raster_side(side: f32) -> u32 {
    let wanted = (side.max(1.0) * OVERSAMPLE / STEP as f32).ceil() as u32 * STEP;
    wanted.clamp(STEP, MAX_SIDE)
}

/// The pixels of `document` rasterized `side` by `side`, its aspect kept and centred, in
/// straight-alpha RGBA. `None` when it cannot be read.
pub fn rasterize(document: &[u8], side: u32) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(document, &usvg::Options::default()).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let size = tree.size();
    let fit = (side as f32 / size.width()).min(side as f32 / size.height());
    let (dx, dy) = (
        (side as f32 - size.width() * fit) / 2.0,
        (side as f32 - size.height() * fit) / 2.0,
    );
    let transform = tiny_skia::Transform::from_scale(fit, fit).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let mut pixels: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    bleed(&mut pixels, side as usize);
    Some(pixels)
}

/// Gives every transparent pixel the color of its visible neighbours, its alpha left at 0. The
/// linear sampler mixes the colors of neighbouring pixels before their alpha is applied: a
/// transparent pixel left black would darken the edge it borders.
fn bleed(pixels: &mut [u8], side: usize) {
    let source = pixels.to_vec();
    for y in 0..side {
        for x in 0..side {
            let at = (y * side + x) * 4;
            if source[at + 3] != 0 {
                continue;
            }
            let mut sum = [0u32; 3];
            let mut weight = 0u32;
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= side as isize || ny >= side as isize {
                    continue;
                }
                let near = (ny as usize * side + nx as usize) * 4;
                let alpha = u32::from(source[near + 3]);
                for (total, channel) in sum.iter_mut().zip(&source[near..near + 3]) {
                    *total += u32::from(*channel) * alpha;
                }
                weight += alpha;
            }
            for (channel, total) in pixels[at..at + 3].iter_mut().zip(sum) {
                if let Some(mean) = total.checked_div(weight) {
                    *channel = u8::try_from(mean).unwrap_or(u8::MAX);
                }
            }
        }
    }
}

/// The bytes of an SVG handle: in memory, or read from its file.
fn document(handle: &svg::Handle) -> Option<Vec<u8>> {
    match handle.data() {
        iced::advanced::svg::Data::Bytes(bytes) => Some(bytes.to_vec()),
        iced::advanced::svg::Data::Path(path) => std::fs::read(path).ok(),
    }
}

type Cache = Mutex<HashMap<(u64, u32, Option<[u8; 3]>), image::Handle>>;

/// The image of `handle` at the raster side of `side`, rasterized once.
pub fn raster(handle: &svg::Handle, side: f32) -> image::Handle {
    raster_in(handle, side, None)
}

/// The image of a single-color `handle` (a symbolic icon of a theme) in the color `tint`, its
/// own color left out and its alpha kept: the icon follows the theme, light or dark (lot D-45).
pub fn tinted(handle: &svg::Handle, side: f32, tint: crate::color::Color) -> image::Handle {
    raster_in(handle, side, Some([tint.r, tint.g, tint.b]))
}

fn raster_in(handle: &svg::Handle, side: f32, tint: Option<[u8; 3]>) -> image::Handle {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let px = raster_side(side);
    let key = (handle.id(), px, tint);
    let cache = CACHE.get_or_init(Cache::default);
    if let Some(found) = cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return found.clone();
    }
    let mut pixels = document(handle).and_then(|bytes| rasterize(&bytes, px));
    if let (Some(pixels), Some(tint)) = (pixels.as_mut(), tint) {
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel[..3].copy_from_slice(&tint);
        }
    }
    let drawn = match pixels {
        Some(pixels) => image::Handle::from_rgba(px, px, pixels),
        // A document resvg cannot read draws nothing, as iced's own SVG would.
        None => image::Handle::from_rgba(1, 1, vec![0; 4]),
    };
    let mut kept = cache.lock().unwrap_or_else(PoisonError::into_inner);
    if kept.len() >= MAX_KEPT {
        kept.clear();
    }
    kept.insert(key, drawn.clone());
    drawn
}

/// An application icon, `side` logical pixels wide, smooth at any size and scale: see the
/// module. Its opacity is set like any image's (`.opacity(..)`).
pub fn icon(handle: &svg::Handle, side: f32) -> Image<image::Handle> {
    iced::widget::image(raster(handle, side))
        .width(side)
        .height(side)
        .filter_method(FilterMethod::Linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\
        <rect x=\"2\" y=\"2\" width=\"20\" height=\"20\" rx=\"5\" fill=\"#F4EBE3\"/></svg>";

    #[test]
    fn rasters_are_twice_the_size_in_steps() {
        assert_eq!(raster_side(52.0), 112);
        // A magnifying dock: every size between two steps shares one raster.
        assert_eq!(raster_side(52.5), raster_side(55.0));
        assert_eq!(raster_side(88.4), 192);
        assert_eq!(raster_side(0.0), STEP);
        assert_eq!(raster_side(10_000.0), MAX_SIDE);
    }

    #[test]
    fn edges_are_straight_alpha_without_a_dark_fringe() {
        let Some(pixels) = rasterize(TILE.as_bytes(), 64) else {
            panic!("the tile rasterizes");
        };
        // Every pixel with some alpha keeps the cream of the tile, however transparent: the
        // alpha is not multiplied into the color.
        let mut edges = 0;
        for pixel in pixels.chunks(4) {
            let alpha = pixel[3];
            if alpha > 8 && alpha < 247 {
                edges += 1;
                assert!(pixel[0] >= 0xF0 && pixel[1] >= 0xE4, "{pixel:?}");
            }
        }
        assert!(edges > 20, "the edges are antialiased: {edges}");
    }

    #[test]
    fn a_raster_is_made_once_per_size() {
        let handle = svg::Handle::from_memory(TILE.as_bytes());
        let first = raster(&handle, 52.0);
        assert_eq!(first.id(), raster(&handle, 52.0).id());
        assert_eq!(first.id(), raster(&handle, 53.0).id(), "the same step");
        assert_ne!(first.id(), raster(&handle, 90.0).id());
        assert!(rasterize(b"not an svg", 32).is_none());
    }

    #[test]
    fn a_symbolic_icon_takes_the_tint_and_keeps_its_alpha() {
        let handle = svg::Handle::from_memory(TILE.as_bytes());
        let tint = crate::color::Color::rgb(0x14, 0x23, 0x3B);
        let plain = raster(&handle, 20.0);
        let tinted = tinted(&handle, 20.0, tint);
        assert_ne!(plain.id(), tinted.id());
        assert_eq!(tinted.id(), super::tinted(&handle, 20.0, tint).id());
        let Some(pixels) = rasterize(TILE.as_bytes(), raster_side(20.0)) else {
            panic!("the tile rasterizes");
        };
        let iced::advanced::image::Handle::Rgba { pixels: drawn, .. } = tinted else {
            panic!("pixels");
        };
        for (shown, source) in drawn.chunks(4).zip(pixels.chunks(4)) {
            assert_eq!(shown[3], source[3]);
            assert_eq!(&shown[..3], &[0x14, 0x23, 0x3B]);
        }
    }
}
