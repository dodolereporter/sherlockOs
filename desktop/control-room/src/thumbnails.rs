//! Thumbnails of the installed wallpapers, for the picker of the Appearance page: the smallest
//! variant of each, decoded and reduced off the interface thread.

use std::path::{Path, PathBuf};

use desktop_config::wallpaper;
use sherlock_ui::control_room::{THUMBNAIL_SCALE, WALLPAPER_TILE, WALLPAPER_TILE_MAX};
use sherlock_ui::wallpapers;

/// A thumbnail: its pixels, RGBA, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    pub id: &'static str,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The wallpapers of the collection that are installed in `dirs`, in the picker's order.
pub fn installed(dirs: &[PathBuf]) -> Vec<&'static wallpapers::Wallpaper> {
    wallpapers::ALL
        .iter()
        .filter(|w| wallpaper::find(w.id, dirs).is_some())
        .collect()
}

/// The thumbnail of each installed wallpaper. One that cannot be read is left out, and goes to
/// the log: the picker shows its name on a plain tile.
pub fn load(dirs: &[PathBuf]) -> Vec<Thumbnail> {
    installed(dirs)
        .into_iter()
        .filter_map(|w| {
            let folder = wallpaper::find(w.id, dirs)?;
            match read(w.id, &folder) {
                Ok(thumbnail) => Some(thumbnail),
                Err(error) => {
                    tracing::warn!(%error, folder = %folder.display(), "no wallpaper thumbnail");
                    None
                }
            }
        })
        .collect()
}

fn read(id: &'static str, folder: &Path) -> Result<Thumbnail, image::ImageError> {
    let smallest = wallpaper::variants(folder)?
        .into_iter()
        .next()
        .ok_or_else(|| std::io::Error::other("no variant"))?;
    let picture = image::open(&smallest.path)?.to_rgba8();
    let (width, height) = size();
    let small = image::imageops::thumbnail(&picture, width, height);
    Ok(Thumbnail {
        id,
        width,
        height,
        pixels: small.into_raw(),
    })
}

/// Pixels of a thumbnail: the widest tile, at `THUMBNAIL_SCALE`.
pub fn size() -> (u32, u32) {
    let width = WALLPAPER_TILE_MAX * THUMBNAIL_SCALE;
    (
        width as u32,
        (width * WALLPAPER_TILE.1 / WALLPAPER_TILE.0) as u32,
    )
}
