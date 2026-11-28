//! Application icons drawn once at their size on the screen (lot P-02). Drawn by iced's SVG
//! widget, an icon was rasterized again at every size it took: at every frame of the opening,
//! whose scale moves, and each time a page came back on screen, since iced forgets what a frame
//! did not draw. Here each icon becomes pixels once, on a thread of the pool, at the size of the
//! grid in the screen's pixels; the GPU then only scales that picture.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::image;

use crate::catalog::Catalog;

/// Threads that draw the icons together.
const THREADS: usize = 4;

/// Icons drawn at one size, by file.
#[derive(Debug, Clone, Default)]
pub struct Rasters {
    /// Their side, in pixels of the screen.
    pub side: u32,
    pub icons: HashMap<PathBuf, image::Handle>,
}

impl Rasters {
    /// The icon in `path`, when it is drawn at `side`.
    pub fn get(&self, path: &Path, side: u32) -> Option<&image::Handle> {
        (self.side == side).then(|| self.icons.get(path)).flatten()
    }

    /// Takes the icons of `other`, when they are of the same size; otherwise they replace these.
    pub fn merge(&mut self, other: Rasters) {
        if other.side != self.side {
            *self = other;
        } else {
            self.icons.extend(other.icons);
        }
    }
}

/// The side, in pixels of the screen, of an icon `logical` wide at `scale`.
pub fn side(logical: f32, scale: f32) -> u32 {
    (logical * scale).ceil().max(1.0) as u32
}

/// Draws at `side` the icons of `catalog` that `done` does not hold yet, `THREADS` at a time.
pub fn draw(catalog: &Catalog, side: u32, done: &Rasters) -> Rasters {
    let mut paths: Vec<PathBuf> = catalog
        .tiles()
        .iter()
        .filter_map(|tile| catalog.icon(tile))
        .filter(|path| done.get(path, side).is_none())
        .collect();
    paths.sort();
    paths.dedup();
    let chunk = paths.len().div_ceil(THREADS).max(1);
    let icons = std::thread::scope(|scope| {
        let workers: Vec<_> = paths
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|path| Some((path.clone(), rasterize(path, side)?)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .filter_map(|worker| worker.join().ok())
            .flatten()
            .collect()
    });
    Rasters { side, icons }
}

/// What `draw` gives, for a message.
pub type Shared = Arc<Rasters>;

/// The icon in `path`, an SVG or PNG file, drawn in a square of `side` pixels, kept in its
/// proportions and centred: by `sherlock_ui::raster`, with its bounds (file sizes, no file read
/// from inside an SVG).
pub fn rasterize(path: &Path, side: u32) -> Option<image::Handle> {
    let pixels = sherlock_ui::raster::rasterize_file(path, side);
    if pixels.is_none() {
        tracing::warn!(path = %path.display(), "the icon could not be drawn");
    }
    Some(image::Handle::from_rgba(side, side, pixels?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svg_icon_is_drawn_at_its_side_in_screen_pixels() {
        let dir = std::env::temp_dir().join(format!("apps-icons-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("folder");
        let path = dir.join("square.svg");
        std::fs::write(
            &path,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"20\">\
             <rect width=\"10\" height=\"20\" fill=\"#ff0000\"/></svg>",
        )
        .expect("written");
        assert_eq!(side(96.0, 1.0), 96);
        assert_eq!(side(95.2, 1.5), 143);
        let handle = rasterize(&path, 40).expect("drawn");
        let image::Handle::Rgba {
            width,
            height,
            pixels,
            ..
        } = handle
        else {
            panic!("pixels");
        };
        assert_eq!((width, height), (40, 40));
        // Kept in its proportions, centred: 20 wide in the middle, red and opaque there.
        let at = |x: usize, y: usize| &pixels[(y * 40 + x) * 4..(y * 40 + x) * 4 + 4];
        assert_eq!(at(20, 20), [255, 0, 0, 255]);
        assert_eq!(at(2, 20)[3], 0, "transparent beside");
        assert!(rasterize(&dir.join("missing.svg"), 40).is_none());
        let mut rasters = Rasters {
            side: 40,
            ..Rasters::default()
        };
        rasters.merge(Rasters {
            side: 40,
            icons: HashMap::from([(path.clone(), rasterize(&path, 40).expect("drawn"))]),
        });
        assert!(rasters.get(&path, 40).is_some());
        assert!(rasters.get(&path, 80).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
