//! Changing the screens (ADR 0025, lot D-26b): mode, scale, place, rotation, on or off, as a
//! whole configuration that is checked before anything changes, then applied to the
//! hardware first and to the space after. The setting of each screen is saved under its EDID
//! names in `displays.conf`, once it has held for `SAVE_AFTER`, and taken again when the
//! screen comes back. `wlr-output-management` (`output_management.rs`, D-26b3) is how clients ask.

use std::{path::PathBuf, time::Duration};

use desktop_config::displays::{self as file, Identity};
use smithay::{
    desktop::Window,
    output::{Mode, Output, Scale},
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Point, Rectangle, Transform},
};

use crate::{State, output::Screen};

/// How long a configuration holds before it is saved: longer than the 15 seconds Control
/// Room waits for a confirmation. A session that dies before keeps the setting that worked.
pub const SAVE_AFTER: Duration = Duration::from_secs(20);

/// Changes what a screen of the real session shows: lit at a mode, turned, or switched off
/// (`None`). An error leaves the screen as it was.
pub type Driver = Box<dyn FnMut(&Output, Option<(Mode, Transform)>) -> Result<(), String>>;

/// The screens switched off, the saved settings and where they go.
#[derive(Default)]
pub struct Displays {
    file: Option<PathBuf>,
    saved: file::Config,
    /// Screens switched off: still listed to clients that configure them.
    pub(crate) off: Vec<Screen>,
    driver: Option<Driver>,
    save: Option<RegistrationToken>,
}

impl Displays {
    /// Forgets a screen unplugged while off.
    pub(crate) fn forget(&mut self, output: &Output) {
        self.off.retain(|s| &s.output != output);
    }
}

/// What a client asks of one screen. `None` fields keep what the screen has.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Setting {
    pub mode: Option<Mode>,
    pub position: Option<Point<i32, Logical>>,
    pub transform: Option<Transform>,
    pub scale: Option<f64>,
}

/// What a screen becomes: `None` is off.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub output: Output,
    pub on: Option<Setting>,
}

/// A screen lit, everything set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lit {
    pub mode: Mode,
    pub position: Point<i32, Logical>,
    pub transform: Transform,
    pub scale: f64,
}

/// Why a configuration is refused. Nothing changed.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum Refused {
    #[error("at least one screen stays on")]
    AllOff,
    #[error("{0} is not a screen")]
    Unknown(String),
    #[error("{0} cannot take this mode")]
    Mode(String),
    #[error("{0} cannot turn")]
    Rotation(String),
    #[error("{0} cannot be switched off")]
    Off(String),
    #[error("scale {0} is out of 0.5 to 4")]
    Scale(f64),
    #[error("position out of reach")]
    Position,
    #[error("the screen refused: {0}")]
    Driver(String),
}

/// `desktop-config`'s rotation as smithay's.
pub fn transform(t: file::Transform) -> Transform {
    match t {
        file::Transform::Normal => Transform::Normal,
        file::Transform::Rotate90 => Transform::_90,
        file::Transform::Rotate180 => Transform::_180,
        file::Transform::Rotate270 => Transform::_270,
        file::Transform::Flipped => Transform::Flipped,
        file::Transform::Flipped90 => Transform::Flipped90,
        file::Transform::Flipped180 => Transform::Flipped180,
        file::Transform::Flipped270 => Transform::Flipped270,
    }
}

/// Smithay's rotation as `desktop-config`'s.
pub fn turn(t: Transform) -> file::Transform {
    file::Transform::ALL
        .into_iter()
        .find(|f| transform(*f) == t)
        .unwrap_or_default()
}

/// The mode of `modes` that `wanted` names: the same size and refresh, or the same size when
/// the refresh is not given (0).
fn pick(modes: &[Mode], wanted: Mode) -> Option<Mode> {
    let same = |m: &&Mode| m.size == wanted.size;
    modes
        .iter()
        .filter(same)
        .find(|m| m.refresh == wanted.refresh || wanted.refresh == 0)
        .or_else(|| {
            // A refresh given to the hertz, as `wlr-randr --mode 1920x1080@60` does.
            let near = |m: &&Mode| (m.refresh - wanted.refresh).abs() < 500;
            modes.iter().filter(same).find(near)
        })
        .copied()
}

impl State {
    /// Reads the saved settings of the screens, and saves there what is applied from now on.
    pub fn remember_screens(&mut self, path: Option<PathBuf>) {
        if let Some(path) = &path {
            match file::Config::read(path) {
                Ok((saved, problems)) => {
                    for problem in problems {
                        tracing::warn!("{}: {problem}", path.display());
                    }
                    self.displays.saved = saved;
                }
                Err(err) => tracing::warn!("screens: {err}"),
            }
        }
        self.displays.file = path;
    }

    /// The real session changes its screens through `driver`.
    pub(crate) fn set_screen_driver(&mut self, driver: Driver) {
        self.displays.driver = Some(driver);
    }

