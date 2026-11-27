//! The backdrop of the view (lot P-02c): the wallpaper of the active theme, blurred and darkened,
//! as Launchpad's. It does not wait for the blur of the compositor (D-37): the picture is
//! computed once per wallpaper and per screen, on a copy reduced to `BACKDROP_WIDTH`, then kept
//! in `~/.cache/sherlockos/` and read from there at the next start. A new wallpaper, another
//! theme or another screen make another key, so another picture.
//!
//! The wallpaper may come from the user (`path` of `wallpaper.conf`): it is read only if it is a
//! regular file of `MAX_FILE` bytes at most, and decoded with bounded sides and memory, as the
//! lock screen does. Anything else leaves the plain veil.

use std::ffi::OsString;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use desktop_config::wallpaper::{self, Choice, Mode};
use image::{DynamicImage, RgbaImage, imageops};
use sherlock_ui::apps::{BACKDROP_BLUR, BACKDROP_DIM, BACKDROP_MAX_LUMINANCE, BACKDROP_WIDTH};
use sherlock_ui::color::{self, Color};
use sherlock_ui::wallpapers::ThemeVariant;

/// The largest picture file read, its largest side, the memory its decoding may take.
pub const MAX_FILE: u64 = 64 * 1024 * 1024;
pub const MAX_SIDE: u32 = 16_384;
pub const MAX_ALLOC: u64 = 256 * 1024 * 1024;

/// The folder of the desktop in the cache, the name of the blurred copies, how many are kept.
pub const CACHE_SUBDIR: &str = "sherlockos";
const PREFIX: &str = "apps-backdrop-";
const KEPT: usize = 6;
/// Changed with the processing, so that an older copy is never reused.
const VERSION: u32 = 1;

/// Why there is no blurred wallpaper.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} is not a regular file of {MAX_FILE} bytes at most")]
    Refused(PathBuf),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Image(#[from] image::ImageError),
}

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

/// What a loading found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The same key as the picture shown: nothing to do.
    Unchanged,
    /// A new picture, and its key.
    New(u64, Picture),
    /// No wallpaper to blur: the plain veil.
    Nothing,
}

/// The wallpaper file of `theme`: the user's image, or the smallest variant of the wallpaper
/// chosen (blurred, its details do not matter).
pub fn wallpaper(var: impl Fn(&str) -> Option<OsString>, theme: color::Theme) -> Option<PathBuf> {
    let config = wallpaper::config_file(&var)
        .and_then(|file| wallpaper::Config::read(&file).ok())
        .unwrap_or_default();
    let variant = match theme {
        color::Theme::Light => ThemeVariant::Light,
        color::Theme::Dark => ThemeVariant::Dark,
    };
    let path = match config.choice(variant) {
        Choice::Path(path) => path,
        Choice::Installed(id) => wallpaper::find(&id, &wallpaper::dirs(&var))?,
    };
    if !path.is_dir() {
        return Some(path);
    }
    let variants = wallpaper::variants(&path).ok()?;
    variants.into_iter().next().map(|variant| variant.path)
}

/// `$XDG_CACHE_HOME/sherlockos`, or `~/.cache/sherlockos`.
pub fn cache_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let absolute = |key: &str| var(key).map(PathBuf::from).filter(|p| p.is_absolute());
    let cache = absolute("XDG_CACHE_HOME").or_else(|| absolute("HOME").map(|h| h.join(".cache")));
    cache.map(|dir| dir.join(CACHE_SUBDIR))
}

/// The key of a blurred copy: the file, its time and length, the screen in pixels and the
/// processing (FNV-1a, the same from one run to the next).
pub fn key(path: &Path, screen: (u32, u32)) -> Option<u64> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |t| t.as_nanos());
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(path.as_os_str().as_encoded_bytes());
    eat(&modified.to_le_bytes());
    eat(&meta.len().to_le_bytes());
    eat(&screen.0.to_le_bytes());
    eat(&screen.1.to_le_bytes());
    eat(&VERSION.to_le_bytes());
    Some(hash)
}

/// Decodes `path` within the bounds above.
fn decode(path: &Path) -> Result<DynamicImage, Error> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return Err(Error::Refused(path.to_owned()));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(Error::Refused(path.to_owned()));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    reader.limits(limits);
    Ok(reader.decode()?)
}

