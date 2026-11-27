//! The touchpad setting, `<config dir>/touchpad.conf`: what libinput does with the touchpad
//! (tap, scrolling, clicks, speed) and what the three finger swipe down does. The compositor
//! reads it again when it changes; Control Room's touchpad page writes it.
//!
//! ```text
//! # Every key is optional; these are the defaults.
//! tap-to-click = true
//! # Tap, then slide with the finger down: drags.
//! tap-and-drag = true
//! # The drag goes on a moment after the finger is lifted.
//! drag-lock = false
//! # Two finger scrolling moves the content with the fingers.
//! natural-scroll = true
//! # The touchpad is off while the keyboard is being typed on.
//! disable-while-typing = true
//! # Pointer speed, from -1 (slowest) to 1 (fastest).
//! speed = 0
//! # adaptive (faster movements go further) or flat.
//! acceleration = adaptive
//! # fingers (one finger clicks, two right click, three middle click) or areas (the bottom
//! # right corner right clicks).
//! click = fingers
//! # Three fingers down out of Mission Control: nothing, or minimize the active window.
//! swipe-down = nothing
//! ```

use std::{
    ffi::OsString,
    fmt, fs, io,
    path::{Path, PathBuf},
    str::FromStr,
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "touchpad.conf";

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// How the pointer speeds up with the movement of the finger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Acceleration {
    /// Faster movements take the pointer further.
    #[default]
    Adaptive,
    /// The pointer moves by the same factor at any speed.
    Flat,
}

/// How a press of the touchpad chooses its button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Click {
    /// By the number of fingers down: one left, two right, three middle (libinput's
    /// clickfinger).
    #[default]
    Fingers,
    /// By where the finger presses: the bottom right corner is the right button (libinput's
    /// button areas).
    Areas,
}

/// What a three finger swipe down does out of Mission Control, which it closes otherwise.
/// Nothing by default (lot D-33): a window should not go away by a gesture made by mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SwipeDown {
    /// Minimizes the window that has the keyboard.
    Minimize,
    #[default]
    Nothing,
}

/// Lowest and highest pointer speed, libinput's range.
pub const SPEED_RANGE: (f64, f64) = (-1.0, 1.0);

/// The touchpad setting. By default, the behavior of the reference: tap to click, natural
/// scrolling, clicks by the number of fingers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    pub tap_to_click: bool,
    pub tap_and_drag: bool,
    pub drag_lock: bool,
    pub natural_scroll: bool,
    pub disable_while_typing: bool,
    /// From -1 to 1.
    pub speed: f64,
    pub acceleration: Acceleration,
    pub click: Click,
    pub swipe_down: SwipeDown,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tap_to_click: true,
            tap_and_drag: true,
            drag_lock: false,
            natural_scroll: true,
            disable_while_typing: true,
            speed: 0.0,
            acceleration: Acceleration::default(),
            click: Click::default(),
            swipe_down: SwipeDown::default(),
        }
    }
}

/// Value names of an enumeration, in both directions.
macro_rules! names {
    ($type:ty, $($variant:ident = $name:literal),+) => {
        impl $type {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
        }

        impl FromStr for $type {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, String> {
                match s {
                    $($name => Ok(Self::$variant),)+
                    _ => Err(format!("unknown value {s:?}, expected {}", [$($name),+].join(" or "))),
                }
            }
        }

        impl fmt::Display for $type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

names!(Acceleration, Adaptive = "adaptive", Flat = "flat");
names!(Click, Fingers = "fingers", Areas = "areas");
names!(SwipeDown, Minimize = "minimize", Nothing = "nothing");

impl Config {
    /// Reads `key = value` lines; `#` starts a comment line. Unknown keys and values, and a
    /// speed out of its range, are refused, so that a typo does not go unnoticed: the error of
    /// the first one is returned.
    pub fn parse(text: &str) -> Result<Self, Error> {
        let (config, mut problems) = Self::parse_lenient(text);
        match problems.is_empty() {
            true => Ok(config),
            false => Err(problems.remove(0)),
        }
    }

    /// Reads what can be read: each line that cannot is left at its default and listed.
    /// Control Room shows the file so, and changes it without losing the rest.
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

    /// Takes the value of one key.
    fn take(&mut self, key: &str, value: &str) -> Result<(), String> {
        let flag = || {
            value
                .parse::<bool>()
                .map_err(|_| format!("expected true or false, not {value:?}"))
        };
        match key {
            "tap-to-click" => self.tap_to_click = flag()?,
            "tap-and-drag" => self.tap_and_drag = flag()?,
            "drag-lock" => self.drag_lock = flag()?,
            "natural-scroll" => self.natural_scroll = flag()?,
            "disable-while-typing" => self.disable_while_typing = flag()?,
            "speed" => {
                let (low, high) = SPEED_RANGE;
                self.speed = value
                    .parse::<f64>()
                    .ok()
                    .filter(|s| (low..=high).contains(s))
                    .ok_or_else(|| format!("expected a number from -1 to 1, not {value:?}"))?;
            }
            "acceleration" => self.acceleration = value.parse()?,
            "click" => self.click = value.parse()?,
            "swipe-down" => self.swipe_down = value.parse()?,
            key => return Err(format!("unknown key {key:?}")),
        }
        Ok(())
    }

    /// Reads the settings file. A missing file is the default setting.
    pub fn read(path: &Path) -> Result<Self, Error> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Read {
                path: path.into(),
                source,
            }),
        }
    }

    /// Each key and its value as written in the file.
    fn values(&self) -> [(&'static str, String); 9] {
        [
            ("tap-to-click", self.tap_to_click.to_string()),
            ("tap-and-drag", self.tap_and_drag.to_string()),
            ("drag-lock", self.drag_lock.to_string()),
            ("natural-scroll", self.natural_scroll.to_string()),
            (
                "disable-while-typing",
                self.disable_while_typing.to_string(),
            ),
            ("speed", self.speed.to_string()),
            ("acceleration", self.acceleration.to_string()),
            ("click", self.click.to_string()),
            ("swipe-down", self.swipe_down.to_string()),
        ]
    }

    /// `text`, the file as it is, with the keys whose value differs from what it says set to
    /// this setting's: comments, unknown keys, lines it gets wrong and the order stay. Control
    /// Room writes the file so.
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse_lenient(text);
        crate::conf::update(text, &self.values(), &read.values())
    }
}

