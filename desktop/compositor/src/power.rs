//! The screens off (lot D-38), like an old television: the picture is squashed from the top and
//! the bottom into a bright line in the middle, the line shrinks to a dot, and the dot fades
//! out (`sherlock_ui::screen_off`, 350 ms). Then the screens are dark: nothing is drawn, and the
//! real session turns them off (DPMS, through DRM). Backwards when they come back.
//!
//! The screens go off:
//! - before the sleep (`PrepareForSleep`): the session locks, the screens go off, then the sleep
//!   inhibitor is released (`triggers`);
//! - when the lid closes (the session locks too);
//! - after `screen-off-minutes` of `power.conf` without input (10 by default, 0 never);
//! - with Super + Shift + Escape, which locks the session too.
//!
//! A key, the pointer, a gesture, the lid opened or the wake-up bring them back; the key or the
//! click that wakes them goes nowhere else. While something keeps the session awake (a visible
//! video, a cookie on the bus, logind: `inhibit.rs`, lot D-43), neither the inactivity lock nor
//! the screens off for inactivity happen.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use desktop_config::power::Config;
use sherlock_ui::{color, screen_off as ui};
use smithay::{
    reexports::calloop::{
        LoopHandle, RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::{Physical, Rectangle},
};

use crate::{
    State,
    damage::{Key, Part},
    output::View,
    render::Item,
};

/// How often the clients get a frame callback while the screens are dark.
pub const DARK_FRAMES: Duration = Duration::from_secs(1);
/// How often the setting is read again.
const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// What follows the screens off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Then {
    /// They stay off until an input.
    Stay,
    /// The machine sleeps: the sleep inhibitor goes once they are dark.
    Sleep,
}

/// Where the screens stand. Times are counted from the start of the compositor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Phase {
    On,
    Leaving { since: Duration, then: Then },
    Dark { then: Then },
    Coming { since: Duration },
}

/// What a screen shows at a point of the animation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Crt {
    /// The picture, `height` of the screen tall, centered, with a light of opacity `glow` over
    /// it.
    Picture { height: f32, glow: f32 },
    /// The line, `width` of the screen wide (0: the dot), of opacity `alpha`.
    Line { width: f32, alpha: f32 },
    /// Nothing but black.
    Black,
}

fn ease_in(t: f32) -> f32 {
    t * t * t
}