/// `path` cut to cover a screen of `screen` pixels, reduced, blurred and darkened.
pub fn blur(path: &Path, screen: (u32, u32)) -> Result<Picture, Error> {
    let picture = decode(path)?.to_rgba8();
    let screen = (screen.0.max(1), screen.1.max(1));
    let fit = wallpaper::fit(Mode::Cover, picture.dimensions(), screen).src;
    let cut = imageops::crop_imm(
        &picture,
        fit.x.round() as u32,
        fit.y.round() as u32,
        (fit.w.round() as u32).max(1),
        (fit.h.round() as u32).max(1),
    )
    .to_image();
    let width = BACKDROP_WIDTH.min(cut.width());
    let height = ((u64::from(width) * u64::from(screen.1)) / u64::from(screen.0)).max(1) as u32;
    let small = imageops::resize(&cut, width, height, imageops::FilterType::Triangle);
    let mut soft = imageops::blur(&small, BACKDROP_BLUR);
    darken(&mut soft);
    Ok(Picture {
        width,
        height,
        rgba: soft.into_raw(),
    })
}

/// The brightest relative luminance of `image`.
fn brightest(image: &RgbaImage) -> f32 {
    image
        .pixels()
        .map(|p| Color::rgb(p[0], p[1], p[2]).luminance())
        .fold(0.0, f32::max)
}

/// Darkens by `BACKDROP_DIM` at least, and more until no point is brighter than
/// `BACKDROP_MAX_LUMINANCE`: slightly for a dark wallpaper, much for a white one.
fn darken(image: &mut RgbaImage) {
    let scale = |image: &RgbaImage, factor: f32| {
        let mut out = image.clone();
        for pixel in out.pixels_mut() {
            for channel in &mut pixel.0[..3] {
                *channel = (f32::from(*channel) * factor).round() as u8;
            }
        }
        out
    };
    let top = brightest(image).max(f32::EPSILON);
    // About the square of the factor on the luminance: the first guess, then smaller steps.
    let mut factor = BACKDROP_DIM.min((BACKDROP_MAX_LUMINANCE / top).powf(1.0 / 2.2));
    let mut out = scale(image, factor);
    while brightest(&out) > BACKDROP_MAX_LUMINANCE && factor > 0.0 {
        factor -= 0.02;
        out = scale(image, factor.max(0.0));
    }
    *image = out;
}

/// The blurred copy of key `key` in `dir`, if there is one.
fn cached(dir: &Path, key: u64) -> Option<Picture> {
    let picture = decode(&file(dir, key)).ok()?.to_rgba8();
    Some(Picture {
        width: picture.width(),
        height: picture.height(),
        rgba: picture.into_raw(),
    })
}

fn file(dir: &Path, key: u64) -> PathBuf {
    dir.join(format!("{PREFIX}{key:016x}.png"))
}

/// Writes the copy of `key` (whole, or not at all), then keeps the `KEPT` newest ones.
fn store(dir: &Path, key: u64, picture: &Picture) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    let path = file(dir, key);
    let part = path.with_extension("part");
    image::save_buffer_with_format(
        &part,
        &picture.rgba,
        picture.width,
        picture.height,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    std::fs::rename(&part, &path)?;
    let mut copies: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(PREFIX))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    copies.sort_by_key(|copy| std::cmp::Reverse(copy.0));
    for (_, old) in copies.into_iter().skip(KEPT) {
        let _ = std::fs::remove_file(old);
    }
    Ok(())
}

