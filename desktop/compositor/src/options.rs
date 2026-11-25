//! Command line of the compositor.
//!
//! `compositor [--nested [--outputs N]] [--measure] [--scale S]… [command [arguments…]]`.
//! Without `--nested`, it is the real session (lot D-14), started from a text console or by
//! the login screen.

use std::ffi::OsString;

use crate::Error;

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// In a window of the current session, rather than on the screens of the machine.
    pub nested: bool,
    /// Frame statistics in the log, and the session stops with the command.
    pub measure: bool,
    /// Scale of each screen, in order; missing ones repeat the last one given, or 1. In nested
    /// mode, one per screen of the window (`--outputs`).
    pub scales: Vec<f64>,
    /// Command launched once XWayland is ready.
    pub command: Vec<OsString>,
}

impl Options {
    /// Reads the arguments, program name left out.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, Error> {
        let mut command: Vec<OsString> = args.into_iter().collect();
        let nested = command.first().is_some_and(|a| a == "--nested");
        if nested {
            command.remove(0);
        }
        let (mut measure, mut screens, mut scales) = (false, None, Vec::new());
        loop {
            match command.first().and_then(|a| a.to_str()) {
                Some("--measure") => measure = true,
                Some("--outputs") if nested => screens = Some(option(&mut command, 1..=4)?),
                Some("--scale") => scales.push(option(&mut command, 0.5..=4.0)?),
                Some("--outputs" | "--nested") => return Err(Error::Usage),
                _ => break,
            }
            command.remove(0);
        }
        // In nested mode, as many scales as screens of the window.
        if nested {
            scales = (0..screens.unwrap_or(1))
                .map(|i| scale_of(&scales, i))
                .collect();
        }
        Ok(Self {
            nested,
            measure,
            scales,
            command,
        })
    }

    /// Scale of the screen number `i`, from 0.
    pub fn scale(&self, i: usize) -> f64 {
        scale_of(&self.scales, i)
    }
}

fn scale_of(scales: &[f64], i: usize) -> f64 {
    scales.get(i).or(scales.last()).copied().unwrap_or(1.0)
}

/// Value of the option in front of `args`, which is removed with it, the option itself being
/// left for the caller. A decimal comma is read as a point.
fn option<T: std::str::FromStr + PartialOrd>(
    args: &mut Vec<OsString>,
    range: std::ops::RangeInclusive<T>,
) -> Result<T, Error> {
    if args.len() < 2 {
        return Err(Error::Usage);
    }
    let value = args.remove(1);
    let value = value.to_str().map(|v| v.replace(',', "."));
    value
        .and_then(|v| v.parse().ok())
        .filter(|v| range.contains(v))
        .ok_or(Error::Usage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, Error> {
        Options::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn without_nested_it_is_the_real_session() {
        let options = parse(&[]).unwrap();
        assert!(!options.nested && !options.measure);
        assert!(options.command.is_empty());
        assert_eq!(options.scale(0), 1.0);

        let options = parse(&["--scale", "1,5", "sh", "-c", "bar & dock"]).unwrap();
        assert!(!options.nested);
        assert_eq!((options.scale(0), options.scale(3)), (1.5, 1.5));
        assert_eq!(options.command, ["sh", "-c", "bar & dock"]);
    }

    #[test]
    fn nested_options_are_kept() {
        let options = parse(&[
            "--nested",
            "--measure",
            "--outputs",
            "2",
            "--scale",
            "2",
            "konsole",
            "--scale",
        ])
        .unwrap();
        assert!(options.nested && options.measure);
        assert_eq!(options.scales, [2.0, 2.0]);
        // What follows the command belongs to it.
        assert_eq!(options.command, ["konsole", "--scale"]);
        assert_eq!(parse(&["--nested"]).unwrap().scales, [1.0]);
    }

    #[test]
    fn wrong_options_are_refused() {
        for args in [
            &["--outputs", "2"][..],
            &["--nested", "--outputs", "5"],
            &["--scale", "9"],
            &["--scale"],
            &["--measure", "--nested"],
        ] {
            assert!(matches!(parse(args), Err(Error::Usage)), "{args:?}");
        }
    }
}