fn ease_in_out(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// The old television at `progress`, from 0 (the whole picture) to 1 (black).
pub fn crt(progress: f32) -> Crt {
    let p = progress.clamp(0.0, 1.0);
    if p < ui::SQUASH {
        let e = ease_in(p / ui::SQUASH);
        return Crt::Picture {
            height: 1.0 - e,
            glow: ui::GLOW * e,
        };
    }
    if p < ui::SHRINK {
        let q = (p - ui::SQUASH) / (ui::SHRINK - ui::SQUASH);
        return Crt::Line {
            width: 1.0 - ease_in_out(q),
            alpha: 1.0,
        };
    }
    if p < 1.0 {
        return Crt::Line {
            width: 0.0,
            alpha: 1.0 - (p - ui::SHRINK) / (1.0 - ui::SHRINK),
        };
    }
    Crt::Black
}

/// The screens, their setting and what holds them on.
pub(crate) struct Power {
    phase: Phase,
    config: Config,
    file: Option<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    /// A delay set by the tests, over the file's.
    forced: Option<Option<Duration>>,
    /// The timer of the frame callbacks while dark.
    dark_frames: Option<RegistrationToken>,
}

impl Power {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::On,
            config: Config::default(),
            file: None,
            stamp: None,
            forced: None,
            dark_frames: None,
        }
    }

    fn reload(&mut self) {
        let stamp = self.file.as_deref().and_then(stamp);
        if stamp == self.stamp {
            return;
        }
        self.stamp = stamp;
        self.config = match &self.file {
            Some(file) => Config::read(file).unwrap_or_else(|err| {
                tracing::warn!("power: {}: {err}", file.display());
                Config::default()
            }),
            None => Config::default(),
        };
        tracing::info!(screen_off = ?self.config.screen_off(), "power setting");
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

impl State {
    /// Reads the power setting of `file`, then again every `RELOAD_EVERY`.
    pub fn watch_power(
        &mut self,
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
    ) -> Result<(), crate::Error> {
        self.power.file = file;
        self.power.reload();
        handle
            .insert_source(Timer::from_duration(RELOAD_EVERY), |_, _, state| {
                state.power.reload();
                TimeoutAction::ToDuration(RELOAD_EVERY)
            })
            .map_err(|err| err.error)?;
        Ok(())
    }

    /// The delay of inactivity before the screens go off, over the file's (the tests).
    pub fn set_screen_off_delay(&mut self, delay: Option<Duration>) {
        self.power.forced = Some(delay);
    }

    pub(crate) fn screen_off_delay(&self) -> Option<Duration> {
        self.power
            .forced
            .unwrap_or_else(|| self.power.config.screen_off())
    }

    /// Where the screens stand.
    pub fn power_phase(&self) -> Phase {
        self.power.phase
    }

    /// The screens are dark: nothing is drawn, the real session turns them off.
    pub fn screens_dark(&self) -> bool {
        matches!(self.power.phase, Phase::Dark { .. })
    }

    /// An input now brings the screens back, and goes nowhere else.
    pub(crate) fn input_wakes(&self) -> bool {
        matches!(self.power.phase, Phase::Leaving { .. } | Phase::Dark { .. })
    }

    /// The screens go off, then `then`.
    pub fn screens_off(&mut self, then: Then) {
        let now = self.now();
        self.power.phase = match self.power.phase {
            Phase::Dark { .. } => Phase::Dark { then },
            Phase::Leaving { since, .. } => Phase::Leaving { since, then },
            Phase::On | Phase::Coming { .. } => {
                tracing::info!(?then, "the screens go off");
                Phase::Leaving { since: now, then }
            }
        };
        self.queue_redraw();
    }

    /// The screens come back.
    pub fn screens_on(&mut self) {
        if matches!(self.power.phase, Phase::Leaving { .. } | Phase::Dark { .. }) {
            tracing::info!("the screens come back");
            self.power.phase = Phase::Coming { since: self.now() };
            self.queue_redraw();
        }
    }

    /// The animation at `now`: `None` with the screens on, from 0 (the whole picture) to 1
    /// (black) otherwise.
    pub fn screen_off_progress(&self, now: Duration) -> Option<f32> {
        let share = |since: Duration| {
            (now.saturating_sub(since).as_secs_f32() / ui::DURATION.as_secs_f32()).min(1.0)
        };
        match self.power.phase {
            Phase::On => None,
            Phase::Leaving { since, .. } => Some(share(since)),
            Phase::Dark { .. } => Some(1.0),
            Phase::Coming { since } => Some(1.0 - share(since)),
        }
    }

    /// While dark, nothing is drawn, but the clients still get their frame callbacks, every
    /// `DARK_FRAMES`: one waiting for its callback before it presents again (Mesa's Vulkan and
    /// EGL do) would stay blocked, the lock program first, which then answers nothing.
    fn send_dark_frames(&mut self) {
        if self.power.dark_frames.is_some() {
            return;
        }
        let timer = Timer::from_duration(DARK_FRAMES);
        let inserted = self.loop_handle.insert_source(timer, |_, _, state| {
            if !state.screens_dark() {
                state.power.dark_frames = None;
                return TimeoutAction::Drop;
            }
            for output in state.outputs() {
                state.send_frames(&output);
            }
            TimeoutAction::ToDuration(DARK_FRAMES)
        });
        match inserted {
            Ok(token) => self.power.dark_frames = Some(token),
            Err(err) => tracing::warn!("no frame callbacks while dark: {}", err.error),
        }
    }

    /// Cleanup of each loop iteration: the animation ends.
    pub(crate) fn refresh_power(&mut self) {
        let elapsed = |since: Duration| self.now().saturating_sub(since) >= ui::DURATION;
        match self.power.phase {
            Phase::Leaving { since, then } if elapsed(since) => {
                tracing::info!("the screens are dark");
                self.power.phase = Phase::Dark { then };
                self.queue_redraw();
                self.release_sleep();
                self.send_dark_frames();
            }
            Phase::Coming { since } if elapsed(since) => {
                self.power.phase = Phase::On;
                self.queue_redraw();
            }
            _ => {}
        }
    }
}

/// What a screen shows while the screens go off, are dark or come back: black, and the picture
/// squashed (`inner`, the screen's items in a view of its own at the origin), the line or the
/// dot of `crt(progress)`.
pub fn items(
    view: &View,
    progress: f32,
    dark: bool,
    inner: impl FnOnce(&View) -> (Vec<Item>, bool),
) -> (Vec<Item>, bool) {
    let mut items = Vec::new();
    let frame = view.frame;
    let scale = view.scale as f32;
    let light = ui::light(color::palette());
    let line = (ui::LINE * scale).round().max(1.0) as i32;
    let center_y = frame.loc.y + frame.size.h / 2;
    let center_x = frame.loc.x + frame.size.w / 2;
    let mut animating = !dark;
    match crt(progress) {
        Crt::Picture { height, glow } => {
            let h = ((frame.size.h as f32 * height).round() as i32).max(line);
            let rect = Rectangle::new(
                (frame.loc.x, center_y - h / 2).into(),
                (frame.size.w, h).into(),
            );
            let shape = |part, alpha: f32| Item::Shape {
                key: Key::of(None, part),
                rect,
                fill: light.premultiplied(),
                border: light.premultiplied(),
                border_width: 0.0,
                radius: 0.0,
                alpha,
            };
            items.push(shape(Part::ScreenOffGlow, glow));
            let local = View {
                frame: Rectangle::from_size(frame.size),
                frame_size: frame.size,
                ..*view
            };
            let (content, moving) = inner(&local);
            animating |= moving;
            items.push(Item::Squashed {
                key: Key::of(None, Part::ScreenOffPicture),
                content,
                size: frame.size,
                rect,
                alpha: 1.0,
            });
        }
        Crt::Line { width, alpha } => {
            let w = ((frame.size.w as f32 * width).round() as i32).max(line);
            let centered = |w: i32, h: i32| -> Rectangle<i32, Physical> {
                Rectangle::new((center_x - w / 2, center_y - h / 2).into(), (w, h).into())
            };
            let round = |part, rect: Rectangle<i32, Physical>, alpha: f32| Item::Shape {
                key: Key::of(None, part),
                rect,
                fill: light.premultiplied(),
                border: light.premultiplied(),
                border_width: 0.0,
                radius: rect.size.h as f32 / 2.0,
                alpha,
            };
            items.push(round(Part::ScreenOffLine, centered(w, line), alpha));
            let halo = (line as f32 * ui::HALO).round() as i32;
            items.push(round(
                Part::ScreenOffHalo,
                centered(w + halo, halo),
                alpha * ui::HALO_OPACITY,
            ));
        }
        Crt::Black => {}
    }
    let black = ui::BLACK.premultiplied();
    items.push(Item::Shape {
        key: Key::of(None, Part::ScreenOff),
        rect: frame,
        fill: black,
        border: black,
        border_width: 0.0,
        radius: 0.0,
        alpha: 1.0,
    });
    (items, animating)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_becomes_a_line_then_a_dot_then_black() {
        assert_eq!(
            crt(0.0),
            Crt::Picture {
                height: 1.0,
                glow: 0.0
            }
        );
        let Crt::Picture { height, glow } = crt(ui::SQUASH / 2.0) else {
            panic!("still the picture")
        };
        assert!(height < 1.0 && height > 0.0 && glow > 0.0);
        // Narrower and narrower, then the dot fading out.
        let widths: Vec<f32> = [0.6, 0.7, 0.8]
            .map(|p| match crt(p) {
                Crt::Line { width, .. } => width,
                other => panic!("{other:?}"),
            })
            .to_vec();
        assert!(widths.windows(2).all(|w| w[0] > w[1]), "{widths:?}");
        let Crt::Line { width, alpha } = crt(0.95) else {
            panic!("the dot")
        };
        assert_eq!(width, 0.0);
        assert!(alpha > 0.0 && alpha < 1.0);
        // The last frame is black.
        assert_eq!(crt(1.0), Crt::Black);
        assert_eq!(crt(2.0), Crt::Black);
    }
}
