//! The wallpaper setting, `<config dir>/wallpaper.conf`, and the geometry that applies it to a
//! screen: which variant to load, which part of it to draw, and where.
//!
//! ```text
//! # A wallpaper of the collection, by id: first-clue by default.
//! wallpaper = first-clue
//! # true (default): in the other theme, its pendant if it has one.
//! follow-theme = true
//! # Or an image of your own (PNG or JPEG), or a folder of variants named
//! # <width>x<height>.png: it comes before `wallpaper`.
//! path = /home/me/Images/wall.png
//! # cover (default), contain or center.
//! mode = cover
//! ```

use sherlock_ui::wallpapers::{self, ThemeVariant};
use std::{
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "wallpaper.conf";

/// Folder of the installed wallpapers, in each data folder of the desktop: one folder of
/// variants per wallpaper, named by its id.
pub const DATA_SUBDIR: &str = "wallpapers";

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// How the image fills a screen. Its aspect ratio is always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Fills the screen, centered: what overflows is cut.
    #[default]
    Cover,
    /// Whole image, centered: the desktop color fills the bands around it.
    Contain,
    /// One image pixel per screen pixel, centered: cut if larger, bands if smaller.
    Center,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cover => "cover",
            Self::Contain => "contain",
            Self::Center => "center",
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        [Self::Cover, Self::Contain, Self::Center]
            .into_iter()
            .find(|mode| mode.as_str() == s)
            .ok_or_else(|| format!("unknown mode {s:?}, expected cover, contain or center"))
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The wallpaper setting. By default, the default pair of the collection, following the theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// An image or a folder of variants of the user's: comes before `wallpaper`.
    pub path: Option<PathBuf>,
    /// Id of a wallpaper of the collection; `wallpapers::DEFAULT` without one.
    pub wallpaper: Option<String>,
    /// The wallpaper changes with the theme, to its pendant when it has one.
    pub follow_theme: bool,
    pub mode: Mode,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            path: None,
            wallpaper: None,
            follow_theme: true,
            mode: Mode::default(),
        }
    }
}

/// What a setting shows in a theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// The user's image or folder of variants.
    Path(PathBuf),
    /// A wallpaper installed in the data folders, by id (see `find`).
    Installed(String),
}

impl Config {
    /// What to show in the theme `variant`: the user's path, or else the chosen wallpaper (the
    /// default one without a choice), replaced by its pendant in that theme when it follows
    /// the theme and has one.
    pub fn choice(&self, variant: ThemeVariant) -> Choice {
        if let Some(path) = &self.path {
            return Choice::Path(path.clone());
        }
        let id = self.wallpaper.as_deref().unwrap_or(wallpapers::DEFAULT);
        let id = if self.follow_theme {
            wallpapers::for_theme(id, variant)
        } else {
            id
        };
        Choice::Installed(id.to_owned())
    }

    /// Reads `key = value` lines; `#` starts a comment line. Unknown keys and values are
    /// refused, so that a typo does not go unnoticed: the error of the first one is returned.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (config, mut problems) = Self::parse_lenient(text);
        match problems.is_empty() {
            true => Ok(config),
            false => Err(problems.remove(0)),
        }
    }

    /// Reads what can be read: each line that cannot is left at its default and listed.
    pub fn parse_lenient(text: &str) -> (Self, Vec<Error>) {
        let mut config = Self::default();
        let mut problems = Vec::new();
        for (line, pair) in crate::conf::pairs(text) {
            if let Err(reason) = pair.and_then(|(key, value)| config.take(key, value)) {
                problems.push(Error::Line { line, reason });
            }
        }
        (config, problems)
    }

    fn take(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "path" if value.is_empty() => self.path = None,
            "path" => self.path = Some(value.into()),
            "wallpaper" if value.is_empty() => self.wallpaper = None,
            "wallpaper" => self.wallpaper = Some(value.into()),
            "follow-theme" => {
                self.follow_theme = value
                    .parse()
                    .map_err(|_| format!("expected true or false, not {value:?}"))?;
            }
            "mode" => self.mode = value.parse()?,
            key => return Err(format!("unknown key {key:?}")),
        }
        Ok(())
    }

    /// Reads the settings file as `read_lenient` does, without the list of its wrong lines.
    pub fn read(path: &Path) -> Result<Self, Error> {
        Self::read_lenient(path).map(|(config, _)| config)
    }

    /// Reads the settings file. A missing file is the default setting. A line that cannot be
    /// taken is left out and listed, and the other lines still count: one typo never turns
    /// the whole file back to the defaults (a lock set to never would lock after 5 minutes).
    /// Only a file that cannot be read at all is an error.
    pub fn read_lenient(path: &Path) -> Result<(Self, Vec<Error>), Error> {
        match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
            Ok(text) => Ok(Self::parse_lenient(&text)),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(source) => Err(Error::Read {
                path: path.into(),
                source,
            }),
        }
    }

    /// Each key and its value as written in the file; an absent one is an empty value.
    fn values(&self) -> [(&'static str, String); 4] {
        let path = self.path.as_ref().map(|p| p.display().to_string());
        [
            ("wallpaper", self.wallpaper.clone().unwrap_or_default()),
            ("follow-theme", self.follow_theme.to_string()),
            ("path", path.unwrap_or_default()),
            ("mode", self.mode.to_string()),
        ]
    }

    /// `text`, the file as it is, with the keys whose value changed set again; everything
    /// else stays (see `touchpad::Config::write_into`). Control Room writes the file so.
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse_lenient(text);
        crate::conf::update(text, &self.values(), &read.values())
    }
}