/// `<config dir>/touchpad.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_the_default() {
        let config = Config::parse("# nothing\n\n").unwrap();
        assert_eq!(config, Config::default());
        assert!(config.tap_to_click && config.natural_scroll && config.disable_while_typing);
        assert_eq!(config.click, Click::Fingers);
        assert_eq!(config.swipe_down, SwipeDown::Nothing);
    }

    #[test]
    fn every_key_is_read() {
        let text = "tap-to-click = false\ntap-and-drag=false\ndrag-lock = true\n\
                    natural-scroll = false\ndisable-while-typing = false\nspeed = -0.5\n\
                    acceleration = flat\nclick = areas\nswipe-down = minimize\n";
        let config = Config::parse(text).unwrap();
        let expected = Config {
            tap_to_click: false,
            tap_and_drag: false,
            drag_lock: true,
            natural_scroll: false,
            disable_while_typing: false,
            speed: -0.5,
            acceleration: Acceleration::Flat,
            click: Click::Areas,
            swipe_down: SwipeDown::Minimize,
        };
        assert_eq!(config, expected);
        // What Control Room writes reads back the same.
        assert_eq!(Config::parse(&expected.write_into("")).unwrap(), expected);
        assert_eq!(Config::default().write_into(""), "");
    }

    #[test]
    fn mistakes_are_refused_with_their_line() {
        for (text, line) in [
            ("speed = 2", 1),
            ("\nspeed = fast", 2),
            ("natural-scroll = yes", 1),
            ("click = middle", 1),
            ("swipe-down = close", 1),
            ("tap = true", 1),
            ("tap-to-click", 1),
        ] {
            match Config::parse(text) {
                Err(Error::Line { line: at, .. }) => assert_eq!(at, line, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn control_room_changes_one_key_and_keeps_the_rest() {
        let text = "# my touchpad\nspeed = 0.4\nclick = middle\n\n# kept\nfuture = 1\n";
        let (mut config, problems) = Config::parse_lenient(text);
        // The lines it gets wrong are listed; the others are read.
        assert_eq!(problems.len(), 2);
        assert_eq!(config.speed, 0.4);
        config.tap_to_click = false;
        assert_eq!(
            config.write_into(text),
            "# my touchpad\nspeed = 0.4\nclick = middle\n\n# kept\nfuture = 1\ntap-to-click = false\n"
        );
        config.speed = 0.5;
        let written = config.write_into(text);
        assert!(
            written.contains("speed = 0.5\nclick = middle\n"),
            "{written}"
        );
    }

    #[test]
    fn a_missing_file_is_the_default() {
        let path = Path::new("/nonexistent/sherlockos/touchpad.conf");
        assert_eq!(Config::read(path).unwrap(), Config::default());
        let env = |key: &str| (key == "HOME").then(|| OsString::from("/home/u"));
        assert_eq!(
            config_file(env),
            Some("/home/u/.config/sherlockos/touchpad.conf".into())
        );
    }
}
