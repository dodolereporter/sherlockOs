//! The appearance settings file, `$XDG_CONFIG_HOME/appearance/appearance.conf`: the theme, and
//! the hours of the automatic mode. Every program reads it the same way and follows it live
//! (`follow`); the settings application (Control Room, D-26) will write it. Standard library
//! only, besides the tokens of sherlock-ui.
//!
//! ```text
//! [Appearance]
//! Theme=auto
//! AutoLightFrom=07:00
//! AutoDarkFrom=19:00
//! ```
//!
//! Every key is optional. A value that cannot be read gives the default and a `Problem` that
//! the caller logs: a bad file never stops a program. See `docs/appearance.md`.

mod watch;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use sherlock_ui::color::Theme;
pub use watch::{Event, follow};

const GROUP: &str = "Appearance";
const THEME: &str = "Theme";
const LIGHT_FROM: &str = "AutoLightFrom";
const DARK_FROM: &str = "AutoDarkFrom";

const DAY: u32 = 24 * 60 * 60;

/// The theme chosen in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
    /// Light from `light_from`, dark from `dark_from`.
    Auto,
}

impl Mode {
    pub const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::Auto];

    /// The word written in the file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Auto => "auto",
        }
    }
}

/// A local time of day, to the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(u32);

impl TimeOfDay {
    /// `None` outside a day.
    pub const fn new(hour: u32, minute: u32, second: u32) -> Option<Self> {
        if hour < 24 && minute < 60 && second < 60 {
            Some(Self(hour * 3600 + minute * 60 + second))
        } else {
            None
        }
    }

    /// Reads `HH:MM` or `HH:MM:SS`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut fields = text.trim().split(':').map(|f| f.parse::<u32>().ok());
        let (hour, minute) = (fields.next()??, fields.next()??);
        let second = fields.next().unwrap_or(Some(0))?;
        if fields.next().is_some() {
            return None;
        }
        Self::new(hour, minute, second)
    }

    /// The time of day now, from the `date` command: the standard library does not know the
    /// time zone.
    pub fn now() -> io::Result<Self> {
        let output = std::process::Command::new("date")
            .arg("+%H:%M:%S")
            .output()?;
        let text = String::from_utf8_lossy(&output.stdout);
        Self::parse(&text).ok_or_else(|| io::Error::other(format!("date gave « {text} »")))
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.0 / 3600, self.0 / 60 % 60)
    }
}

/// A value of the file that was not taken: the default is used instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub key: &'static str,
    pub value: String,
    pub expected: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { key, value, .. } = self;
        write!(f, "{key}={value} ignored, expected {}", self.expected)
    }
}

/// The appearance settings, each with its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceConfig {
    pub mode: Mode,
    pub light_from: TimeOfDay,
    pub dark_from: TimeOfDay,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Dark,
            light_from: TimeOfDay(7 * 3600),
            dark_from: TimeOfDay(19 * 3600),
        }
    }
}

