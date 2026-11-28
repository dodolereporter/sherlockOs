//! The keyboard setting, `<config dir>/keyboard.conf`: the layout and how a held key repeats
//! (lot D-26c). The compositor reads it at startup and again when it changes; Control Room's
//! keyboard page writes it.
//!
//! ```text
//! # Every key is optional. Without a layout, the system's (or French AZERTY).
//! # An xkb layout and its variant: fr, us, de, ch with fr…
//! layout = fr
//! variant =
//! # Delay before a held key repeats, from 150 to 1000 milliseconds.
//! repeat-delay = 200
//! # Repeats per second while the key is held, from 2 to 50.
//! repeat-rate = 25
//! ```

use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
};

/// Name of the settings file, in the desktop's configuration folder.
pub const FILE: &str = "keyboard.conf";

/// Delay before a held key repeats, in milliseconds: default and bounds.
pub const DEFAULT_REPEAT_DELAY: u32 = 200;
pub const REPEAT_DELAY_RANGE: (u32, u32) = (150, 1000);
/// Repeats per second while a key is held: default and bounds.
pub const DEFAULT_REPEAT_RATE: u32 = 25;
pub const REPEAT_RATE_RANGE: (u32, u32) = (2, 50);

/// Longest layout or variant name taken: xkb's are a few letters.
const NAME_MAX: usize = 64;

/// What can go wrong when reading the settings.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("line {line}: {reason}")]
    Line { line: usize, reason: String },
}

/// The keyboard setting. By default, no layout (the system's) and the repeat the compositor
/// always had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// xkb layout, such as `fr` or `us,fr`; `None` leaves it to the system.
    pub layout: Option<String>,
    /// xkb variant of the layout, such as `bepo`; empty for the basic one.
    pub variant: String,
    /// Milliseconds before a held key repeats.
    pub repeat_delay: u32,
    /// Repeats per second.
    pub repeat_rate: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            layout: None,
            variant: String::new(),
            repeat_delay: DEFAULT_REPEAT_DELAY,
            repeat_rate: DEFAULT_REPEAT_RATE,
        }
    }
}

/// An xkb name: lower case letters, digits, `_` and `-`, and `,` between the layouts of a list.
/// Nothing that could reach xkbcommon's file lookup as a path.
fn valid_name(value: &str, list: bool) -> bool {
    value.len() <= NAME_MAX
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || (list && c == ','))
        && !value.starts_with(',')
        && !value.ends_with(',')
}

impl Config {
    /// Reads `key = value` lines; `#` starts a comment line. Unknown keys and values, and
    /// numbers out of their range, are refused: the error of the first one is returned.
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
        let number = |(low, high): (u32, u32)| {
            value
                .parse::<u32>()
                .ok()
                .filter(|n| (low..=high).contains(n))
                .ok_or_else(|| format!("expected a whole number from {low} to {high}"))
        };
        match key {
            "layout" if valid_name(value, true) => {
                self.layout = (!value.is_empty()).then(|| value.to_owned());
            }
            "variant" if valid_name(value, true) => self.variant = value.to_owned(),
            "layout" | "variant" => return Err(format!("not an xkb name: {value:?}")),
            "repeat-delay" => self.repeat_delay = number(REPEAT_DELAY_RANGE)?,
            "repeat-rate" => self.repeat_rate = number(REPEAT_RATE_RANGE)?,
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

    fn values(&self) -> [(&'static str, String); 4] {
        [
            ("layout", self.layout.clone().unwrap_or_default()),
            ("variant", self.variant.clone()),
            ("repeat-delay", self.repeat_delay.to_string()),
            ("repeat-rate", self.repeat_rate.to_string()),
        ]
    }

    /// `text`, the file as it is, with the keys whose value differs from what it says set to
    /// this setting's: comments, unknown keys and the order stay.
    pub fn write_into(&self, text: &str) -> String {
        let (read, _) = Self::parse_lenient(text);
        crate::conf::update(text, &self.values(), &read.values())
    }
}

/// `<config dir>/keyboard.conf`.
pub fn config_file(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    crate::config_dir(var).map(|dir| dir.join(FILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_is_the_default() {
        assert_eq!(Config::parse("# nothing\n").unwrap(), Config::default());
        assert_eq!(Config::default().write_into(""), "");
        // An empty layout is the system's.
        assert_eq!(Config::parse("layout =\n").unwrap().layout, None);
    }

    #[test]
    fn every_key_is_read_and_written_back() {
        let text = "layout = fr\nvariant = bepo\nrepeat-delay = 500\nrepeat-rate = 30\n";
        let config = Config::parse(text).unwrap();
        let expected = Config {
            layout: Some("fr".into()),
            variant: "bepo".into(),
            repeat_delay: 500,
            repeat_rate: 30,
        };
        assert_eq!(config, expected);
        assert_eq!(Config::parse(&expected.write_into("")).unwrap(), expected);
        assert!(Config::parse("layout = us,fr\n").is_ok());
    }

    #[test]
    fn mistakes_and_strange_names_are_refused() {
        for (text, line) in [
            ("repeat-delay = 100", 1),
            ("repeat-delay = 1001", 1),
            ("\nrepeat-rate = 0", 2),
            ("repeat-rate = fast", 1),
            ("layout = ../../etc/passwd", 1),
            ("layout = fr bepo", 1),
            ("variant = x/y", 1),
            ("layout = ,fr", 1),
            ("model = pc105", 1),
        ] {
            match Config::parse(text) {
                Err(Error::Line { line: at, .. }) => assert_eq!(at, line, "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
        let long = format!("layout = {}", "a".repeat(NAME_MAX + 1));
        assert!(Config::parse(&long).is_err());
    }

    #[test]
    fn one_key_changes_and_the_rest_stays() {
        let text = "# mine\nlayout = de\nfuture = 1\n";
        let (mut config, problems) = Config::parse_lenient(text);
        assert_eq!(problems.len(), 1);
        config.repeat_rate = 40;
        assert_eq!(
            config.write_into(text),
            "# mine\nlayout = de\nfuture = 1\nrepeat-rate = 40\n"
        );
        let env = |key: &str| (key == "HOME").then(|| OsString::from("/home/u"));
        assert_eq!(
            config_file(env),
            Some("/home/u/.config/sherlockos/keyboard.conf".into())
        );
    }
}
