//! The screens' setting, `<config dir>/displays.conf` (ADR 0025, lot D-26b): for each screen
//! met, named by its make, model and serial number (read from its EDID), the mode, scale,
//! position, rotation and whether it is on. The compositor writes it once a configuration has
//! held for a while, and applies the saved one when that screen comes back. Clients change the
//! screens through `wlr-output-management`, never through this file.
//!
//! ```text
//! [screen]
//! make = BOE
//! model = NE135FBM-N41
//! serial = 0
//! mode = 2256x1504@60.000
//! scale = 1.5
//! position = 0,0
//! transform = normal
//! enabled = true
//! ```

use std::{
    ffi::OsString,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "displays.conf";

/// The most screens kept: the oldest go first.
pub const MAX_SCREENS: usize = 32;
/// The longest make, model or serial kept, in characters.
pub const MAX_NAME: usize = 64;
/// The largest file read: anything longer is not ours.
pub const MAX_BYTES: u64 = 64 * 1024;
/// Width and height of a mode, in pixels.
pub const MAX_PIXELS: i32 = 16_384;
/// Refresh of a mode, in millihertz.
pub const MAX_REFRESH: i32 = 1_000_000;
/// Smallest and largest scale.
pub const SCALES: (f64, f64) = (0.5, 4.0);
/// How far from the origin a screen may sit, in logical pixels.
pub const MAX_POSITION: i32 = 65_536;

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{path} is larger than {MAX_BYTES} bytes")]
    TooLarge { path: PathBuf },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// Who a screen is: its make, model and serial number, as its EDID gives them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Identity {
    pub make: String,
    pub model: String,
    pub serial: String,
}

impl Identity {
    /// Keeps printable characters only, at most `MAX_NAME` of them.
    pub fn new(make: &str, model: &str, serial: &str) -> Self {
        Self {
            make: clean(make),
            model: clean(model),
            serial: clean(serial),
        }
    }
}

fn clean(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    text.trim().chars().take(MAX_NAME).collect()
}

/// A rotation of the screen, clockwise, maybe flipped first: `wl_output.transform`, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    pub const ALL: [Self; 8] = [
        Self::Normal,
        Self::Rotate90,
        Self::Rotate180,
        Self::Rotate270,
        Self::Flipped,
        Self::Flipped90,
        Self::Flipped180,
        Self::Flipped270,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Rotate90 => "90",
            Self::Rotate180 => "180",
            Self::Rotate270 => "270",
            Self::Flipped => "flipped",
            Self::Flipped90 => "flipped-90",
            Self::Flipped180 => "flipped-180",
            Self::Flipped270 => "flipped-270",
        }
    }

    /// Its value in `wl_output.transform`.
    pub fn code(self) -> u32 {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as u32
    }

    pub fn from_code(code: u32) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }
}

/// A mode: size in pixels and refresh in millihertz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
}

impl Mode {
    fn parse(value: &str) -> Option<Self> {
        let (size, refresh) = value.split_once('@').unwrap_or((value, "0"));
        let (width, height) = size.split_once('x')?;
        let hertz: f64 = refresh.trim().parse().ok()?;
        let mode = Self {
            width: width.trim().parse().ok()?,
            height: height.trim().parse().ok()?,
            refresh: (hertz * 1000.0).round() as i32,
        };
        mode.is_valid().then_some(mode)
    }

    pub fn is_valid(&self) -> bool {
        (1..=MAX_PIXELS).contains(&self.width)
            && (1..=MAX_PIXELS).contains(&self.height)
            && (0..=MAX_REFRESH).contains(&self.refresh)
    }
}

/// What is saved of a screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub identity: Identity,
    /// `None`: the screen's preferred mode.
    pub mode: Option<Mode>,
    pub scale: f64,
    pub position: (i32, i32),
    pub transform: Transform,
    pub enabled: bool,
}

impl Screen {
    pub fn new(identity: Identity) -> Self {
        Self {
            identity,
            mode: None,
            scale: 1.0,
            position: (0, 0),
            transform: Transform::Normal,
            enabled: true,
        }
    }
}

/// Every screen saved, the most recently changed last.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub screens: Vec<Screen>,
}