/// `<config dir>/wallpaper.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

/// A folder of installed wallpapers looked in before the data folders: the session sets it to
/// `target/wallpapers`, what `just wallpapers` writes, when it runs from the repository.
pub const DIR_VAR: &str = "SHERLOCKOS_WALLPAPERS";

/// The folders of installed wallpapers, most important first: `$SHERLOCKOS_WALLPAPERS` when it
/// is an absolute path, then `wallpapers` in each data folder of the desktop
/// (`crate::data_dirs`). `var` reads the environment.
pub fn dirs(var: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let own = var(DIR_VAR)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute());
    own.into_iter()
        .chain(
            crate::data_dirs(var)
                .into_iter()
                .map(|dir| dir.join(DATA_SUBDIR)),
        )
        .collect()
}

/// Folder of variants of the installed wallpaper `id`, in the first of `dirs` (see `dirs`)
/// that has it.
pub fn find(id: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter().map(|dir| dir.join(id)).find(|dir| dir.is_dir())
}

/// A variant of a wallpaper: a PNG at one definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub path: PathBuf,
    pub size: (u32, u32),
}

/// The variants of a folder: the files named `<width>x<height>.png`, smallest first.
pub fn variants(dir: &Path) -> io::Result<Vec<Variant>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let size = path
            .file_name()
            .and_then(|name| name.to_str()?.strip_suffix(".png"))
            .and_then(|name| name.split_once('x'))
            .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
        if let Some(size @ (w, h)) = size
            && w > 0
            && h > 0
        {
            found.push(Variant { path, size });
        }
    }
    found.sort_by_key(|v| u64::from(v.size.0) * u64::from(v.size.1));
    Ok(found)
}

/// The variant to load for a screen of `screen` pixels: the smallest one that is not enlarged
/// to fill it, so that it stays sharp without loading more than needed; failing that, the
/// largest. `variants` comes from `variants`, smallest first.
pub fn pick(variants: &[Variant], screen: (u32, u32), mode: Mode) -> Option<&Variant> {
    let factor = |v: &Variant| scale(mode, v.size, screen);
    variants
        .iter()
        .filter(|v| factor(v) <= 1.0 + 1e-9)
        .max_by(|a, b| factor(a).total_cmp(&factor(b)))
        .or_else(|| {
            variants
                .iter()
                .min_by(|a, b| factor(a).total_cmp(&factor(b)))
        })
}

/// A rectangle, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Which part of the image is drawn (`src`, in image pixels) and where (`dst`, in screen
/// pixels from its top left corner). Outside `dst`, the desktop color shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub src: Rect,
    pub dst: Rect,
}

/// How many screen pixels an image pixel takes in `mode`.
fn scale(mode: Mode, image: (u32, u32), screen: (u32, u32)) -> f64 {
    let sx = f64::from(screen.0) / f64::from(image.0.max(1));
    let sy = f64::from(screen.1) / f64::from(image.1.max(1));
    match mode {
        Mode::Cover => sx.max(sy),
        Mode::Contain => sx.min(sy),
        Mode::Center => 1.0,
    }
}