    /// What was saved of the screen `identity`.
    pub(crate) fn saved_screen(&self, identity: &Identity) -> Option<file::Screen> {
        self.displays.saved.get(identity).cloned()
    }

    /// Every screen, those switched off last, with what it shows if it is on.
    pub fn heads(&self) -> Vec<(&Screen, Option<Lit>)> {
        let on = self.screens.iter().map(|s| (s, self.lit(s)));
        on.chain(self.displays.off.iter().map(|s| (s, None)))
            .collect()
    }

    fn lit(&self, screen: &Screen) -> Option<Lit> {
        let output = &screen.output;
        Some(Lit {
            mode: output.current_mode()?,
            position: self.space.output_geometry(output)?.loc,
            transform: output.current_transform(),
            scale: output.current_scale().fractional_scale(),
        })
    }

    /// Every screen as it would be with `wanted`, or why not. Screens left out of `wanted`
    /// stay as they are.
    pub fn check_screens(&self, wanted: &[Wanted]) -> Result<Vec<(Output, Option<Lit>)>, Refused> {
        let heads = self.heads();
        if let Some(w) = wanted
            .iter()
            .find(|w| !heads.iter().any(|(s, _)| s.output == w.output))
        {
            return Err(Refused::Unknown(w.output.name()));
        }
        let mut targets = Vec::new();
        for (screen, now) in heads {
            let name = screen.output.name();
            let details = &screen.details;
            let asked = wanted.iter().find(|w| w.output == screen.output);
            let target = match asked.map(|w| &w.on) {
                None => now,
                Some(None) if details.modes.is_empty() => return Err(Refused::Off(name)),
                Some(None) => None,
                Some(Some(setting)) => {
                    let current = now.map(|l| l.mode).or(screen.output.current_mode());
                    let modes = match details.modes.is_empty() {
                        true => current.into_iter().collect(),
                        false => details.modes.clone(),
                    };
                    let mode = match setting.mode {
                        Some(m) => pick(&modes, m).ok_or_else(|| Refused::Mode(name.clone()))?,
                        None => current
                            .or(modes.first().copied())
                            .ok_or(Refused::Mode(name.clone()))?,
                    };
                    let transform = setting
                        .transform
                        .unwrap_or(now.map_or(Transform::Normal, |l| l.transform));
                    if transform != Transform::Normal && !details.rotates {
                        return Err(Refused::Rotation(name));
                    }
                    let scale = setting
                        .scale
                        .unwrap_or(screen.output.current_scale().fractional_scale());
                    if !(file::SCALES.0..=file::SCALES.1).contains(&scale) {
                        return Err(Refused::Scale(scale));
                    }
                    let position = setting
                        .position
                        .or(now.map(|l| l.position))
                        .or(screen.position)
                        .unwrap_or_default();
                    if position.x.abs() > file::MAX_POSITION
                        || position.y.abs() > file::MAX_POSITION
                    {
                        return Err(Refused::Position);
                    }
                    Some(Lit {
                        mode,
                        position,
                        transform,
                        scale,
                    })
                }
            };
            targets.push((screen.output.clone(), target));
        }
        if targets.iter().all(|(_, t)| t.is_none()) {
            return Err(Refused::AllOff);
        }
        Ok(targets)
    }

    /// Applies `wanted`: checked first, then given to the hardware (a refusal there puts back
    /// what had changed), then to the space. Windows follow their screen. Saved after
    /// `SAVE_AFTER`.
    pub fn apply_screens(&mut self, wanted: &[Wanted]) -> Result<(), Refused> {
        let targets = self.check_screens(wanted)?;
        let before: Vec<(Output, Option<Lit>)> = self
            .heads()
            .into_iter()
            .map(|(s, lit)| (s.output.clone(), lit))
            .collect();
        let hardware = |lit: Option<Lit>| lit.map(|l| (l.mode, l.transform));
        if let Some(driver) = self.displays.driver.as_mut() {
            let mut done = Vec::new();
            for (output, target) in &targets {
                let old = before
                    .iter()
                    .find(|(o, _)| o == output)
                    .and_then(|(_, l)| *l);
                if hardware(old) == hardware(*target) {
                    continue;
                }
                if let Err(err) = driver(output, hardware(*target)) {
                    for (output, back) in done.into_iter().rev() {
                        if let Err(err) = driver(&output, back) {
                            tracing::warn!(screen = output.name(), "putting back: {err}");
                        }
                    }
                    return Err(Refused::Driver(err));
                }
                done.push((output.clone(), hardware(old)));
            }
        }
        self.set_screens(&targets, &before);
        self.save_screens_later();
        Ok(())
    }