impl Config {
    /// What was saved of the screen `identity`.
    pub fn get(&self, identity: &Identity) -> Option<&Screen> {
        self.screens.iter().find(|s| &s.identity == identity)
    }

    /// Saves `screen`, in place of what was saved of it; the oldest screen goes past
    /// `MAX_SCREENS`.
    pub fn put(&mut self, screen: Screen) {
        self.screens.retain(|s| s.identity != screen.identity);
        self.screens.push(screen);
        let extra = self.screens.len().saturating_sub(MAX_SCREENS);
        self.screens.drain(..extra);
    }

    /// Reads `[screen]` groups of `key = value` lines; `#` starts a comment line. A line that
    /// cannot be read is left at its default and listed; a group without make nor model is
    /// left out.
    pub fn parse_lenient(text: &str) -> (Self, Vec<Error>) {
        let mut config = Self::default();
        let mut problems = Vec::new();
        let mut current: Option<Screen> = None;
        let finish = |config: &mut Self, screen: Option<Screen>| {
            if let Some(screen) =
                screen.filter(|s| !(s.identity.make.is_empty() && s.identity.model.is_empty()))
            {
                config.put(screen);
            }
        };
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line == "[screen]" {
                finish(&mut config, current.take());
                current = Some(Screen::new(Identity::default()));
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let taken = match (&mut current, line.split_once('=')) {
                (None, _) => Err("expected [screen] first".to_owned()),
                (_, None) => Err("expected key = value".to_owned()),
                (Some(screen), Some((key, value))) => read_key(screen, key.trim(), value.trim()),
            };
            if let Err(reason) = taken {
                problems.push(Error::Line {
                    line: number + 1,
                    reason,
                });
            }
        }
        finish(&mut config, current);
        (config, problems)
    }

    /// Reads the settings file, what can be read of it. A missing file is no screen saved.
    pub fn read(path: &Path) -> Result<(Self, Vec<Error>), Error> {
        let fail = |source| Error::Read {
            path: path.into(),
            source,
        };
        match fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_BYTES => {
                return Err(Error::TooLarge { path: path.into() });
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok((Self::default(), Vec::new()));
            }
            Err(err) => return Err(fail(err)),
            Ok(_) => {}
        }
        // Bounded again: a link to a device has no length to check first.
        let text = mime_apps::read_bounded(path, MAX_BYTES).map_err(fail)?;
        Ok(Self::parse_lenient(&text))
    }

    /// The whole file. The compositor owns it and writes it whole.
    pub fn to_text(&self) -> String {
        let mut text = String::from(
            "# The screens met, one group each, written by the compositor (ADR 0025).\n",
        );
        for s in &self.screens {
            let _ = write!(
                text,
                "\n[screen]\nmake = {}\nmodel = {}\nserial = {}\n",
                s.identity.make, s.identity.model, s.identity.serial
            );
            if let Some(m) = s.mode {
                let hertz = f64::from(m.refresh) / 1000.0;
                let _ = writeln!(text, "mode = {}x{}@{hertz:.3}", m.width, m.height);
            }
            let _ = write!(
                text,
                "scale = {}\nposition = {},{}\ntransform = {}\nenabled = {}\n",
                s.scale,
                s.position.0,
                s.position.1,
                s.transform.as_str(),
                s.enabled
            );
        }
        text
    }
}

fn read_key(screen: &mut Screen, key: &str, value: &str) -> Result<(), String> {
    let wrong = |what: &str| format!("{key}: expected {what}, not {value:?}");
    match key {
        "make" => screen.identity.make = clean(value),
        "model" => screen.identity.model = clean(value),
        "serial" => screen.identity.serial = clean(value),
        "mode" => screen.mode = Some(Mode::parse(value).ok_or_else(|| wrong("WxH@Hz"))?),
        "scale" => {
            screen.scale = value
                .parse()
                .ok()
                .filter(|s| (SCALES.0..=SCALES.1).contains(s))
                .ok_or_else(|| wrong("a scale from 0.5 to 4"))?;
        }
        "position" => {
            let in_range = |v: &str| {
                v.trim()
                    .parse::<i32>()
                    .ok()
                    .filter(|v| v.abs() <= MAX_POSITION)
            };
            screen.position = value
                .split_once(',')
                .and_then(|(x, y)| Some((in_range(x)?, in_range(y)?)))
                .ok_or_else(|| wrong("x,y"))?;
        }
        "transform" => {
            screen.transform = Transform::ALL
                .into_iter()
                .find(|t| t.as_str() == value)
                .ok_or_else(|| wrong("normal, 90, 180, 270 or flipped…"))?;
        }
        "enabled" => {
            screen.enabled = value.parse().map_err(|_| wrong("true or false"))?;
        }
        _ => return Err(format!("unknown key {key:?}")),
    }
    Ok(())
}

