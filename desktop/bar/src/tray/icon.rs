//! The icon of an item, found once when the item is read, off the interface thread: a name in
//! the folder the application gives (`IconThemePath`) or in the icon theme, else its pixels.
//! Drawn through `sherlock_ui::raster`, sharp at any scale; a symbolic icon takes the color of
//! the text of the theme, light or dark.

use std::path::{Path, PathBuf};

use iced::widget::{image, svg};
use icon_theme::Lookup;

use super::{Source, best};

/// Side the icons are looked up for, in logical pixels: the popup's, the larger.
const SIDE: u32 = sherlock_ui::tray::POPUP_ICON as u32;
/// Scale they are looked up and rasterized for: sharp up to a scale 2 screen.
const SCALE: u32 = 2;

/// An icon ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub enum Icon {
    /// A drawing: an SVG of the theme, or a PNG inside an SVG, rasterized by
    /// `sherlock_ui::raster`. A symbolic one is drawn in the color of the text.
    Drawing { handle: svg::Handle, symbolic: bool },
    /// The pixels the application sent.
    Pixels(image::Handle),
}

/// Where icons are looked for: the user's theme, what it inherits, `hicolor`, `pixmaps`.
#[derive(Debug)]
pub struct Finder {
    lookup: Lookup,
}

impl Finder {
    /// From the environment: the XDG data folders and the user's icon theme.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
        let dirs = mime_apps::Dirs::from_env(&home);
        let data: Vec<PathBuf> = dirs.data().cloned().collect();
        let theme = icon_theme::user_theme(&dirs.config_home);
        Self {
            lookup: Lookup::new(&data, Some(&home), theme.as_deref()),
        }
    }

    /// Looks in `data` only, with no theme of the user's: for the tests.
    pub fn in_folders(data: &[PathBuf]) -> Self {
        Self {
            lookup: Lookup::new(data, None, None),
        }
    }

    /// The icon of `source`: its name in `theme_path`, then in the theme, then its pixels.
    pub fn find(&self, source: &Source, theme_path: Option<&str>) -> Option<Icon> {
        let named = source.name.as_deref().and_then(|name| {
            let file = theme_path
                .and_then(|folder| in_folder(Path::new(folder), name))
                .or_else(|| self.lookup.find(name, SIDE, SCALE))?;
            drawing(&file, name)
        });
        named.or_else(|| {
            let side = (SIDE * SCALE) as i32;
            let pixmap = best(&source.pixmaps, side)?;
            let rgba = pixmap.rgba()?;
            let (width, height) = (pixmap.width as u32, pixmap.height as u32);
            Some(Icon::Pixels(image::Handle::from_rgba(width, height, rgba)))
        })
    }
}

/// `name` straight in `folder`, as Electron and libappindicator put their icons, or in a theme
/// of `folder` when it is an `icons` folder.
fn in_folder(folder: &Path, name: &str) -> Option<PathBuf> {
    let direct = ["svg", "png"]
        .iter()
        .map(|ext| folder.join(format!("{name}.{ext}")))
        .find(|file| file.is_file());
    direct.or_else(|| {
        let data = folder
            .file_name()
            .is_some_and(|n| n == "icons")
            .then(|| folder.parent())
            .flatten()?;
        Lookup::new(&[data.to_path_buf()], None, None).find(name, SIDE, SCALE)
    })
}

/// The drawing of the file `file`, found for the name `name`.
fn drawing(file: &Path, name: &str) -> Option<Icon> {
    let symbolic = name.ends_with("-symbolic")
        || file
            .file_stem()
            .is_some_and(|stem| stem.to_string_lossy().ends_with("-symbolic"));
    let handle = match file.extension().and_then(|e| e.to_str()) {
        Some("svg") => svg::Handle::from_memory(std::fs::read(file).ok()?),
        Some("png") => wrapped(file),
        _ => return None,
    };
    Some(Icon::Drawing { handle, symbolic })
}

/// A PNG inside an SVG, which resvg reads: the same rasterizer as the rest, as the dock does.
fn wrapped(file: &Path) -> svg::Handle {
    let href = file
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"{href}\"/></svg>"
    );
    svg::Handle::from_memory(document.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray::Pixmap;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bar-tray-icon-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 16 16\">\
        <rect width=\"16\" height=\"16\" fill=\"#bebebe\"/></svg>";

    #[test]
    fn the_folder_of_the_application_comes_first_then_the_theme_then_the_pixels() {
        let own = folder("own");
        std::fs::write(own.join("discord-tray.svg"), SVG).unwrap();
        let data = folder("data");
        let theme = data.join("icons/hicolor");
        std::fs::create_dir_all(theme.join("scalable/status")).unwrap();
        std::fs::write(
            theme.join("index.theme"),
            "[Icon Theme]\nName=Hicolor\nDirectories=scalable/status\n\n\
             [scalable/status]\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n",
        )
        .unwrap();
        std::fs::write(theme.join("scalable/status/network-vpn-symbolic.svg"), SVG).unwrap();
        let finder = Finder::in_folders(std::slice::from_ref(&data));

        let named = |name: &str| Source {
            name: Some(name.to_owned()),
            pixmaps: Vec::new(),
        };
        let own_path = own.to_string_lossy().into_owned();
        assert!(matches!(
            finder.find(&named("discord-tray"), Some(&own_path)),
            Some(Icon::Drawing {
                symbolic: false,
                ..
            })
        ));
        assert!(matches!(
            finder.find(&named("network-vpn-symbolic"), None),
            Some(Icon::Drawing { symbolic: true, .. })
        ));
        // A name found nowhere: its pixels.
        let pixels = Source {
            pixmaps: vec![Pixmap {
                width: 1,
                height: 1,
                argb: vec![0xFF, 1, 2, 3],
            }],
            ..named("missing")
        };
        assert!(matches!(
            finder.find(&pixels, Some(&own_path)),
            Some(Icon::Pixels(_))
        ));
        assert_eq!(finder.find(&named("missing"), None), None);
        let _ = std::fs::remove_dir_all(&own);
        let _ = std::fs::remove_dir_all(&data);
    }
}
