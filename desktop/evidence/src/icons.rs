//! Application icons for the « Ouvrir avec » submenu and the chooser: found in the hicolor
//! theme, the pixmaps folder or at an absolute path, and decoded off the interface thread.
//! Other icon themes (Breeze…) are not searched: most applications also install a hicolor
//! icon.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use iced::widget::{image, svg};
use mime_apps::Catalog;

/// Pixel size icons are decoded to: twice the 16 px they are shown at.
const DECODED: u32 = 32;
/// hicolor sizes to look in, the closest to what is shown first.
const SIZES: [&str; 8] = [
    "scalable", "32x32", "48x48", "24x24", "64x64", "16x16", "128x128", "256x256",
];

/// An icon ready to draw.
#[derive(Debug, Clone)]
pub enum AppIcon {
    Raster(image::Handle),
    Vector(svg::Handle),
}

/// The file of an icon name, if one can be found.
pub fn find(name: &str, data_dirs: &[PathBuf]) -> Option<PathBuf> {
    let direct = Path::new(name);
    if direct.is_absolute() {
        return direct.is_file().then(|| direct.to_path_buf());
    }
    for dir in data_dirs {
        for size in SIZES {
            for extension in ["svg", "png"] {
                let path = dir.join(format!("icons/hicolor/{size}/apps/{name}.{extension}"));
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        for extension in ["svg", "png"] {
            let path = dir.join(format!("pixmaps/{name}.{extension}"));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn load(path: &Path) -> Option<AppIcon> {
    if path.extension().is_some_and(|e| e == "svg") {
        // Read here, off the interface thread; the 16 px rendering is left to the renderer.
        return Some(AppIcon::Vector(svg::Handle::from_memory(
            std::fs::read(path).ok()?,
        )));
    }
    let picture = ::image::open(path)
        .ok()?
        .thumbnail(DECODED, DECODED)
        .to_rgba8();
    let (width, height) = picture.dimensions();
    Some(AppIcon::Raster(image::Handle::from_rgba(
        width,
        height,
        picture.into_raw(),
    )))
}

/// The icon of every application of the catalog that has one, by desktop id.
pub fn load_all(catalog: &Catalog) -> HashMap<String, AppIcon> {
    let data_dirs: Vec<PathBuf> = catalog.dirs.data().cloned().collect();
    catalog
        .applications
        .iter()
        .filter_map(|app| {
            let path = find(app.icon.as_deref()?, &data_dirs)?;
            Some((app.id.clone(), load(&path)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn icons_are_found_in_hicolor_pixmaps_or_by_path() {
        let tmp = TempDir::new("icons");
        let data = tmp.path().join("share");
        let svg_icon = data.join("icons/hicolor/scalable/apps/kate.svg");
        std::fs::create_dir_all(svg_icon.parent().expect("parent")).expect("folder");
        std::fs::write(&svg_icon, "<svg xmlns='http://www.w3.org/2000/svg'/>").expect("write");
        let png = data.join("pixmaps/code.png");
        std::fs::create_dir_all(png.parent().expect("parent")).expect("folder");
        ::image::RgbaImage::new(64, 64).save(&png).expect("png");
        let dirs = [data];
        assert_eq!(find("kate", &dirs), Some(svg_icon));
        assert_eq!(find("code", &dirs), Some(png.clone()));
        assert_eq!(find(&png.to_string_lossy(), &[]), Some(png.clone()));
        assert_eq!(find("absent", &dirs), None);
        assert!(matches!(load(&png), Some(AppIcon::Raster(_))));
    }
}
