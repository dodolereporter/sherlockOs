//! The picture behind the lock screen (lot D-25b), in this order:
//!
//! 1. `background` of `lock.conf`, shown as is, in both themes;
//! 2. the pictures of the lock, `lock/lock-dark.jpg` and `lock/lock-light.jpg` in the data
//!    folders of the desktop (`just install-session` puts them in
//!    `/usr/local/share/sherlockos/lock/`), one per theme;
//! 3. else the wallpaper of the desktop in the active theme, blurred and darkened (computed once,
//!    on a copy reduced to `BACKGROUND_WIDTH`).
//!
//! Every picture is decoded once, on a thread of its own, then reduced to the size of the
//! largest screen (cover: centered, neither stretched nor banded), never kept at 4K for a small
//! screen. Until it is ready, or if nothing can be read, the plain background of the system
//! screens shows.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use desktop_config::wallpaper::{self, Choice};
use image::{DynamicImage, GenericImageView as _, imageops};
use sherlock_ui::color;
use sherlock_ui::lock::screen::{BACKGROUND_WIDTH, BLUR_SIGMA, DIM};
use sherlock_ui::wallpapers::ThemeVariant;

/// The folder of the lock pictures in a data folder of the desktop, and their names.
pub const FOLDER: &str = "lock";
pub const DARK: &str = "lock-dark.jpg";
pub const LIGHT: &str = "lock-light.jpg";

/// A picture ready to show, in RGBA.
#[derive(Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Picture({}×{})", self.width, self.height)
    }
}

/// The picture of each theme (the same one twice when there is a single picture).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pictures {
    pub dark: Picture,
    pub light: Picture,
}

/// Where the background comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// One picture for both themes, shown as is.
    Single(PathBuf),
    /// A picture per theme.
    Themed { dark: PathBuf, light: PathBuf },
    /// The wallpaper, blurred and darkened.
    Blurred(PathBuf),
}

/// The source of the background. `var` reads the environment.
pub fn source(var: impl Fn(&str) -> Option<OsString>) -> Option<Source> {
    let lock = desktop_config::lock::config_file(&var)
        .map(|file| desktop_config::lock::Config::read(&file))
        .and_then(|config| {
            config
                .inspect_err(|err| tracing::warn!("lock.conf: {err}"))
                .ok()
        });
    if let Some(path) = lock.and_then(|config| config.background) {
        return Some(Source::Single(path));
    }
    let themed = desktop_config::data_dirs(&var).into_iter().find_map(|dir| {
        let (dark, light) = (dir.join(FOLDER).join(DARK), dir.join(FOLDER).join(LIGHT));
        (dark.is_file() && light.is_file()).then_some(Source::Themed { dark, light })
    });
    if themed.is_some() {
        return themed;
    }
    let config = wallpaper::config_file(&var)
        .and_then(|file| wallpaper::Config::read(&file).ok())
        .unwrap_or_default();
    let variant = match color::active() {
        color::Theme::Light => ThemeVariant::Light,
        color::Theme::Dark => ThemeVariant::Dark,
    };
    let path = match config.choice(variant) {
        Choice::Path(path) => smallest(&path),
        Choice::Installed(id) => smallest(&wallpaper::find(&id, &wallpaper::dirs(&var))?),
    };
    path.map(Source::Blurred)
}

/// A file as is, or the smallest variant of a folder of variants.
fn smallest(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() {
        return Some(path.to_owned());
    }
    let variants = wallpaper::variants(path).ok()?;
    variants.into_iter().next().map(|variant| variant.path)
}

/// The size `image` takes to cover `screen` without being stretched: reduced so that it still
/// covers it, never enlarged (the renderer does that).
pub fn cover(image: (u32, u32), screen: (u32, u32)) -> (u32, u32) {
    let (w, h) = (u64::from(image.0.max(1)), u64::from(image.1.max(1)));
    let (sw, sh) = (u64::from(screen.0.max(1)), u64::from(screen.1.max(1)));
    // The side whose ratio is larger fits exactly; the other one overflows, rounded up.
    let (cw, ch) = if sw * h >= sh * w {
        (sw, (h * sw).div_ceil(w))
    } else {
        ((w * sh).div_ceil(h), sh)
    };
    if cw >= w || ch >= h {
        return image;
    }
    (cw as u32, ch as u32)
}

fn rgba(picture: DynamicImage) -> Picture {
    let picture = picture.to_rgba8();
    Picture {
        width: picture.width(),
        height: picture.height(),
        rgba: picture.into_raw(),
    }
}

/// `path` reduced to cover a screen of `screen` pixels.
pub fn fitted(path: &Path, screen: (u32, u32)) -> Result<Picture, image::ImageError> {
    let picture = image::open(path)?;
    let (w, h) = cover(picture.dimensions(), screen);
    if (w, h) == picture.dimensions() {
        return Ok(rgba(picture));
    }
    Ok(rgba(picture.resize_exact(w, h, imageops::Triangle)))
}

