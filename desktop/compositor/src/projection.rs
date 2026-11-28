//! The projection chooser (lot D-39): Super + P opens a small panel in the middle of each
//! screen, with four choices: the laptop's screen alone, both showing the same, the second to
//! the right, the second alone. Super + P again, or the arrows, move the choice; Enter or a click
//! applies it, Escape or a click outside closes. Space switches Do not disturb for the time of
//! the projection.
//!
//! A choice is a whole configuration of the screens (ADR 0025, `displays.rs`): checked, then
//! applied, then saved with the screens; the second screen keeps how it was projected on
//! (`projection` in `displays.conf`), taken again when it is plugged in. Duplicating screens of
//! different sizes scales the second so that it shows the whole of the laptop's screen, centred.
//! When the last screen lit is unplugged, the laptop's screen comes back (`relight`).

use std::path::PathBuf;
use std::time::Duration;

use desktop_config::displays::Projection;
use sherlock_ui::{
    color,
    projection::{self as ui, Colors, text},
};
use smithay::{
    input::keyboard::Keysym,
    output::{Mode, Output},
    utils::{Logical, Point, Rectangle, Size},
};

use crate::{
    State,
    damage::{Key, Part},
    displays::{Setting, Wanted},
    mission_control::{Label, RectF},
    output::View,
    render::Item,
};

/// The panel while it is open.
#[derive(Debug, Clone, Copy)]
struct Open {
    /// The choice under the highlight, and the one before it, for the slide.
    selected: usize,
    previous: usize,
    /// When it opened, and when the highlight last moved.
    opened_at: Duration,
    moved_at: Duration,
    do_not_disturb: bool,
}

/// The chooser, and Do not disturb as it was before a projection switched it on.
#[derive(Debug, Default)]
pub struct Chooser {
    open: Option<Open>,
    /// `notifications.conf`, in the real session; none in the tests that do not give one.
    notifications: Option<PathBuf>,
    /// Do not disturb was switched on by a projection: switched off again when it ends.
    switched_on: bool,
}

/// What the panel is on a screen: its rectangle, the tiles, the line of Do not disturb.
#[derive(Debug, Clone)]
pub struct Layout {
    pub panel: RectF,
    pub title: RectF,
    pub tiles: [RectF; 4],
    pub option: RectF,
}

/// The internal screen: its connector is `eDP`, `LVDS` or `DSI`.
fn internal(output: &Output) -> bool {
    let name = output.name();
    ["eDP", "LVDS", "DSI"].iter().any(|p| name.starts_with(p))
}

/// The label of a choice.
fn label(choice: Projection) -> &'static str {
    match choice {
        Projection::Laptop => text::LAPTOP,
        Projection::Duplicate => text::DUPLICATE,
        Projection::Extend => text::EXTEND,
        Projection::Second => text::SECOND,
    }
}

/// The size in points of a screen at `mode` and `scale`.
fn logical(mode: Mode, scale: f64) -> Size<f64, Logical> {
    (
        f64::from(mode.size.w) / scale,
        f64::from(mode.size.h) / scale,
    )
        .into()
}

impl State {
    /// Where Do not disturb is switched for a projection (`notifications.conf`).
    pub fn set_projection_notifications(&mut self, path: Option<PathBuf>) {
        self.projection.notifications = path;
    }

    pub fn projection_open(&self) -> bool {
        self.projection.open.is_some()
    }

    /// The choice under the highlight, while the panel is open.
    pub fn projection_selected(&self) -> Option<Projection> {
        self.projection.open.map(|o| Projection::ALL[o.selected])
    }

    /// The laptop's screen and the second one, lit or not: the internal screen, or else the
    /// first; the first other one. `None` with a single screen.
    pub fn projection_screens(&self) -> Option<(Output, Output)> {
        let outputs: Vec<Output> = self
            .heads()
            .into_iter()
            .map(|(screen, _)| screen.output.clone())
            .collect();
        let laptop = outputs
            .iter()
            .find(|o| internal(o))
            .or(outputs.first())?
            .clone();
        let second = outputs.iter().find(|o| **o != laptop)?.clone();
        Some((laptop, second))
    }