impl AppearanceConfig {
    /// Reads `text`, the content of the file. Values that cannot be taken are listed.
    pub fn parse(text: &str) -> (Self, Vec<Problem>) {
        let mut config = Self::default();
        let mut problems = Vec::new();
        let mut group = "";
        for line in text.lines().map(str::trim) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                group = name;
                continue;
            }
            let Some((key, value)) = line.split_once('=').filter(|_| group == GROUP) else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                THEME => match Mode::ALL.into_iter().find(|m| m.key() == value) {
                    Some(mode) => config.mode = mode,
                    None => problems.push(problem(THEME, value, "one of dark, light, auto")),
                },
                LIGHT_FROM => read_time(LIGHT_FROM, value, &mut config.light_from, &mut problems),
                DARK_FROM => read_time(DARK_FROM, value, &mut config.dark_from, &mut problems),
                _ => {}
            }
        }
        if config.light_from == config.dark_from {
            let defaults = Self::default();
            let value = config.dark_from.to_string();
            problems.push(problem(
                DARK_FROM,
                &value,
                "a time other than AutoLightFrom",
            ));
            (config.light_from, config.dark_from) = (defaults.light_from, defaults.dark_from);
        }
        (config, problems)
    }

    /// Reads the file at `path`; an absent file is the default configuration.
    pub fn load(path: &Path) -> io::Result<(Self, Vec<Problem>)> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok((Self::default(), Vec::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// The theme at `now`, and how long it holds before the automatic mode switches; `None`
    /// when it never switches by itself.
    pub fn theme_at(&self, now: TimeOfDay) -> (Theme, Option<Duration>) {
        let (light, dark, now) = (self.light_from.0, self.dark_from.0, now.0);
        let is_light = if light < dark {
            (light..dark).contains(&now)
        } else {
            now >= light || now < dark
        };
        let (theme, next) = match (self.mode, is_light) {
            (Mode::Dark, _) => return (Theme::Dark, None),
            (Mode::Light, _) => return (Theme::Light, None),
            (Mode::Auto, true) => (Theme::Light, dark),
            (Mode::Auto, false) => (Theme::Dark, light),
        };
        let wait = (next + DAY - now) % DAY;
        (theme, Some(Duration::from_secs(u64::from(wait))))
    }
}

fn problem(key: &'static str, value: &str, expected: &str) -> Problem {
    Problem {
        key,
        value: value.to_owned(),
        expected: expected.to_owned(),
    }
}

/// Sets `slot` to the time `value` of `key`, or leaves it and adds a problem.
fn read_time(key: &'static str, value: &str, slot: &mut TimeOfDay, problems: &mut Vec<Problem>) {
    match TimeOfDay::parse(value) {
        Some(time) => *slot = time,
        None => problems.push(problem(key, value, "a time such as 07:00")),
    }
}

/// Path of the file: `$XDG_CONFIG_HOME/appearance/appearance.conf`, `~/.config` by default.
pub fn path() -> Option<PathBuf> {
    let home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(home.join("appearance/appearance.conf"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(hour: u32, minute: u32, second: u32) -> TimeOfDay {
        TimeOfDay::new(hour, minute, second).unwrap()
    }

    fn auto(light: &str, dark: &str) -> AppearanceConfig {
        let text =
            format!("[Appearance]\nTheme=auto\nAutoLightFrom={light}\nAutoDarkFrom={dark}\n");
        let (config, problems) = AppearanceConfig::parse(&text);
        assert!(problems.is_empty(), "{problems:?}");
        config
    }

    #[test]
    fn every_setting_is_read() {
        let config = auto("06:30", "20:15");
        assert_eq!(config.mode, Mode::Auto);
        assert_eq!(
            (config.light_from, config.dark_from),
            (at(6, 30, 0), at(20, 15, 0))
        );
        assert_eq!(config.dark_from.to_string(), "20:15");
        let (config, problems) = AppearanceConfig::parse("[Appearance]\nTheme=light\n");
        assert!(problems.is_empty());
        assert_eq!(config.theme_at(at(3, 0, 0)), (Theme::Light, None));
    }

    #[test]
    fn without_a_file_the_theme_is_dark() {
        let (config, problems) = AppearanceConfig::parse("");
        assert!(problems.is_empty());
        assert_eq!(config, AppearanceConfig::default());
        assert_eq!(config.theme_at(at(12, 0, 0)), (Theme::Dark, None));
        let path = std::env::temp_dir().join(format!("appearance-absent-{}", std::process::id()));
        assert_eq!(AppearanceConfig::load(&path).unwrap().0, config);
    }

    #[test]
    fn invalid_values_give_the_default_and_a_problem() {
        let (config, problems) = AppearanceConfig::parse(
            "[Other]\nTheme=light\n[Appearance]\nTheme=sepia\nAutoLightFrom=25:00\nAutoDarkFrom=7h\n",
        );
        assert_eq!(config, AppearanceConfig::default());
        let keys: Vec<&str> = problems.iter().map(|p| p.key).collect();
        assert_eq!(keys, [THEME, LIGHT_FROM, DARK_FROM]);
        assert_eq!(
            problems[0].to_string(),
            "Theme=sepia ignored, expected one of dark, light, auto"
        );
        let (config, problems) =
            AppearanceConfig::parse("[Appearance]\nAutoLightFrom=08:00\nAutoDarkFrom=08:00\n");
        assert_eq!(config.light_from, AppearanceConfig::default().light_from);
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn the_automatic_mode_switches_at_its_bounds() {
        let config = auto("07:00", "19:00");
        let hours = |h: u64| Some(Duration::from_secs(h * 3600));
        assert_eq!(config.theme_at(at(7, 0, 0)), (Theme::Light, hours(12)));
        assert_eq!(
            config.theme_at(at(6, 59, 59)),
            (Theme::Dark, Some(Duration::from_secs(1)))
        );
        assert_eq!(config.theme_at(at(19, 0, 0)), (Theme::Dark, hours(12)));
        assert_eq!(
            config.theme_at(at(18, 59, 59)),
            (Theme::Light, Some(Duration::from_secs(1)))
        );
    }

    #[test]
    fn the_automatic_mode_crosses_midnight() {
        // Dark evenings: the wait runs over midnight.
        let config = auto("07:00", "19:00");
        assert_eq!(
            config.theme_at(at(23, 59, 59)),
            (Theme::Dark, Some(Duration::from_secs(7 * 3600 + 1)))
        );
        assert_eq!(
            config.theme_at(at(0, 0, 0)),
            (Theme::Dark, Some(Duration::from_secs(7 * 3600)))
        );
        // A light night, for whoever works at night: the light span holds midnight.
        let night = auto("22:00", "06:00");
        assert_eq!(
            night.theme_at(at(23, 0, 0)),
            (Theme::Light, Some(Duration::from_secs(7 * 3600)))
        );
        assert_eq!(
            night.theme_at(at(0, 0, 0)),
            (Theme::Light, Some(Duration::from_secs(6 * 3600)))
        );
        assert_eq!(
            night.theme_at(at(6, 0, 0)),
            (Theme::Dark, Some(Duration::from_secs(16 * 3600)))
        );
    }

    #[test]
    fn times_of_day_are_read_strictly() {
        assert_eq!(TimeOfDay::parse("07:05"), Some(at(7, 5, 0)));
        assert_eq!(TimeOfDay::parse(" 23:59:59\n"), Some(at(23, 59, 59)));
        for bad in ["24:00", "7", "07:60", "07:00:00:00", "", "aa:bb"] {
            assert_eq!(TimeOfDay::parse(bad), None, "{bad}");
        }
        assert!(TimeOfDay::now().is_ok());
    }
}
