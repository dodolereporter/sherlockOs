//! The appearance settings file, `$XDG_CONFIG_HOME/appearance/appearance.conf`: the theme, and
//! the hours of the automatic mode. Every program reads it the same way and follows it live
//! (`follow`); the settings application (Control Room, D-26a) writes it (`write_into`). Depends on the
//! tokens of sherlock-ui, on `libc` and on `tracing`.
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

pub use sherlock_ui::color::Theme;
pub use watch::{Event, follow, follow_themes};

const GROUP: &str = "Appearance";
const THEME: &str = "Theme";
const LIGHT_FROM: &str = "AutoLightFrom";
const DARK_FROM: &str = "AutoDarkFrom";

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

    /// Minutes since midnight, the seconds dropped: what the file and Control Room show.
    pub const fn minutes(self) -> u32 {
        self.0 / 60
    }

    /// Reads `HH:MM` or `HH:MM:SS`.
    pub fn parse(text: &str) -> Option<Self> {
        // Digits only: `u32::from_str` would also take a sign.
        let digits = |f: &str| (1..=2).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit());
        let mut fields = text
            .trim()
            .split(':')
            .map(|f| f.parse::<u32>().ok().filter(|_| digits(f)));
        let (hour, minute) = (fields.next()??, fields.next()??);
        let second = fields.next().unwrap_or(Some(0))?;
        if fields.next().is_some() {
            return None;
        }
        Self::new(hour, minute, second)
    }

    /// The local time of day at `epoch` (seconds since 1970), in the time zone of the system
    /// as it is now.
    pub fn at(epoch: i64) -> io::Result<Self> {
        let tm = local(epoch)?;
        let field = |v: libc::c_int| u32::try_from(v).unwrap_or(0);
        Self::new(
            field(tm.tm_hour),
            field(tm.tm_min),
            field(tm.tm_sec).min(59),
        )
        .ok_or_else(|| io::Error::other("local time out of a day"))
    }

    /// The first instant after `epoch` whose local time is `self`, in seconds since 1970.
    /// `mktime` places it in the right day across a change of daylight saving time.
    pub fn next_after(self, epoch: i64) -> io::Result<i64> {
        let mut day = local(epoch)?;
        for _ in 0..2 {
            let mut tm = day;
            (tm.tm_hour, tm.tm_min, tm.tm_sec) = (
                (self.0 / 3600) as libc::c_int,
                (self.0 / 60 % 60) as libc::c_int,
                (self.0 % 60) as libc::c_int,
            );
            // Unknown: `mktime` finds whether daylight saving time applies at that hour.
            tm.tm_isdst = -1;
            // `time_t` is 64 bits on the target, x86-64 (ADR 0001).
            // SAFETY: `tm` is a valid, initialized `struct tm` that we own.
            let instant: i64 = unsafe { libc::mktime(&mut tm) };
            if instant == -1 {
                return Err(io::Error::other("mktime cannot place the local time"));
            }
            if instant > epoch {
                return Ok(instant);
            }
            // Already passed today: the same time tomorrow, `mktime` normalizes the day.
            day.tm_mday += 1;
        }
        Err(io::Error::other("no next local time"))
    }
}

/// Seconds since 1970, from the real-time clock.
pub fn epoch_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Broken-down local time of `epoch`, in seconds since 1970. The time zone is read again
/// first: glibc's `mktime` runs `tzset` (POSIX requires it), which notices a new
/// `/etc/localtime`; `localtime_r` alone would keep the zone it read first. The top bar reads
/// its clock with it too.
pub fn local(epoch: i64) -> io::Result<libc::tm> {
    let time = libc::time_t::try_from(epoch).map_err(io::Error::other)?;
    // SAFETY: `tm` is plain data; zeroed, then filled by the calls below.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_mday = 1;
    tm.tm_year = 70;
    // SAFETY: `tm` is a valid `struct tm` that we own; the result is not needed.
    unsafe { libc::mktime(&mut tm) };
    // SAFETY: both pointers are valid for the call: `time` is read, `tm` written.
    if unsafe { libc::localtime_r(&time, &mut tm) }.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(tm)
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
        match mime_apps::read_bounded(path, mime_apps::MAX_SETTINGS_SIZE) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok((Self::default(), Vec::new()))
            }
            Err(error) => Err(error),
        }
    }

    /// `text`, the file as it is, with the keys whose value differs from what it says set to
    /// these settings': the line of the key in `[Appearance]` is replaced, or the key is added
    /// at the end of the group (the group at the end of the file without one). Comments, other
    /// groups, unknown keys and the order stay. Control Room writes the file so.
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse(text);
        let values = |c: &Self| {
            [
                (THEME, c.mode.key().to_owned()),
                (LIGHT_FROM, c.light_from.to_string()),
                (DARK_FROM, c.dark_from.to_string()),
            ]
        };
        let mut text = text.to_owned();
        for ((key, value), (_, before)) in values(self).into_iter().zip(values(&read)) {
            if value != before {
                text = set(&text, key, &value);
            }
        }
        text
    }

    /// The theme at `now`, and the time of day of the next switch of the automatic mode;
    /// `None` when it never switches by itself.
    pub fn theme_at(&self, now: TimeOfDay) -> (Theme, Option<TimeOfDay>) {
        let (light, dark, now) = (self.light_from.0, self.dark_from.0, now.0);
        let is_light = if light < dark {
            (light..dark).contains(&now)
        } else {
            now >= light || now < dark
        };
        let (theme, next) = match (self.mode, is_light) {
            (Mode::Dark, _) => return (Theme::Dark, None),
            (Mode::Light, _) => return (Theme::Light, None),
            (Mode::Auto, true) => (Theme::Light, self.dark_from),
            (Mode::Auto, false) => (Theme::Dark, self.light_from),
        };
        (theme, Some(next))
    }
}