    /// How the screens are projected now, as far as one of the four says it.
    pub fn projection_now(&self) -> Option<Projection> {
        let (laptop, second) = self.projection_screens()?;
        let heads = self.heads();
        let lit = |o: &Output| heads.iter().find(|(s, _)| s.output == *o).and_then(|h| h.1);
        Some(match (lit(&laptop), lit(&second)) {
            (Some(_), None) => Projection::Laptop,
            (None, Some(_)) => Projection::Second,
            (Some(a), Some(b)) => {
                let (a, b) = (
                    Rectangle::new(a.position.to_f64(), logical(a.mode, a.scale)),
                    Rectangle::new(b.position.to_f64(), logical(b.mode, b.scale)),
                );
                if a.overlaps(b) {
                    Projection::Duplicate
                } else {
                    Projection::Extend
                }
            }
            (None, None) => return None,
        })
    }

    /// Super + P: opens the panel on the choice after the current one, or moves to the next
    /// choice when it is open. Nothing with a single screen.
    pub fn toggle_projection(&mut self) {
        let now = self.now();
        self.queue_redraw();
        if let Some(open) = self.projection.open.as_mut() {
            open.previous = open.selected;
            open.selected = (open.selected + 1) % Projection::ALL.len();
            open.moved_at = now;
            return;
        }
        if self.projection_screens().is_none() {
            tracing::debug!("Super + P: a single screen, nothing to project on");
            return;
        }
        let current = self.projection_now().unwrap_or(Projection::Laptop);
        let at = Projection::ALL
            .iter()
            .position(|p| *p == current)
            .unwrap_or(0);
        let selected = (at + 1) % Projection::ALL.len();
        self.projection.open = Some(Open {
            selected,
            previous: selected,
            opened_at: now,
            moved_at: now.saturating_sub(ui::SLIDE),
            do_not_disturb: true,
        });
    }

    /// A key while the panel is open.
    pub fn projection_key(&mut self, sym: Keysym) {
        let now = self.now();
        self.queue_redraw();
        let Some(open) = self.projection.open.as_mut() else {
            return;
        };
        let count = Projection::ALL.len();
        let step = match sym {
            Keysym::Right | Keysym::Down | Keysym::p | Keysym::P => Some(1),
            Keysym::Left | Keysym::Up => Some(count - 1),
            _ => None,
        };
        if let Some(step) = step {
            open.previous = open.selected;
            open.selected = (open.selected + step) % count;
            open.moved_at = now;
            return;
        }
        match sym {
            Keysym::space => open.do_not_disturb = !open.do_not_disturb,
            Keysym::Return | Keysym::KP_Enter => {
                let (choice, quiet) = (Projection::ALL[open.selected], open.do_not_disturb);
                self.projection.open = None;
                self.project(choice, quiet);
            }
            Keysym::Escape => self.projection.open = None,
            _ => {}
        }
    }

    /// A click while the panel is open: on a choice, it is applied; outside, the panel closes.
    pub fn projection_click(&mut self, at: Point<f64, Logical>) {
        let Some(open) = self.projection.open else {
            return;
        };
        self.queue_redraw();
        let layouts: Vec<Layout> = self
            .space
            .outputs()
            .filter_map(|o| self.projection_layout(o))
            .collect();
        for layout in &layouts {
            if let Some(i) = layout.tiles.iter().position(|t| t.contains(at)) {
                self.projection.open = None;
                self.project(Projection::ALL[i], open.do_not_disturb);
                return;
            }
            if layout.option.contains(at) {
                if let Some(open) = self.projection.open.as_mut() {
                    open.do_not_disturb = !open.do_not_disturb;
                }
                return;
            }
            if layout.panel.contains(at) {
                return;
            }
        }
        self.projection.open = None;
    }

    /// Applies `choice`, and Do not disturb for the time of the projection when `quiet`.
    pub fn project(&mut self, choice: Projection, quiet: bool) {
        let Some(wanted) = self.projection_wanted(choice) else {
            return;
        };
        match self.apply_screens(&wanted) {
            Ok(()) => {
                tracing::info!(choice = choice.as_str(), "projection");
                if let Some((_, second)) = self.projection_screens() {
                    self.remember_projection(&second, choice);
                }
                self.projection_quiet(choice != Projection::Laptop && quiet);
            }
            Err(err) => tracing::warn!(choice = choice.as_str(), "projection refused: {err}"),
        }
    }

