//! Command line of the compositor.
//!
//! `compositor [--nested [--outputs N] | --headless] [--xwayland] [--measure] [--scale S]…
//! [--workspaces N]
//! [--snap-gap PX] [--minimize-effect genie|scale|none] [--window-animations on|off]
//! [--greeter] [--] [command [arguments…]]`.
//! Without `--nested`, it is the real session (lot D-14), started from a text console or by
//! the login screen, never inside another graphical session. `--greeter` makes it the
//! compositor of the login screen (lot D-24, ADR 0026). `--` ends the options: what follows is
//! the command, even if it starts with `--`.

use std::ffi::OsString;

use crate::{Error, genie::Effect};

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// In a window of the current session, rather than on the screens of the machine.
    pub nested: bool,
    /// Without any screen: virtual screens, nothing drawn (lot D-36).
    pub headless: bool,
    /// Frame statistics in the log, and the session stops with the command.
    pub measure: bool,
    /// `--xwayland`: X11 applications in the real session too (see `Options::xwayland`).
    pub xwayland: bool,
    /// Scale of each screen, in order; missing ones repeat the last one given, or 1. In nested
    /// mode, one per screen of the window (`--outputs`).
    pub scales: Vec<f64>,
    /// Number of workspaces, from 1 to 9 (`--workspaces`); `None` keeps the default, 3.
    pub workspaces: Option<usize>,
    /// Margin around snapped windows, from 0 to 64 pixels (`--snap-gap`); `None` keeps the
    /// default, `SNAP_GAP` of sherlock-ui.
    pub snap_gap: Option<i32>,
    /// What minimizing a window shows (`--minimize-effect`); `None` keeps the default, the
    /// genie.
    pub minimize_effect: Option<Effect>,
    /// Opening and closing animations of the windows (`--window-animations on|off`); `None`
    /// keeps the default, on.
    pub window_animations: Option<bool>,
    /// The compositor of the login screen (`--greeter`, lot D-24, ADR 0026): one client shown
    /// fullscreen, no shortcut of the desktop, and the compositor stops with the command.
    pub greeter: bool,
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
        let (mut measure, mut xwayland, mut screens, mut scales) = (false, false, None, Vec::new());
        let (mut workspaces, mut snap_gap, mut minimize_effect) = (None, None, None);
        let (mut window_animations, mut headless, mut greeter) = (None, false, false);
        loop {
            match command.first().and_then(|a| a.to_str()) {
                Some("--") => {
                    command.remove(0);
                    break;
                }
                Some("--greeter") => greeter = true,
                Some("--headless") if !nested => headless = true,
                Some("--measure") => measure = true,
                Some("--xwayland") => xwayland = true,
                Some("--outputs") if nested => screens = Some(option(&mut command, 1..=4)?),
                Some("--scale") => scales.push(option(&mut command, 0.5..=4.0)?),
                Some("--workspaces") => workspaces = Some(option(&mut command, 1..=9)?),
                Some("--snap-gap") => snap_gap = Some(option(&mut command, 0..=64)?),
                Some("--minimize-effect") => minimize_effect = Some(named(&mut command)?),
                Some("--window-animations") => window_animations = Some(switch(&mut command)?),
                Some("--outputs" | "--nested") => return Err(Error::Usage),
                _ => break,
            }
            command.remove(0);
        }
        if headless && scales.is_empty() {
            scales.push(1.0);
        }
        // In nested mode, as many scales as screens of the window.
        if nested {
            scales = (0..screens.unwrap_or(1))
                .map(|i| scale_of(&scales, i))
                .collect();
        }
        Ok(Self {
            nested,
            headless,
            measure,
            xwayland,
            scales,
            workspaces,
            snap_gap,
            minimize_effect,
            window_animations,
            greeter,
            command,
        })
    }

    /// XWayland starts: always in nested mode, only with `--xwayland` in the real session.
    /// Xwayland 24 opens the NVIDIA card even when it draws in software (see "Real session" in
    /// `docs/compositor.md`), and the card must stay asleep.
    pub fn xwayland(&self) -> bool {
        self.nested || self.xwayland
    }

    /// The session stops when the command ends: the measured scene with `--measure`, the
    /// login screen with `--greeter` (greetd starts the chosen session once its greeter, this
    /// compositor, has left).
    pub fn stops_with_command(&self) -> bool {
        self.measure || self.greeter
    }

    /// The real session takes the screens and the input devices of the seat: it refuses to
    /// start inside another graphical session (`DISPLAY` or `WAYLAND_DISPLAY` set), where it
    /// would fight that session for them.
    pub fn check_environment(&self, var: impl Fn(&str) -> Option<OsString>) -> Result<(), Error> {
        let set = |key: &str| var(key).is_some_and(|v| !v.is_empty());
        if self.nested || self.headless || !(set("DISPLAY") || set("WAYLAND_DISPLAY")) {
            return Ok(());
        }
        Err(Error::InSession)
    }

    /// Scale of the screen number `i`, from 0.
    pub fn scale(&self, i: usize) -> f64 {
        scale_of(&self.scales, i)
    }
}

