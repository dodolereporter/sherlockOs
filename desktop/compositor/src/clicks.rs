//! Show the clicks (lot C-01): the accessibility option of the cursor boards. With
//! `show-clicks = true` in `pointer.conf` (`desktop_config::pointer`), each press of a button
//! leaves a green ring from the pressed point, and the pointer the compositor draws sinks a
//! little then comes back. Off by default. The file is read again when it changes, like the
//! touchpad's. Measures, colors and durations: `sherlock_ui::cursor`.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use desktop_config::pointer::Config;
use sherlock_ui::{
    cursor as ui,
    motion::{Ease, Fade},
};
use smithay::{
    output::Output,
    reexports::calloop::{
        LoopHandle,
        timer::{TimeoutAction, Timer},
    },
    utils::{Logical, Point, Rectangle},
};

use crate::{
    State,
    damage::{Key, Part},
    output::View,
    render::Item,
};

/// How often the settings file is looked at.
const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// The setting, its file, and the rings of the last clicks.
#[derive(Default)]
pub struct Clicks {
    config: Config,
    file: Option<PathBuf>,
    /// Modification time and length of the file when last read.
    stamp: Option<(SystemTime, u64)>,
    /// Each ring: an identity, the pressed point, when.
    rings: Vec<(u64, Point<f64, Logical>, Duration)>,
    next: u64,
    /// The last press, for the pointer that sinks.
    pressed: Option<Duration>,
}

impl Clicks {
    /// Reads the file again if it changed, appeared or went away. `true` if it was read.
    fn check(&mut self) -> bool {
        let stamp = self.file.as_deref().and_then(stamp);
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        self.config = match &self.file {
            Some(file) => match Config::read_lenient(file) {
                Ok((config, problems)) => {
                    for problem in problems {
                        tracing::warn!("pointer: {}: {problem}, line ignored", file.display());
                    }
                    config
                }
                Err(err) => {
                    tracing::warn!("pointer: {}: {err}", file.display());
                    Config::default()
                }
            },
            None => Config::default(),
        };
        tracing::info!(config = ?self.config, "pointer");
        true
    }
}

/// Modification time and length of a file, if it exists.
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// Scale and opacity of a ring `elapsed` after its click, `None` once it is gone.
pub fn ring(elapsed: Duration) -> Option<(f32, f32)> {
    if elapsed >= ui::CLICK_RING {
        return None;
    }
    let fade = Fade::eased(0.0, 1.0, Duration::ZERO, ui::CLICK_RING, Ease::OutCubic);
    let p = fade.value(elapsed);
    let (from, to) = ui::CLICK_RING_SCALE;
    Some((from + (to - from) * p, ui::CLICK_RING_OPACITY * (1.0 - p)))
}

/// Scale of the pointer `elapsed` after a press: down to `CLICK_PRESS_SCALE`, then back.
pub fn sink(elapsed: Duration) -> f32 {
    let depth = 1.0 - ui::CLICK_PRESS_SCALE;
    if elapsed < ui::CLICK_SINK {
        let t = elapsed.as_secs_f32() / ui::CLICK_SINK.as_secs_f32();
        return 1.0 - depth * Ease::OutCubic.apply(t);
    }
    let t = (elapsed - ui::CLICK_SINK).as_secs_f32() / ui::CLICK_RISE.as_secs_f32();
    ui::CLICK_PRESS_SCALE + depth * Ease::OutCubic.apply(t)
}

impl State {
    /// Reads the pointer setting of `file`, then looks at it again every `RELOAD_EVERY`.
    pub fn watch_pointer(
        &mut self,
        handle: &LoopHandle<'static, State>,
        file: Option<PathBuf>,
    ) -> Result<(), crate::Error> {
        self.clicks.file = file;
        self.clicks.check();
        handle
            .insert_source(Timer::from_duration(RELOAD_EVERY), |_, _, state| {
                state.clicks.check();
                TimeoutAction::ToDuration(RELOAD_EVERY)
            })
            .map_err(|err| err.error)?;
        Ok(())
    }