    /// The configuration of `choice`: each screen at its current mode (its preferred one when
    /// it is off) and scale, the laptop's at the origin.
    pub fn projection_wanted(&self, choice: Projection) -> Option<Vec<Wanted>> {
        let (laptop, second) = self.projection_screens()?;
        let heads = self.heads();
        let mode_of = |o: &Output| {
            let (screen, lit) = heads.iter().find(|(s, _)| s.output == *o)?;
            lit.map(|l| l.mode)
                .or_else(|| screen.details.modes.first().copied())
                .or_else(|| o.current_mode())
        };
        let scale_of = |o: &Output| o.current_scale().fractional_scale();
        let (laptop_mode, second_mode) = (mode_of(&laptop)?, mode_of(&second)?);
        let at = |output: &Output, mode: Mode, x: i32, y: i32, scale: f64| Wanted {
            output: output.clone(),
            on: Some(Setting {
                mode: Some(mode),
                position: Some((x, y).into()),
                transform: None,
                scale: Some(scale),
            }),
        };
        let off = |output: &Output| Wanted {
            output: output.clone(),
            on: None,
        };
        let laptop_scale = scale_of(&laptop);
        let laptop_size = logical(laptop_mode, laptop_scale);
        let mut wanted = match choice {
            Projection::Laptop => vec![at(&laptop, laptop_mode, 0, 0, laptop_scale), off(&second)],
            Projection::Second => vec![
                off(&laptop),
                at(&second, second_mode, 0, 0, scale_of(&second)),
            ],
            Projection::Extend => vec![
                at(&laptop, laptop_mode, 0, 0, laptop_scale),
                at(
                    &second,
                    second_mode,
                    laptop_size.w.round() as i32,
                    0,
                    scale_of(&second),
                ),
            ],
            Projection::Duplicate => {
                // The second shows the whole of the laptop's screen: its size in points covers
                // it, centred on it.
                let (w, h) = (f64::from(second_mode.size.w), f64::from(second_mode.size.h));
                let fit = (w / laptop_size.w).min(h / laptop_size.h);
                let scale = (fit * 4.0).floor().max(2.0) / 4.0;
                let scale = scale.clamp(0.5, 4.0);
                let size = logical(second_mode, scale);
                let x = ((laptop_size.w - size.w) / 2.0).round() as i32;
                let y = ((laptop_size.h - size.h) / 2.0).round() as i32;
                vec![
                    at(&laptop, laptop_mode, 0, 0, laptop_scale),
                    at(&second, second_mode, x, y, scale),
                ]
            }
        };
        // Any other screen stays as it is.
        wanted.retain(|w| w.output == laptop || w.output == second);
        Some(wanted)
    }

    /// Switches Do not disturb on for a projection, or back off once it ends if a projection
    /// switched it on. Through `notifications.conf`, which the notifications daemon follows.
    fn projection_quiet(&mut self, on: bool) {
        let Some(path) = self.projection.notifications.clone() else {
            return;
        };
        let (mut config, _) =
            notifications_config::NotificationsConfig::load(&path).unwrap_or_default();
        let change = match (on, config.do_not_disturb, self.projection.switched_on) {
            (true, false, _) => Some(true),
            (false, true, true) => Some(false),
            _ => None,
        };
        let Some(value) = change else {
            if !on {
                self.projection.switched_on = false;
            }
            return;
        };
        config.do_not_disturb = value;
        match config.save(&path) {
            Ok(()) => self.projection.switched_on = value,
            Err(err) => tracing::warn!("Do not disturb for the projection: {err}"),
        }
    }

    /// The second screen `output` was plugged in: projected on as it was last time, if it was.
    pub(crate) fn projection_plugged(&mut self, output: &Output) {
        let Some(choice) = self.saved_projection(output) else {
            return;
        };
        let output = output.clone();
        // Not while the screen is being added: the hardware is busy with it.
        self.loop_handle.insert_idle(move |state| {
            if state
                .projection_screens()
                .is_some_and(|(_, second)| second == output)
            {
                state.project(choice, false);
            }
        });
    }