/// Places an image of `image` pixels on a screen of `screen` pixels: the image is scaled
/// (`mode`), centered, then cut to the screen. The scale of the screen does not matter: it is
/// already in its size in pixels.
pub fn fit(mode: Mode, image: (u32, u32), screen: (u32, u32)) -> Fit {
    let s = scale(mode, image, screen);
    // Along one axis: the visible span on screen, and the image span it shows.
    let axis = |image: u32, screen: u32| {
        let (image, screen) = (f64::from(image), f64::from(screen));
        let drawn = image * s;
        let start = (screen - drawn) / 2.0;
        let (lo, hi) = (start.max(0.0), (start + drawn).min(screen));
        let src = ((lo - start) / s, (hi - lo) / s);
        ((lo, hi - lo), src)
    };
    let ((dx, dw), (sx, sw)) = axis(image.0, screen.0);
    let ((dy, dh), (sy, sh)) = axis(image.1, screen.1);
    Fit {
        src: Rect {
            x: sx,
            y: sy,
            w: sw,
            h: sh,
        },
        dst: Rect {
            x: dx,
            y: dy,
            w: dw,
            h: dh,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }

    #[track_caller]
    fn close(a: Rect, b: Rect) {
        let d = [a.x - b.x, a.y - b.y, a.w - b.w, a.h - b.h];
        assert!(d.iter().all(|d| d.abs() < 1e-6), "{a:?} != {b:?}");
    }

    #[test]
    fn the_settings_are_read_and_written_back() {
        let text = "# comment\n\npath = /walls/first-clue \nmode=contain\n";
        let config = Config::parse(text).unwrap();
        assert_eq!(config.path.as_deref(), Some(Path::new("/walls/first-clue")));
        assert_eq!(config.mode, Mode::Contain);
        assert_eq!(Config::parse(&config.write_into("")).unwrap(), config);
        // Empty: the default wallpaper, in cover; nothing to write.
        assert_eq!(Config::parse("").unwrap(), Config::default());
        assert_eq!(Config::default().write_into(""), "");
        // Choosing a wallpaper of the collection empties the path in place, the comment stays.
        let mut chosen = config.clone();
        (chosen.wallpaper, chosen.path) = (Some("ring".into()), None);
        assert_eq!(
            chosen.write_into(text),
            "# comment\n\npath = \nmode=contain\nwallpaper = ring\n"
        );
        let config = Config::parse("wallpaper = ring\nfollow-theme = false\n").unwrap();
        assert_eq!(config.wallpaper.as_deref(), Some("ring"));
        assert!(!config.follow_theme);
        assert_eq!(Config::parse(&config.write_into("")).unwrap(), config);
    }

    #[test]
    fn the_choice_follows_the_theme_variant() {
        use ThemeVariant::{Dark, Light};
        let installed = |id: &str| Choice::Installed(id.into());
        // By default: the default pair.
        let config = Config::default();
        assert_eq!(config.choice(Dark), installed("first-clue"));
        assert_eq!(config.choice(Light), installed("first-clue-light"));
        // Without following the theme, the chosen one stays.
        let fixed = Config {
            follow_theme: false,
            ..Config::default()
        };
        assert_eq!(fixed.choice(Light), installed("first-clue"));
        // A wallpaper without a pendant stays itself in both themes.
        let ring = Config {
            wallpaper: Some("ring".into()),
            ..Config::default()
        };
        assert_eq!(ring.choice(Light), installed("ring"));
        // The user's path comes first, whatever the theme.
        let own = Config {
            path: Some("/walls/mine.png".into()),
            ..ring
        };
        assert_eq!(own.choice(Light), Choice::Path("/walls/mine.png".into()));
    }

    #[test]
    fn a_wrong_line_is_refused_with_its_number() {
        let err = Config::parse("mode = cover\nmode = stretch").unwrap_err();
        assert!(matches!(err, Error::Line { line: 2, .. }), "{err}");
        assert!(matches!(
            Config::parse("size = 3"),
            Err(Error::Line { line: 1, .. })
        ));
        assert!(matches!(
            Config::parse("follow-theme = yes"),
            Err(Error::Line { line: 1, .. })
        ));
        assert!(matches!(
            Config::parse("path"),
            Err(Error::Line { line: 1, .. })
        ));
    }

    #[test]
    fn a_missing_file_is_the_default_setting() {
        let missing = Path::new("/nonexistent/wallpaper.conf");
        assert_eq!(Config::read(missing).unwrap(), Config::default());
        // A folder cannot be read as a file.
        assert!(matches!(
            Config::read(Path::new("/")),
            Err(Error::Read { .. })
        ));
    }

    #[test]
    fn the_settings_file_is_in_the_config_dir() {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| "/cfg".into());
        assert_eq!(
            config_file(var),
            Some("/cfg/sherlockos/wallpaper.conf".into())
        );
    }

    fn variant(w: u32, h: u32) -> Variant {
        Variant {
            path: format!("{w}x{h}.png").into(),
            size: (w, h),
        }
    }

    #[test]
    fn the_variant_is_the_smallest_that_stays_sharp() {
        let all = [
            variant(1920, 1080),
            variant(2560, 1440),
            variant(3840, 2160),
        ];
        let pick = |w, h, mode| pick(&all, (w, h), mode).map(|v| v.size);
        // 16:9 screens.
        assert_eq!(pick(1920, 1080, Mode::Cover), Some((1920, 1080)));
        assert_eq!(pick(1366, 768, Mode::Cover), Some((1920, 1080)));
        assert_eq!(pick(2560, 1440, Mode::Cover), Some((2560, 1440)));
        // 16:10: the height decides in cover. 1440 × 900 at scale 1, then the same screen at
        // scale 2 (2880 × 1800 pixels).
        assert_eq!(pick(1440, 900, Mode::Cover), Some((1920, 1080)));
        assert_eq!(pick(2560, 1600, Mode::Cover), Some((3840, 2160)));
        assert_eq!(pick(2880, 1800, Mode::Cover), Some((3840, 2160)));
        // In contain, the width decides: 2560 × 1600 shows 2560 × 1440 without enlarging it.
        assert_eq!(pick(2560, 1600, Mode::Contain), Some((2560, 1440)));
        // Larger than every variant: the largest.
        assert_eq!(pick(5120, 2880, Mode::Cover), Some((3840, 2160)));
        assert_eq!(super::pick(&[], (1920, 1080), Mode::Cover), None);
    }

    #[test]
    fn cover_fills_the_screen_and_cuts_the_overflow() {
        // 16:9 image on a 16:10 screen: the sides are cut.
        let fit = fit(Mode::Cover, (1920, 1080), (1440, 900));
        close(fit.dst, rect(0.0, 0.0, 1440.0, 900.0));
        close(fit.src, rect(96.0, 0.0, 1728.0, 1080.0));
        // The same screen at scale 2 shows the same part of the 4K variant.
        let fit = super::fit(Mode::Cover, (3840, 2160), (2880, 1800));
        close(fit.dst, rect(0.0, 0.0, 2880.0, 1800.0));
        close(fit.src, rect(192.0, 0.0, 3456.0, 2160.0));
        // Same ratio: the whole image.
        let fit = super::fit(Mode::Cover, (1920, 1080), (2560, 1440));
        close(fit.src, rect(0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn contain_shows_the_whole_image_between_bands() {
        // 16:9 on 16:10: bands at the top and the bottom.
        let fit = fit(Mode::Contain, (1920, 1080), (1440, 900));
        close(fit.src, rect(0.0, 0.0, 1920.0, 1080.0));
        close(fit.dst, rect(0.0, 45.0, 1440.0, 810.0));
        // At scale 2, everything doubles.
        let fit = super::fit(Mode::Contain, (3840, 2160), (2880, 1800));
        close(fit.dst, rect(0.0, 90.0, 2880.0, 1620.0));
    }

    #[test]
    fn center_keeps_one_image_pixel_per_screen_pixel() {
        // Smaller image: centered, bands around.
        let fit = fit(Mode::Center, (1000, 500), (1440, 900));
        close(fit.src, rect(0.0, 0.0, 1000.0, 500.0));
        close(fit.dst, rect(220.0, 200.0, 1000.0, 500.0));
        // Larger image: its middle.
        let fit = super::fit(Mode::Center, (3840, 2160), (1440, 900));
        close(fit.src, rect(1200.0, 630.0, 1440.0, 900.0));
        close(fit.dst, rect(0.0, 0.0, 1440.0, 900.0));
    }

    #[test]
    fn the_variants_of_a_folder_are_found_and_sorted() {
        let dir = std::env::temp_dir().join(format!("desktop-config-{}", std::process::id()));
        let wallpaper = dir.join(DATA_SUBDIR).join("first-clue");
        fs::create_dir_all(&wallpaper).unwrap();
        for name in [
            "3840x2160.png",
            "1920x1080.png",
            "notes.txt",
            "0x10.png",
            "axb.png",
        ] {
            fs::write(wallpaper.join(name), b"").unwrap();
        }
        let found = variants(&wallpaper).unwrap();
        let sizes: Vec<_> = found.iter().map(|v| v.size).collect();
        assert_eq!(sizes, [(1920, 1080), (3840, 2160)]);
        let dirs = [dir.join("missing"), dir.join(DATA_SUBDIR)];
        assert_eq!(find("first-clue", &dirs), Some(wallpaper));
        assert_eq!(find("ring", &dirs), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_folder_of_the_session_comes_before_the_data_folders() {
        let env = |pairs: &'static [(&str, &str)]| {
            move |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into())
        };
        assert_eq!(
            dirs(env(&[
                ("HOME", "/home/u"),
                ("XDG_DATA_DIRS", "/usr/share"),
                (DIR_VAR, "/repo/target/wallpapers"),
            ])),
            [
                PathBuf::from("/repo/target/wallpapers"),
                "/home/u/.local/share/sherlockos/wallpapers".into(),
                "/usr/share/sherlockos/wallpapers".into(),
            ]
        );
        // A relative folder is ignored, like the XDG ones.
        assert_eq!(
            dirs(env(&[("HOME", "/home/u"), (DIR_VAR, "target/wallpapers")])).len(),
            3
        );
    }
}
