//! The picture behind the lock screen (lot D-25b): the `background` of `lock.conf`, or else the
//! wallpaper of the desktop in the active theme, blurred and darkened. The blur is computed once,
//! at start, on a thread of its own, on a reduced copy (`BACKGROUND_WIDTH` wide): drawn back at
//! the size of each screen, the reduction blurs it further for free. Until it is ready, or if
//! nothing can be read, the plain background of the system screens shows.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use desktop_config::wallpaper::{self, Choice};
use image::{GenericImageView as _, imageops};
use sherlock_ui::color;
use sherlock_ui::lock::screen::{BACKGROUND_WIDTH, BLUR_SIGMA, DIM};
use sherlock_ui::wallpapers::ThemeVariant;

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

/// The file to show: `background` of `lock.conf`, else the wallpaper of the desktop (its
/// smallest variant: it is blurred anyway). `var` reads the environment.
pub fn source(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let lock = desktop_config::lock::config_file(&var)
        .map(|file| desktop_config::lock::Config::read(&file))
        .and_then(|config| {
            config
                .inspect_err(|err| tracing::warn!("lock.conf: {err}"))
                .ok()
        });
    if let Some(path) = lock.and_then(|config| config.background) {
        return smallest(&path);
    }
    let config = wallpaper::config_file(&var)
        .and_then(|file| wallpaper::Config::read(&file).ok())
        .unwrap_or_default();
    let variant = match color::active() {
        color::Theme::Light => ThemeVariant::Light,
        color::Theme::Dark => ThemeVariant::Dark,
    };
    match config.choice(variant) {
        Choice::Path(path) => smallest(&path),
        Choice::Installed(id) => smallest(&wallpaper::find(&id, &wallpaper::dirs(&var))?),
    }
}

/// A file as is, or the smallest variant of a folder of variants.
fn smallest(path: &Path) -> Option<PathBuf> {
    if !path.is_dir() {
        return Some(path.to_owned());
    }
    let variants = wallpaper::variants(path).ok()?;
    variants.into_iter().next().map(|variant| variant.path)
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

/// `source` then `blurred`, logged: what the lock screen shows behind it.
pub fn load(var: impl Fn(&str) -> Option<OsString>) -> Option<Picture> {
    let path = source(var)?;
    blurred(&path)
        .inspect_err(|err| tracing::warn!(path = %path.display(), "lock background: {err}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wallpapers")
    }

    #[test]
    fn the_picture_is_reduced_blurred_and_darkened() {
        let path = assets().join("first-clue.jpg");
        let original = image::open(&path).unwrap();
        let picture = blurred(&path).unwrap();
        assert_eq!(picture.width, BACKGROUND_WIDTH);
        let ratio = f64::from(original.height()) / f64::from(original.width());
        let expected = (ratio * f64::from(BACKGROUND_WIDTH)).round();
        assert!((f64::from(picture.height) - expected).abs() <= 1.0);
        assert_eq!(
            picture.rgba.len(),
            (picture.width * picture.height * 4) as usize
        );
        // Darkened: no channel stays above `DIM` of full white.
        let ceiling = (255.0 * DIM).round() as u8 + 1;
        assert!(
            picture
                .rgba
                .chunks(4)
                .all(|p| p[..3].iter().all(|c| *c <= ceiling))
        );
    }

    #[test]
    fn the_setting_of_the_lock_comes_before_the_wallpaper() {
        let dir = std::env::temp_dir().join(format!("lock-background-{}", std::process::id()));
        let config = dir.join("sherlockos");
        std::fs::create_dir_all(&config).unwrap();
        let chosen = assets().join("aurora.jpg");
        std::fs::write(
            config.join("lock.conf"),
            format!("background = {}\n", chosen.display()),
        )
        .unwrap();
        let home = dir.clone().into_os_string();
        let var = move |key: &str| (key == "XDG_CONFIG_HOME").then(|| home.clone());
        assert_eq!(source(&var), Some(chosen));
        // Without it, the default wallpaper of the collection, from the data folders.
        std::fs::remove_file(config.join("lock.conf")).unwrap();
        let walls = dir.join("walls");
        let first = walls.join("first-clue");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::write(first.join("1920x1080.png"), b"").unwrap();
        std::fs::write(first.join("3840x2160.png"), b"").unwrap();
        let (home, walls) = (dir.clone().into_os_string(), walls.into_os_string());
        let var = move |key: &str| match key {
            "XDG_CONFIG_HOME" => Some(home.clone()),
            "SHERLOCKOS_WALLPAPERS" => Some(walls.clone()),
            _ => None,
        };
        assert_eq!(source(&var), Some(first.join("1920x1080.png")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