    /// The last screen lit was unplugged while the laptop's was off: it comes back, rather
    /// than leave the machine black.
    pub(crate) fn relight(&mut self) {
        if !self.screens.is_empty() || self.displays.off.is_empty() {
            return;
        }
        self.loop_handle.insert_idle(|state| {
            if !state.screens.is_empty() {
                return;
            }
            let Some(screen) = state.displays.off.first() else {
                return;
            };
            let wanted = Wanted {
                output: screen.output.clone(),
                on: Some(Setting {
                    mode: screen.details.modes.first().copied(),
                    position: Some((0, 0).into()),
                    ..Setting::default()
                }),
            };
            match state.apply_screens(&[wanted]) {
                Ok(()) => tracing::info!("the last screen lit is gone: the laptop's is lit again"),
                Err(err) => tracing::warn!("lighting a screen again: {err}"),
            }
            state.projection_quiet(false);
        });
    }

    /// The panel on `output`, while it is open.
    pub fn projection_layout(&self, output: &Output) -> Option<Layout> {
        self.projection.open?;
        let screen = self.space.output_geometry(output)?.to_f64();
        let (tw, th, gap) = (
            f64::from(ui::TILE_WIDTH),
            f64::from(ui::TILE_HEIGHT),
            f64::from(ui::TILE_GAP),
        );
        let padding = f64::from(ui::PADDING);
        let inner = 4.0 * tw + 3.0 * gap;
        let width = inner + 2.0 * padding;
        let (title, option, between) = (
            f64::from(ui::TITLE_HEIGHT),
            f64::from(ui::OPTION_HEIGHT),
            f64::from(ui::GAP),
        );
        let height = 2.0 * padding + title + between + th + between + option;
        let x = screen.loc.x + (screen.size.w - width) / 2.0;
        let y = screen.loc.y + (screen.size.h - height) / 2.0;
        let panel = Rectangle::new((x, y).into(), (width, height).into());
        let title_rect = Rectangle::new((x + padding, y + padding).into(), (inner, title).into());
        let ty = y + padding + title + between;
        let tiles = std::array::from_fn(|i| {
            let tx = x + padding + i as f64 * (tw + gap);
            Rectangle::new((tx, ty).into(), (tw, th).into())
        });
        let option_rect = Rectangle::new(
            (x + padding, ty + th + between).into(),
            (inner, option).into(),
        );
        Some(Layout {
            panel,
            title: title_rect,
            tiles,
            option: option_rect,
        })
    }
}

/// How far the panel has appeared at `now`, and where the highlight is between two tiles.
fn motion(open: &Open, now: Duration) -> (f64, f64) {
    let ratio = |since: Duration, span: Duration| {
        (now.saturating_sub(since).as_secs_f64() / span.as_secs_f64()).clamp(0.0, 1.0)
    };
    let ease = |t: f64| 1.0 - (1.0 - t).powi(3);
    (
        ease(ratio(open.opened_at, ui::APPEAR)),
        ease(ratio(open.moved_at, ui::SLIDE)),
    )
}