/// `<config dir>/displays.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> Screen {
        Screen {
            identity: Identity::new("BOE", "NE135FBM-N41", "0"),
            mode: Some(Mode {
                width: 2256,
                height: 1504,
                refresh: 59_999,
            }),
            scale: 1.5,
            position: (-1504, 120),
            transform: Transform::Rotate90,
            enabled: true,
        }
    }

    #[test]
    fn what_is_written_reads_back() {
        let mut config = Config::default();
        config.put(laptop());
        let mut other = Screen::new(Identity::new("DEL", "U2720Q", "ABC 123"));
        other.enabled = false;
        config.put(other);
        let (read, problems) = Config::parse_lenient(&config.to_text());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(read, config);
        assert_eq!(read.get(&laptop().identity), Some(&laptop()));
    }

    #[test]
    fn a_screen_saved_again_moves_last_and_the_oldest_go() {
        let mut config = Config::default();
        for i in 0..MAX_SCREENS + 3 {
            config.put(Screen::new(Identity::new("M", &i.to_string(), "")));
        }
        assert_eq!(config.screens.len(), MAX_SCREENS);
        assert_eq!(config.screens[0].identity.model, "3");
        config.put(Screen::new(Identity::new("M", "3", "")));
        assert_eq!(config.screens.last().unwrap().identity.model, "3");
        assert_eq!(config.screens.len(), MAX_SCREENS);
    }

    #[test]
    fn wrong_lines_are_left_aside_and_the_rest_counts() {
        let text = "scale = 2\n[screen]\nmake = A\nmodel = B\nscale = 9\nposition = 1\n\
                    mode = 0x0@60\ntransform = sideways\nenabled = yes\ncolour = red\n\
                    position = 10,20\n[screen]\nscale = 2\n";
        let (config, problems) = Config::parse_lenient(text);
        assert_eq!(problems.len(), 7, "{problems:?}");
        // The second group has no name: left out.
        assert_eq!(config.screens.len(), 1);
        let screen = &config.screens[0];
        assert_eq!((screen.scale, screen.position), (1.0, (10, 20)));
        assert_eq!(screen.mode, None);
    }

    #[test]
    fn names_are_bounded_and_printable() {
        let long = "x".repeat(200);
        let identity = Identity::new(&long, "a\nb\u{7}", " s ");
        assert_eq!(identity.make.len(), MAX_NAME);
        assert_eq!(
            (identity.model.as_str(), identity.serial.as_str()),
            ("ab", "s")
        );
    }

    #[test]
    fn a_mode_without_refresh_and_transforms_by_code() {
        let mode = Mode::parse("1920x1080").unwrap();
        assert_eq!(mode.refresh, 0);
        assert_eq!(Mode::parse("1920x1080@143.981").unwrap().refresh, 143_981);
        assert!(Mode::parse("99999x10@60").is_none());
        for t in Transform::ALL {
            assert_eq!(Transform::from_code(t.code()), Some(t));
        }
        assert_eq!(Transform::from_code(8), None);
    }

    #[test]
    fn a_missing_file_is_nothing_saved_and_a_huge_one_is_refused() {
        let dir = std::env::temp_dir().join(format!("displays-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (config, _) = Config::read(&dir.join("none.conf")).unwrap();
        assert!(config.screens.is_empty());
        let huge = dir.join("huge.conf");
        fs::write(&huge, "#".repeat(MAX_BYTES as usize + 1)).unwrap();
        assert!(matches!(Config::read(&huge), Err(Error::TooLarge { .. })));
        fs::remove_dir_all(&dir).unwrap();
    }
}