    /// Shows the clicks or not, as the file would: what Control Room's page will do.
    pub fn set_show_clicks(&mut self, on: bool) {
        self.clicks.config.show_clicks = on;
    }

    pub fn show_clicks(&self) -> bool {
        self.clicks.config.show_clicks
    }

    /// A button is pressed at `at`: a ring leaves it, if the clicks are shown.
    pub(crate) fn click_at(&mut self, at: Point<f64, Logical>) {
        if !self.clicks.config.show_clicks {
            return;
        }
        let now = self.now();
        let clicks = &mut self.clicks;
        clicks.rings.push((clicks.next, at, now));
        clicks.next += 1;
        clicks.pressed = Some(now);
    }

    /// Forgets the rings that are over, and the press once the pointer is back. Called after
    /// each loop iteration.
    pub(crate) fn end_clicks(&mut self) {
        let now = self.now();
        let clicks = &mut self.clicks;
        clicks
            .rings
            .retain(|(_, _, at)| ring(now.saturating_sub(*at)).is_some());
        let back = ui::CLICK_SINK + ui::CLICK_RISE;
        if clicks
            .pressed
            .is_some_and(|at| now.saturating_sub(at) >= back)
        {
            clicks.pressed = None;
        }
    }

    /// Scale of the pointer at `now`, and whether it is still moving: it sinks at a press
    /// while the clicks are shown.
    pub fn click_sink(&self, now: Duration) -> (f32, bool) {
        match self.clicks.pressed {
            Some(at) if now.saturating_sub(at) < ui::CLICK_SINK + ui::CLICK_RISE => {
                (sink(now.saturating_sub(at)), true)
            }
            _ => (1.0, false),
        }
    }
}

/// The rings of the clicks seen on `output` through `view` at `now`, in front of everything,
/// and whether one is running.
pub(crate) fn push_rings(
    items: &mut Vec<Item>,
    state: &State,
    (_output, view): (&Output, &View),
    now: Duration,
) -> bool {
    let mut running = false;
    for (id, at, when) in &state.clicks.rings {
        let Some((scale, alpha)) = ring(now.saturating_sub(*when)) else {
            continue;
        };
        running = true;
        let d = f64::from(ui::CLICK_RING_DIAMETER * scale);
        let rect = Rectangle::new((at.x - d / 2.0, at.y - d / 2.0).into(), (d, d).into());
        let rect = view.rect(rect);
        if !rect.overlaps(view.frame) {
            continue;
        }
        let px = view.scale as f32;
        items.push(Item::Shape {
            key: Key::of(None, Part::Click(*id)),
            rect,
            fill: [0.0; 4],
            border: ui::click_ring_color().premultiplied(),
            border_width: ui::CLICK_RING_WIDTH * px,
            radius: rect.size.w as f32 / 2.0,
            alpha,
        });
    }
    running
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_ring_grows_and_fades_out_in_460_ms() {
        let (scale, alpha) = ring(ms(0)).unwrap();
        assert_eq!((scale, alpha), (0.2, 0.9));
        let mut last = (scale, alpha);
        for t in [50, 150, 300, 450] {
            let now = ring(ms(t)).unwrap();
            assert!(now.0 > last.0 && now.1 < last.1, "{t}");
            last = now;
        }
        assert!(last.0 < 1.7 && last.0 > 1.6);
        assert_eq!(ring(ms(460)), None);
    }

    #[test]
    fn the_pointer_sinks_then_comes_back() {
        assert_eq!(sink(ms(0)), 1.0);
        assert!((sink(ms(54)) - 0.86).abs() < 1e-6);
        assert!(sink(ms(100)) > 0.86 && sink(ms(100)) < 1.0);
        assert!((sink(ms(180)) - 1.0).abs() < 1e-6);
    }
}