/// The panel on `output`, in front of the desktop, and whether it moves.
pub(crate) fn push_items(
    items: &mut Vec<Item>,
    state: &State,
    (output, view): (&Output, &View),
    now: Duration,
) -> bool {
    let (Some(open), Some(layout)) = (state.projection.open, state.projection_layout(output))
    else {
        return false;
    };
    let colors = Colors::of(color::palette());
    let (appear, slide) = motion(&open, now);
    let grow = f64::from(ui::APPEAR_SCALE) + (1.0 - f64::from(ui::APPEAR_SCALE)) * appear;
    let center = layout.panel.loc + layout.panel.size.downscale(2.0).to_point();
    // Every rectangle grows from the panel's centre while it appears.
    let place = |r: RectF| -> RectF {
        let loc = center + (r.loc - center).upscale(grow);
        Rectangle::new(loc, r.size.upscale(grow))
    };
    let alpha = appear as f32;
    let scale = view.scale as f32;
    let mut part = 0;
    let mut shape = |items: &mut Vec<Item>, r: RectF, fill: color::Color, radius: f32| {
        items.push(Item::Shape {
            key: Key::of(None, Part::Projection(part)),
            rect: view.rect(place(r)),
            fill: fill.premultiplied(),
            border: fill.premultiplied(),
            border_width: 0.0,
            radius: radius * scale,
            alpha,
        });
        part += 1;
    };
    let mut labels = 0;
    let mut text = |items: &mut Vec<Item>, r: RectF, words: &str, bg, fg| {
        let at = view.rect(place(r));
        items.push(Item::Label {
            key: Key::of(None, Part::ProjectionLabel(labels)),
            rect: at,
            label: Label {
                text: words.to_owned(),
                size: (at.size.w.max(1) as u32, at.size.h.max(1) as u32),
                scale,
                background: bg,
                color: fg,
                icon: None,
            },
            alpha,
        });
        labels += 1;
    };
    // Front to back: the texts, the pictures, the highlight, the tiles, the panel.
    let option = if open.do_not_disturb {
        text::DO_NOT_DISTURB_ON
    } else {
        text::DO_NOT_DISTURB_OFF
    };
    text(items, layout.option, option, colors.panel, colors.secondary);
    text(items, layout.title, text::TITLE, colors.panel, colors.text);
    let (pw, ph, pg) = (
        f64::from(ui::PICTURE_WIDTH),
        f64::from(ui::PICTURE_HEIGHT),
        f64::from(ui::PICTURE_GAP),
    );
    for (i, (tile, choice)) in layout.tiles.iter().zip(Projection::ALL).enumerate() {
        let selected = i == open.selected;
        let (bg, fg) = if selected {
            (colors.selected, colors.on_selected)
        } else {
            (colors.tile, colors.text)
        };
        let label_height = f64::from(ui::LABEL_HEIGHT);
        let words = Rectangle::new(
            (
                tile.loc.x + 6.0,
                tile.loc.y + tile.size.h - label_height - 6.0,
            )
                .into(),
            (tile.size.w - 12.0, label_height).into(),
        );
        text(items, words, label(choice), bg, fg);
        // Two screens: lit, off, side by side, or one over the other for « Dupliquer ».
        let top = tile.loc.y + 14.0;
        let left = tile.loc.x + (tile.size.w - 2.0 * pw - pg) / 2.0;
        let (first, second_lit) = match choice {
            Projection::Laptop => (true, false),
            Projection::Second => (false, true),
            _ => (true, true),
        };
        let lit = |on: bool| if on { fg } else { colors.off };
        if choice == Projection::Duplicate {
            let behind = Rectangle::new((left + pg + 6.0, top + 4.0).into(), (pw, ph).into());
            let front = Rectangle::new((left + 6.0, top - 2.0).into(), (pw, ph).into());
            shape(items, front, lit(true), ui::PICTURE_RADIUS);
            shape(items, behind, lit(true), ui::PICTURE_RADIUS);
        } else {
            let a = Rectangle::new((left, top).into(), (pw, ph).into());
            let b = Rectangle::new((left + pw + pg, top).into(), (pw, ph).into());
            shape(items, a, lit(first), ui::PICTURE_RADIUS);
            shape(items, b, lit(second_lit), ui::PICTURE_RADIUS);
        }
        if !selected {
            shape(items, *tile, colors.tile, ui::TILE_RADIUS);
        }
    }
    // The highlight slides from the tile it was on.
    let from = layout.tiles[open.previous];
    let to = layout.tiles[open.selected];
    let x = from.loc.x + (to.loc.x - from.loc.x) * slide;
    let highlight = Rectangle::new((x, to.loc.y).into(), to.size);
    shape(items, highlight, colors.selected, ui::TILE_RADIUS);
    shape(items, layout.panel, colors.panel, ui::RADIUS);
    appear < 1.0 || slide < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_appears_and_the_highlight_slides() {
        let open = Open {
            selected: 1,
            previous: 0,
            opened_at: Duration::ZERO,
            moved_at: Duration::from_millis(100),
            do_not_disturb: true,
        };
        assert_eq!(motion(&open, Duration::ZERO).0, 0.0);
        assert_eq!(motion(&open, ui::APPEAR * 2).0, 1.0);
        let (_, half) = motion(&open, Duration::from_millis(100) + ui::SLIDE / 2);
        assert!(half > 0.5 && half < 1.0, "{half}");
        assert_eq!(label(Projection::Extend), text::EXTEND);
    }
}