/// The blurred wallpaper of `theme` for a screen of `screen` pixels: nothing to do if its key is
/// `current`, else from the cache `cache`, else computed and kept there.
pub fn load(
    var: impl Fn(&str) -> Option<OsString>,
    theme: color::Theme,
    screen: (u32, u32),
    cache: Option<&Path>,
    current: Option<u64>,
) -> Outcome {
    let Some(path) = wallpaper(&var, theme) else {
        return Outcome::Nothing;
    };
    let Some(key) = key(&path, screen) else {
        return Outcome::Nothing;
    };
    if current == Some(key) {
        return Outcome::Unchanged;
    }
    if let Some(picture) = cache.and_then(|dir| cached(dir, key)) {
        return Outcome::New(key, picture);
    }
    let started = std::time::Instant::now();
    match blur(&path, screen) {
        Ok(picture) => {
            tracing::info!(
                ?path,
                took_ms = started.elapsed().as_millis(),
                "wallpaper blurred"
            );
            if let Some(dir) = cache
                && let Err(error) = store(dir, key, &picture)
            {
                tracing::warn!(%error, "cannot keep the blurred wallpaper");
            }
            Outcome::New(key, picture)
        }
        Err(error) => {
            tracing::warn!(?path, %error, "cannot blur the wallpaper");
            Outcome::Nothing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wallpapers")
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apps-backdrop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_wallpaper_is_cut_to_the_screen_blurred_and_darkened() {
        for name in ["first-clue.jpg", "first-clue-light.jpg"] {
            for screen in [(1920, 1080), (1366, 768), (2560, 1600)] {
                let picture = blur(&assets().join(name), screen).unwrap();
                assert_eq!(picture.width, BACKDROP_WIDTH);
                let ratio = picture.height as f32 / picture.width as f32;
                assert!((ratio - screen.1 as f32 / screen.0 as f32).abs() < 0.01);
                let image =
                    RgbaImage::from_raw(picture.width, picture.height, picture.rgba).unwrap();
                assert!(brightest(&image) <= BACKDROP_MAX_LUMINANCE, "{name}");
            }
        }
        // A white wallpaper: darkened enough for white names.
        let dir = folder("white");
        let white = dir.join("white.png");
        RgbaImage::from_pixel(64, 36, image::Rgba([255, 255, 255, 255]))
            .save(&white)
            .unwrap();
        let picture = blur(&white, (1920, 1080)).unwrap();
        let image = RgbaImage::from_raw(picture.width, picture.height, picture.rgba).unwrap();
        assert!(brightest(&image) <= BACKDROP_MAX_LUMINANCE);
        assert!(
            brightest(&image) > BACKDROP_MAX_LUMINANCE * 0.8,
            "not darker than needed"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_picture_is_computed_once_per_wallpaper_and_screen() {
        let dir = folder("cache");
        let walls = dir.join("walls/first-clue");
        std::fs::create_dir_all(&walls).unwrap();
        std::fs::copy(assets().join("first-clue.jpg"), walls.join("1920x1080.png")).unwrap();
        let (home, walls_var) = (
            dir.clone().into_os_string(),
            dir.join("walls").into_os_string(),
        );
        let var = |key: &str| match key {
            "HOME" => Some(home.clone()),
            "XDG_CONFIG_HOME" => Some(dir.join("config").into_os_string()),
            "XDG_DATA_DIRS" => Some(OsString::from("/nonexistent")),
            "SHERLOCKOS_WALLPAPERS" => Some(walls_var.clone()),
            _ => None,
        };
        let cache = cache_dir(var).unwrap();
        assert_eq!(cache, dir.join(".cache/sherlockos"));
        let Outcome::New(key, first) =
            load(var, color::Theme::Dark, (1920, 1080), Some(&cache), None)
        else {
            panic!("blurred");
        };
        let file = file(&cache, key);
        assert!(file.is_file(), "kept");
        // The same key: nothing to do; read again: the same picture, from the cache.
        let same = load(
            var,
            color::Theme::Dark,
            (1920, 1080),
            Some(&cache),
            Some(key),
        );
        assert_eq!(same, Outcome::Unchanged);
        assert_eq!(
            load(var, color::Theme::Dark, (1920, 1080), Some(&cache), None),
            Outcome::New(key, first)
        );
        // Another screen: another key.
        let Outcome::New(other, _) = load(
            var,
            color::Theme::Dark,
            (1366, 768),
            Some(&cache),
            Some(key),
        ) else {
            panic!("another screen");
        };
        assert_ne!(other, key);
        // The wallpaper changed: another key again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::copy(assets().join("lagoon.jpg"), walls.join("1920x1080.png")).unwrap();
        assert!(matches!(
            load(var, color::Theme::Dark, (1920, 1080), Some(&cache), Some(key)),
            Outcome::New(changed, _) if changed != key
        ));
        // A broken file: the plain veil, no panic.
        std::fs::write(walls.join("1920x1080.png"), b"not a picture").unwrap();
        assert_eq!(
            load(var, color::Theme::Dark, (1920, 1080), Some(&cache), None),
            Outcome::Nothing
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