fn scale_of(scales: &[f64], i: usize) -> f64 {
    scales.get(i).or(scales.last()).copied().unwrap_or(1.0)
}

/// Value of the option in front of `args`, by its name, removed as `option` does.
fn named<T: std::str::FromStr>(args: &mut Vec<OsString>) -> Result<T, Error> {
    if args.len() < 2 {
        return Err(Error::Usage);
    }
    let value = args.remove(1);
    value
        .to_str()
        .and_then(|v| v.parse().ok())
        .ok_or(Error::Usage)
}

/// `on` or `off` in front of `args`, removed as `option` does.
fn switch(args: &mut Vec<OsString>) -> Result<bool, Error> {
    match named::<String>(args)?.as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(Error::Usage),
    }
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
            "--workspaces",
            "5",
            "--snap-gap",
            "0",
            "--minimize-effect",
            "scale",
            "--window-animations",
            "off",
            "konsole",
            "--scale",
        ])
        .unwrap();
        assert!(options.nested && options.measure);
        assert_eq!(options.scales, [2.0, 2.0]);
        assert_eq!(options.workspaces, Some(5));
        assert_eq!(options.snap_gap, Some(0));
        assert_eq!(parse(&[]).unwrap().snap_gap, None);
        assert_eq!(options.minimize_effect, Some(Effect::Scale));
        assert_eq!(parse(&[]).unwrap().minimize_effect, None);
        assert_eq!(options.window_animations, Some(false));
        assert_eq!(parse(&[]).unwrap().window_animations, None);
        let on = parse(&["--window-animations", "on"]).unwrap();
        assert_eq!(on.window_animations, Some(true));
        // What follows the command belongs to it.
        assert_eq!(options.command, ["konsole", "--scale"]);
        assert_eq!(parse(&["--nested"]).unwrap().scales, [1.0]);
    }

    #[test]
    fn the_greeter_stops_with_its_command_given_after_two_dashes() {
        let options = parse(&["--greeter", "--", "sherlockos-greeter", "--scale"]).unwrap();
        assert!(options.greeter && !options.nested && options.stops_with_command());
        assert_eq!(options.command, ["sherlockos-greeter", "--scale"]);
        // The two dashes are only taken once: a second pair belongs to the command.
        let options = parse(&["--", "--", "x"]).unwrap();
        assert!(!options.greeter && !options.stops_with_command());
        assert_eq!(options.command, ["--", "x"]);
        assert!(parse(&["--headless", "--greeter", "x"]).unwrap().greeter);
        assert!(parse(&["--nested", "--greeter"]).unwrap().greeter);
    }

    #[test]
    fn the_real_session_refuses_to_start_inside_another() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let real = parse(&["konsole"]).unwrap();
        // From a text console or the login screen: neither variable.
        assert!(real.check_environment(env(&[("HOME", "/home/me")])).is_ok());
        assert!(real.check_environment(env(&[("DISPLAY", "")])).is_ok());
        // Inside Plasma, X11 or Wayland.
        for inside in [
            &[("DISPLAY", ":0")][..],
            &[("WAYLAND_DISPLAY", "wayland-0")],
        ] {
            let refused = real.check_environment(env(inside));
            assert!(matches!(refused, Err(Error::InSession)), "{inside:?}");
            assert!(refused.unwrap_err().to_string().contains("--nested"));
        }
        // Nested mode is made for that.
        let nested = parse(&["--nested"]).unwrap();
        assert!(nested.check_environment(env(&[("DISPLAY", ":0")])).is_ok());
    }

    #[test]
    fn xwayland_is_asked_for_in_the_real_session() {
        assert!(!parse(&["konsole"]).unwrap().xwayland());
        assert!(parse(&["--xwayland", "konsole"]).unwrap().xwayland());
        assert!(parse(&["--nested"]).unwrap().xwayland());
    }

    #[test]
    fn wrong_options_are_refused() {
        for args in [
            &["--outputs", "2"][..],
            &["--nested", "--outputs", "5"],
            &["--scale", "9"],
            &["--scale"],
            &["--measure", "--nested"],
            &["--workspaces", "0"],
            &["--workspaces", "10"],
            &["--snap-gap", "-1"],
            &["--snap-gap", "65"],
            &["--minimize-effect", "lamp"],
            &["--minimize-effect"],
            &["--window-animations", "yes"],
            &["--window-animations"],
        ] {
            assert!(matches!(parse(args), Err(Error::Usage)), "{args:?}");
        }
    }
}