    /// Gives the space the screens of `targets`; `before` is how they were.
    fn set_screens(&mut self, targets: &[(Output, Option<Lit>)], before: &[(Output, Option<Lit>)]) {
        self.queue_redraw();
        let areas: Vec<(Output, Rectangle<i32, Logical>)> = self
            .space
            .outputs()
            .filter_map(|o| Some((o.clone(), self.space.output_geometry(o)?)))
            .collect();
        let homes: Vec<(Window, Output)> = self
            .space
            .elements()
            .filter_map(|w| Some((w.clone(), self.window_output(w)?)))
            .collect();
        for (output, target) in targets {
            let Some(lit) = target else { continue };
            if let Some(i) = self.displays.off.iter().position(|s| &s.output == output) {
                let mut screen = self.displays.off.remove(i);
                screen.relight(&self.display_handle);
                self.screens.push(screen);
            }
            let nested = self.nested;
            let Some(screen) = self.screens.iter_mut().find(|s| &s.output == output) else {
                continue;
            };
            output.change_current_state(
                Some(lit.mode),
                Some(lit.transform),
                Some(Scale::Fractional(lit.scale)),
                None,
            );
            if !(nested && screen.details.modes.is_empty()) {
                screen.frame.size = lit.transform.transform_size(lit.mode.size);
            }
            screen.position = Some(lit.position);
        }
        self.place_screens();
        // Windows keep their place on their screen.
        for (window, output) in homes {
            let (Some((_, old)), Some(new)) = (
                areas.iter().find(|(o, _)| *o == output),
                self.space.output_geometry(&output),
            ) else {
                continue;
            };
            if old.loc == new.loc {
                continue;
            }
            if let Some(geometry) = self.space.element_geometry(&window) {
                let loc = geometry.loc - old.loc + new.loc;
                self.move_window(&window, loc, false);
            }
        }
        for (output, target) in targets {
            let was_on = before.iter().any(|(o, l)| o == output && l.is_some());
            if target.is_none()
                && was_on
                && let Some(i) = self.screens.iter().position(|s| &s.output == output)
            {
                let screen = self.screens.remove(i);
                let off = screen.clone();
                self.unplug(screen);
                self.displays.off.push(off.switched_off());
            }
        }
        for output in self.outputs() {
            self.reflow(&output);
        }
    }

    /// Saves the screens once their configuration has held for `SAVE_AFTER`; a newer one
    /// starts the wait again.
    fn save_screens_later(&mut self) {
        if self.displays.file.is_none() {
            return;
        }
        if let Some(token) = self.displays.save.take() {
            self.loop_handle.remove(token);
        }
        let timer = Timer::from_duration(SAVE_AFTER);
        let token = self.loop_handle.insert_source(timer, |_, _, state| {
            state.displays.save = None;
            state.save_screens();
            TimeoutAction::Drop
        });
        match token {
            Ok(token) => self.displays.save = Some(token),
            Err(err) => tracing::warn!("saving the screens later: {err}"),
        }
    }

    /// Writes every screen with EDID names to `displays.conf`, off the event loop.
    pub fn save_screens(&mut self) {
        let Some(path) = self.displays.file.clone() else {
            return;
        };
        let mut records = Vec::new();
        for (screen, lit) in self.heads() {
            let identity = screen.details.identity.clone();
            if identity == Identity::default() {
                continue;
            }
            let mut record = self
                .saved_screen(&identity)
                .unwrap_or_else(|| file::Screen::new(identity));
            record.enabled = lit.is_some();
            if let Some(lit) = lit {
                record.mode = Some(file::Mode {
                    width: lit.mode.size.w,
                    height: lit.mode.size.h,
                    refresh: lit.mode.refresh,
                });
                record.scale = lit.scale;
                record.position = (lit.position.x, lit.position.y);
                record.transform = turn(lit.transform);
            }
            records.push(record);
        }
        for record in records {
            self.displays.saved.put(record);
        }
        let text = self.displays.saved.to_text();
        let written = std::thread::Builder::new()
            .name("displays.conf".into())
            .spawn(move || {
                if let Err(err) = mime_apps::write_atomically(&path, &text) {
                    tracing::warn!("{}: {err}", path.display());
                }
            });
        if let Err(err) = written {
            tracing::warn!("saving the screens: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(w: i32, h: i32, refresh: i32) -> Mode {
        Mode {
            size: (w, h).into(),
            refresh,
        }
    }

    #[test]
    fn a_mode_is_found_by_size_and_refresh() {
        let modes = [
            mode(1920, 1080, 143_981),
            mode(1920, 1080, 60_000),
            mode(1280, 720, 60_000),
        ];
        assert_eq!(pick(&modes, mode(1920, 1080, 60_000)), Some(modes[1]));
        assert_eq!(pick(&modes, mode(1920, 1080, 0)), Some(modes[0]));
        assert_eq!(pick(&modes, mode(1920, 1080, 144_000)), Some(modes[0]));
        assert_eq!(pick(&modes, mode(1920, 1080, 75_000)), None);
        assert_eq!(pick(&modes, mode(800, 600, 0)), None);
    }

    #[test]
    fn rotations_go_both_ways() {
        for t in file::Transform::ALL {
            assert_eq!(turn(transform(t)), t);
        }
    }
}