/// `path` reduced, blurred and darkened.
pub fn blurred(path: &Path) -> Result<Picture, image::ImageError> {
    let picture = image::open(path)?;
    let (w, h) = picture.dimensions();
    let width = BACKGROUND_WIDTH.min(w.max(1));
    let height = ((u64::from(h) * u64::from(width)) / u64::from(w.max(1))).max(1) as u32;
    let small = imageops::resize(&picture.to_rgba8(), width, height, imageops::Triangle);
    let mut soft = imageops::blur(&small, BLUR_SIGMA);
    for pixel in soft.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            *channel = (f32::from(*channel) * DIM).round() as u8;
        }
    }
    Ok(Picture {
        width,
        height,
        rgba: soft.into_raw(),
    })
}

/// The pictures of `source` for a screen of `screen` pixels.
pub fn pictures(source: &Source, screen: (u32, u32)) -> Result<Pictures, image::ImageError> {
    Ok(match source {
        Source::Single(path) => {
            let picture = fitted(path, screen)?;
            Pictures {
                dark: picture.clone(),
                light: picture,
            }
        }
        Source::Themed { dark, light } => Pictures {
            dark: fitted(dark, screen)?,
            light: fitted(light, screen)?,
        },
        Source::Blurred(path) => {
            let picture = blurred(path)?;
            Pictures {
                dark: picture.clone(),
                light: picture,
            }
        }
    })
}

/// `source` then `pictures`, logged: what the lock screen shows behind it.
pub fn load(var: impl Fn(&str) -> Option<OsString>, screen: (u32, u32)) -> Option<Pictures> {
    let source = source(var)?;
    pictures(&source, screen)
        .inspect_err(|err| tracing::warn!(?source, "lock background: {err}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    #[test]
    fn a_picture_covers_the_screen_without_being_stretched() {
        // The 4K picture for a small screen: reduced, still covering it.
        assert_eq!(cover((3840, 2160), (1366, 768)), (1366, 769));
        assert_eq!(cover((3840, 2160), (1920, 1080)), (1920, 1080));
        // Another shape: the larger ratio wins, the other side overflows (cropped, centered).
        assert_eq!(cover((3840, 2160), (1200, 1200)), (2134, 1200));
        // Never enlarged.
        assert_eq!(cover((3840, 2160), (3840, 2400)), (3840, 2160));
        let picture = fitted(&assets().join(FOLDER).join(DARK), (1366, 768)).unwrap();
        assert_eq!((picture.width, picture.height), (1366, 769));
        assert_eq!(picture.rgba.len(), 1366 * 769 * 4);
    }

    #[test]
    fn the_wallpaper_is_reduced_blurred_and_darkened() {
        let path = assets().join("wallpapers/first-clue.jpg");
        let picture = blurred(&path).unwrap();
        assert_eq!(picture.width, BACKGROUND_WIDTH);
        let ceiling = (255.0 * DIM).round() as u8 + 1;
        assert!(
            picture
                .rgba
                .chunks(4)
                .all(|p| p[..3].iter().all(|c| *c <= ceiling))
        );
    }

    #[test]
    fn the_setting_then_the_lock_pictures_then_the_wallpaper() {
        let dir = std::env::temp_dir().join(format!("lock-background-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("config/sherlockos");
        let data = dir.join("data/sherlockos");
        std::fs::create_dir_all(&config).unwrap();
        let env = |dir: &Path| {
            let (config, data) = (
                dir.join("config").into_os_string(),
                dir.join("data").into_os_string(),
            );
            let walls = dir.join("walls").into_os_string();
            move |key: &str| match key {
                "XDG_CONFIG_HOME" => Some(config.clone()),
                "XDG_DATA_HOME" => Some(data.clone()),
                "XDG_DATA_DIRS" => Some(OsString::from("/nonexistent")),
                "SHERLOCKOS_WALLPAPERS" => Some(walls.clone()),
                _ => None,
            }
        };
        // Nothing but the wallpaper: its smallest variant, blurred.
        let first = dir.join("walls/first-clue");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::write(first.join("1920x1080.png"), b"").unwrap();
        std::fs::write(first.join("3840x2160.png"), b"").unwrap();
        let blurred = Source::Blurred(first.join("1920x1080.png"));
        assert_eq!(source(env(&dir)), Some(blurred));
        // The lock pictures, installed.
        std::fs::create_dir_all(data.join(FOLDER)).unwrap();
        for name in [DARK, LIGHT] {
            std::fs::write(data.join(FOLDER).join(name), b"").unwrap();
        }
        let themed = Source::Themed {
            dark: data.join(FOLDER).join(DARK),
            light: data.join(FOLDER).join(LIGHT),
        };
        assert_eq!(source(env(&dir)), Some(themed));
        // The setting comes first.
        let chosen = PathBuf::from("/srv/lock.jpg");
        std::fs::write(config.join("lock.conf"), "background = /srv/lock.jpg\n").unwrap();
        assert_eq!(source(env(&dir)), Some(Source::Single(chosen)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