/// `text` with `key=value` in the `[Appearance]` group: its last line there replaced, or a
/// line added at the end of the group, after its last line that is not blank.
fn set(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}={value}");
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut group = "";
    let (mut found, mut end) = (None, None);
    for (i, raw) in text.lines().enumerate() {
        let trimmed = raw.trim();
        if let Some(name) = trimmed.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name;
            continue;
        }
        if group != GROUP {
            continue;
        }
        if !trimmed.is_empty() {
            end = Some(i);
        }
        let is_key = trimmed
            .split_once('=')
            .is_some_and(|(k, _)| !trimmed.starts_with('#') && k.trim() == key);
        if is_key {
            found = Some(i);
        }
    }
    let header = lines.iter().position(|l| l.trim() == format!("[{GROUP}]"));
    match (found, end, header) {
        (Some(at), _, _) => lines[at] = line,
        (None, Some(at), _) => lines.insert(at + 1, line),
        (None, None, Some(at)) => lines.insert(at + 1, line),
        (None, None, None) => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("[{GROUP}]"));
            lines.push(line);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
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

/// The theme of the file at `path` now, read once: a program makes it active before its first
/// frame, then follows the file (`follow_themes`), so that a light session never shows a dark
/// frame at start. What cannot be read gives the default and goes to the log.
pub fn theme_now(path: &Path) -> Theme {
    let config = match AppearanceConfig::load(path) {
        Ok((config, problems)) => {
            for problem in problems {
                tracing::warn!(%problem, "appearance file");
            }
            config
        }
        Err(error) => {
            tracing::warn!(%error, path = %path.display(), "cannot read the appearance file");
            AppearanceConfig::default()
        }
    };
    match TimeOfDay::at(epoch_now()) {
        Ok(now) => config.theme_at(now).0,
        Err(error) => {
            tracing::warn!(%error, "no local time: the default theme");
            Theme::default()
        }
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
    fn what_control_room_writes_reads_back() {
        let config = AppearanceConfig {
            mode: Mode::Auto,
            light_from: at(6, 30, 0),
            dark_from: at(20, 0, 0),
        };
        let (read, problems) = AppearanceConfig::parse(&config.write_into(""));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(read, config);
        assert_eq!(config.light_from.minutes(), 6 * 60 + 30);
        assert_eq!(AppearanceConfig::default().write_into(""), "");
    }

    #[test]
    fn control_room_changes_one_key_and_keeps_the_rest() {
        let text = "# mine\n[Other]\nA=1\n\n[Appearance]\n# my hours\nTheme=dark\nFuture=x\n\n[Last]\nB=2\n";
        let (mut config, _) = AppearanceConfig::parse(text);
        config.mode = Mode::Light;
        assert_eq!(
            config.write_into(text),
            "# mine\n[Other]\nA=1\n\n[Appearance]\n# my hours\nTheme=light\nFuture=x\n\n[Last]\nB=2\n"
        );
        // A key not there yet goes at the end of its group.
        config.light_from = at(8, 0, 0);
        assert_eq!(
            config.write_into(text),
            "# mine\n[Other]\nA=1\n\n[Appearance]\n# my hours\nTheme=light\nFuture=x\nAutoLightFrom=08:00\n\n[Last]\nB=2\n"
        );
        // Without the group, it is added at the end.
        assert_eq!(
            config.write_into("[Other]\nA=1\n"),
            "[Other]\nA=1\n\n[Appearance]\nTheme=light\nAutoLightFrom=08:00\n"
        );
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
        let (seven, nineteen) = (Some(at(7, 0, 0)), Some(at(19, 0, 0)));
        assert_eq!(config.theme_at(at(7, 0, 0)), (Theme::Light, nineteen));
        assert_eq!(config.theme_at(at(6, 59, 59)), (Theme::Dark, seven));
        assert_eq!(config.theme_at(at(19, 0, 0)), (Theme::Dark, seven));
        assert_eq!(config.theme_at(at(18, 59, 59)), (Theme::Light, nineteen));
    }

    #[test]
    fn the_automatic_mode_crosses_midnight() {
        let config = auto("07:00", "19:00");
        let seven = Some(at(7, 0, 0));
        assert_eq!(config.theme_at(at(23, 59, 59)), (Theme::Dark, seven));
        assert_eq!(config.theme_at(at(0, 0, 0)), (Theme::Dark, seven));
        // A light night, for whoever works at night: the light span holds midnight.
        let night = auto("22:00", "06:00");
        let (six, ten) = (Some(at(6, 0, 0)), Some(at(22, 0, 0)));
        assert_eq!(night.theme_at(at(21, 59, 59)), (Theme::Dark, ten));
        assert_eq!(night.theme_at(at(22, 0, 0)), (Theme::Light, six));
        assert_eq!(night.theme_at(at(0, 0, 0)), (Theme::Light, six));
        assert_eq!(night.theme_at(at(5, 59, 59)), (Theme::Light, six));
        assert_eq!(night.theme_at(at(6, 0, 0)), (Theme::Dark, ten));
    }

    #[test]
    fn times_of_day_are_read_strictly() {
        assert_eq!(TimeOfDay::parse("07:05"), Some(at(7, 5, 0)));
        assert_eq!(TimeOfDay::parse("7:05"), Some(at(7, 5, 0)));
        assert_eq!(TimeOfDay::parse(" 23:59:59\n"), Some(at(23, 59, 59)));
        for bad in [
            "24:00",
            "7",
            "07:60",
            "07:00:00:00",
            "",
            "aa:bb",
            "+7:00",
            "07:+5",
            "007:00",
        ] {
            assert_eq!(TimeOfDay::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_theme_of_a_file_is_read_before_the_first_frame() {
        let dir = std::env::temp_dir().join(format!("appearance-now-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("appearance.conf");
        std::fs::write(&file, "[Appearance]\nTheme=light\n").unwrap();
        assert_eq!(theme_now(&file), Theme::Light);
        std::fs::remove_file(&file).unwrap();
        assert_eq!(theme_now(&file), Theme::Dark, "no file: the default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_next_switch_is_an_instant_of_the_local_clock() {
        let now = epoch_now();
        let local = TimeOfDay::at(now).unwrap();
        // The same time of day: tomorrow, about a day later.
        let next = local.next_after(now).unwrap();
        assert!(
            (now + 22 * 3600..=now + 26 * 3600).contains(&next),
            "{now} {next}"
        );
        // One minute later, unless midnight falls in between: today.
        if local.0 < 24 * 3600 - 60 {
            let soon = TimeOfDay(local.0 + 60).next_after(now).unwrap();
            assert!((now + 59..=now + 61).contains(&soon), "{now} {soon}");
        }
    }

    /// Runs `test` in a new process of this test binary, with `TZ=Europe/Paris`: changing the
    /// environment of the running process would race with the other tests.
    fn in_paris(test: &str) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--ignored"])
            .env("TZ", "Europe/Paris")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{test} with TZ=Europe/Paris:\n{text}"
        );
        // A misspelt name would run nothing and succeed.
        assert!(text.contains("1 passed"), "{text}");
    }

    #[test]
    fn the_next_switch_follows_daylight_saving_time() {
        in_paris("tests::daylight_saving_time_in_paris");
    }

    #[test]
    #[ignore = "run by the_next_switch_follows_daylight_saving_time, with TZ=Europe/Paris"]
    fn daylight_saving_time_in_paris() {
        // Run straight, by `--include-ignored`, it would read the zone of the machine.
        if std::env::var("TZ").as_deref() != Ok("Europe/Paris") {
            println!("skipped: needs TZ=Europe/Paris, which its parent test gives");
            return;
        }
        let seven = at(7, 0, 0);
        // Saturday 28 March 2026, 20:00 in Paris (19:00 UTC). The clocks go forward that
        // night: 07:00 comes 10 hours later, at 05:00 UTC.
        let evening = 1_774_724_400;
        assert_eq!(TimeOfDay::at(evening).unwrap(), at(20, 0, 0));
        assert_eq!(seven.next_after(evening).unwrap(), 1_774_760_400);
        // Saturday 24 October 2026, 20:00 in Paris (18:00 UTC). The clocks go back that night:
        // 07:00 comes 12 hours later, at 06:00 UTC.
        let evening = 1_792_864_800;
        assert_eq!(TimeOfDay::at(evening).unwrap(), at(20, 0, 0));
        assert_eq!(seven.next_after(evening).unwrap(), 1_792_908_000);
    }
}
