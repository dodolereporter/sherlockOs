//! Mission Control (lot D-33): every window of the active workspace side by side, under the
//! strip of the workspaces.
//!
//! - Opens with three fingers up (the view follows the fingers), the gesture key alone, or the
//!   gesture key + Tab; closes with three fingers down, Escape, the same keys, or a click on
//!   nothing.
//! - A click on a window gives it the keyboard and closes the view; on a thumbnail, goes to
//!   that workspace. A window dragged onto a thumbnail moves to that workspace.
//! - While it is open, no client gets a click nor a key: the keyboard is taken from them and
//!   the pointer is over none of them.
//! - The windows are drawn by `push_items` in place of the desktop: their real surfaces,
//!   scaled by the renderer, between their place and their slot of the grid (`layout`).
//!
//! See "Mission Control" in `docs/compositor.md`.

mod label;
pub mod layout;

use std::time::Duration;

use sherlock_ui::{
    color::{self, Color},
    decorations::{self as ui_decorations, Colors as BarColors},
    metrics,
    mission_control::{self as mc, Colors},
    motion::{self, Animation},
    strings, window as ui_window,
};
use smithay::{
    desktop::Window,
    input::keyboard::Keysym,
    output::Output,
    reexports::wayland_server::Resource as _,
    utils::{IsAlive as _, Logical, Point, Rectangle},
    wayland::seat::WaylandFocus as _,
};

pub use label::{Label, Labels, width as label_width};
pub use layout::RectF;

use crate::{
    State,
    damage::{Key, Part},
    decorations::{self, Look},
    is_override_redirect,
    output::View,
    render::{Item, Shadow, corner_radius},
    window_data,
};

/// Openings per second past which a flick of the fingers decides, whatever the distance.
const FLICK: f64 = 2.0;

/// Half a pixel on a screen 4096 wide, as a share of the opening.
const PRECISION: f64 = motion::GEOMETRY_PRECISION / 4096.0;
const NO_FILL: [f32; 4] = [0.0; 4];

/// What the pointer can be on in the view.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    Window(Window),
    /// A thumbnail of the strip, by the index of its workspace.
    Workspace(usize),
    /// The « × » of a window under the pointer, which closes it (lot D-33c).
    Close(Window),
    /// The « + » after the thumbnails, and the « × » of the thumbnail under the pointer: add a
    /// workspace, remove that one (lot D-33c).
    AddWorkspace,
    RemoveWorkspace(usize),
}

impl Target {
    /// The window it is on, its « × » included.
    pub fn window(&self) -> Option<&Window> {
        match self {
            Self::Window(window) | Self::Close(window) => Some(window),
            Self::Workspace(_) | Self::AddWorkspace | Self::RemoveWorkspace(_) => None,
        }
    }
}

/// The title and the « × » of a window fade in when the pointer comes on it, and out when it
/// leaves (lot D-33c): `mc::HOVER_FADE`, linear, from where it was.
#[derive(Debug, Clone)]
struct TitleFade {
    window: Window,
    from: f32,
    shown: bool,
    start: Duration,
}

impl TitleFade {
    fn value(&self, now: Duration) -> f32 {
        let t = now.saturating_sub(self.start).as_secs_f32() / mc::HOVER_FADE.as_secs_f32();
        let to = if self.shown { 1.0 } else { 0.0 };
        self.from + (to - self.from) * t.clamp(0.0, 1.0)
    }

    fn running(&self, now: Duration) -> bool {
        now < self.start + mc::HOVER_FADE
    }
}

/// A press of the left button in the view.
#[derive(Debug)]
struct Press {
    target: Option<Target>,
    start: Point<f64, Logical>,
    at: Point<f64, Logical>,
    /// A window pressed and moved past the drag threshold: it follows the pointer.
    dragging: bool,
}

/// The state of the view: open or not, how far, and what the pointer does in it.
#[derive(Debug, Default)]
pub struct Overview {
    open: bool,
    /// Progress from 0 (the desktop) to 1 (the grid), on `mc::SPRING`.
    motion: Option<Animation>,
    /// Progress held by three fingers on the touchpad.
    held: Option<f64>,
    /// Progress when the three finger swipe in progress started.
    swipe: Option<f64>,
    hovered: Option<Target>,
    /// The titles shown or fading, one per window the pointer was on.
    titles: Vec<TitleFade>,
    /// The workspace just added by its « + », by index, and when: its thumbnail fades in.
    arrived: Option<(usize, Duration)>,
    press: Option<Press>,
    /// App Expose: only the windows of these ids, without the strip of the workspaces. Kept
    /// while the view closes, so that they go back the way they came.
    only: Option<Vec<u32>>,
    /// The view was closed by a key while fingers held it: the rest of that swipe does nothing.
    swipe_closed: bool,
    /// The gesture key is down and nothing else happened since: its release toggles the view.
    tap: bool,
    toggle: bool,
}

impl Overview {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// From 0 (the desktop) to 1 (the grid) at `now`, a little past 1 with the spring.
    fn progress(&self, now: Duration) -> f64 {
        match (self.held, self.motion) {
            (Some(held), _) => held,
            (None, Some(m)) if !m.is_done(now, PRECISION) => m.value(now),
            // Where the last motion went: a view marked open by `enter_overview` has not left
            // the desktop yet.
            (None, Some(m)) => m.target(),
            (None, None) => 0.0,
        }
    }

    fn animating(&self, now: Duration) -> bool {
        self.held.is_none() && self.motion.is_some_and(|m| !m.is_done(now, PRECISION))
    }

    /// A title is fading in or out at `now`, or a new thumbnail.
    fn titles_fading(&self, now: Duration) -> bool {
        let arriving = self.arrived.is_some_and(|(_, at)| now < at + mc::ARRIVAL);
        arriving || self.titles.iter().any(|t| t.running(now))
    }

    /// How much the thumbnail `index` has faded in at `now`: 1 but for a new one.
    fn arrival(&self, index: usize, now: Duration) -> f32 {
        match self.arrived {
            Some((i, at)) if i == index => {
                let t = now.saturating_sub(at).as_secs_f32() / mc::ARRIVAL.as_secs_f32();
                t.clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    }

    /// How much of the title and the « × » of `window` is shown at `now`.
    fn title(&self, window: &Window, now: Duration) -> f32 {
        let fade = self.titles.iter().find(|t| &t.window == window);
        fade.map_or(0.0, |t| t.value(now))
    }

    /// The pointer, or the keyboard, is now on `target`: the title of its window fades in, the
    /// one it left fades out.
    fn hover(&mut self, target: Option<Target>, now: Duration) {
        let window = target.as_ref().and_then(Target::window).cloned();
        self.hovered = target;
        self.titles
            .retain(|t| t.window.alive() && (t.shown || t.running(now)));
        for fade in &mut self.titles {
            let shown = Some(&fade.window) == window.as_ref();
            if fade.shown != shown {
                *fade = TitleFade {
                    from: fade.value(now),
                    shown,
                    start: now,
                    window: fade.window.clone(),
                };
            }
        }
        if let Some(window) = window
            && !self.titles.iter().any(|t| t.window == window)
        {
            self.titles.push(TitleFade {
                window,
                from: 0.0,
                shown: true,
                start: now,
            });
        }
    }

    fn shown(&self, now: Duration) -> bool {
        self.open || self.held.is_some() || self.animating(now)
    }

    /// Leaves from where the fingers let it, towards `to`, at their `speed` (openings per
    /// second): no break between the fingers and the spring (lot D-33c).
    fn fling(&mut self, now: Duration, to: f64, speed: f64) {
        let from = self.progress(now);
        self.held = None;
        self.motion = Some(Animation::with_velocity(mc::SPRING, from, to, speed, now));
    }

    /// Leaves from where the view is, towards `to`, without a jolt.
    fn animate(&mut self, now: Duration, to: f64) {
        let from = self.progress(now);
        self.motion = match (self.held.take(), self.motion) {
            (None, Some(mut m)) if !m.is_done(now, PRECISION) => {
                m.retarget(now, to);
                Some(m)
            }
            _ => Some(Animation::new(mc::SPRING, from, to, now)),
        };
    }

    /// A key went down or up: `gesture` is the gesture key, `alone` with no other modifier. The
    /// gesture key released with nothing in between toggles the view (`take_toggle`).
    pub(crate) fn key(&mut self, gesture: bool, pressed: bool, alone: bool) {
        match (gesture, pressed) {
            (true, true) => self.tap = alone,
            (true, false) => self.toggle |= std::mem::take(&mut self.tap),
            (false, true) => self.tap = false,
            (false, false) => {}
        }
    }

    /// A button or a scroll: the gesture key is no longer alone.
    pub(crate) fn cancel_tap(&mut self) {
        self.tap = false;
    }

    pub(crate) fn take_toggle(&mut self) -> bool {
        std::mem::take(&mut self.toggle)
    }
}

/// The gesture key itself: Super in a real session, Alt in nested mode.
pub(crate) fn is_gesture_key(nested: bool, sym: Keysym) -> bool {
    if nested {
        matches!(sym, Keysym::Alt_L | Keysym::Alt_R)
    } else {
        matches!(sym, Keysym::Super_L | Keysym::Super_R)
    }
}

/// A window shown by the view.
#[derive(Debug, Clone)]
pub struct Tile {
    pub window: Window,
    /// The whole window, title bar included, where it is drawn on the desktop: what it leaves
    /// from and comes back to.
    pub home: RectF,
    /// Its place in the grid.
    pub slot: RectF,
    /// Its opacity and decoration on the desktop (opening, maximized).
    pub opacity: f32,
    pub decoration: f32,
}

fn lerp(a: RectF, b: RectF, t: f64) -> RectF {
    let at = |a: f64, b: f64| a + (b - a) * t;
    Rectangle::new(
        (at(a.loc.x, b.loc.x), at(a.loc.y, b.loc.y)).into(),
        (at(a.size.w, b.size.w), at(a.size.h, b.size.h)).into(),
    )
}

/// Where the « × » of a window drawn in `frame` is centered: inside its top left corner.
fn close_center(frame: RectF) -> (f64, f64) {
    let offset = f64::from(mc::CLOSE_INSET);
    (frame.loc.x + offset, frame.loc.y + offset)
}

fn center(r: RectF) -> (f64, f64) {
    (r.loc.x + r.size.w / 2.0, r.loc.y + r.size.h / 2.0)
}

/// The choice nearest `from` in `direction` (a unit step on one axis): ahead of it, the
/// distance along the way counting less than the distance across it.
fn nearest(choices: &[(Target, RectF)], from: (f64, f64), direction: (f64, f64)) -> Option<Target> {
    let score = |r: &RectF| {
        let (x, y) = center(*r);
        let (dx, dy) = (x - from.0, y - from.1);
        let along = dx * direction.0 + dy * direction.1;
        let across = (dx * direction.1 - dy * direction.0).abs();
        (along > 1.0).then_some(along + 2.0 * across)
    };
    choices
        .iter()
        .filter_map(|(t, r)| Some((t, score(r)?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t.clone())
}

/// Height the strip takes at the top of the work area: thumbnails, their names and margins.
fn strip_height() -> f64 {
    f64::from(2.0 * mc::STRIP_MARGIN + mc::STRIP_HEIGHT + mc::LABEL_GAP + mc::LABEL_HEIGHT)
}

/// Rectangle of a label of `width` centered under `rect`.
fn label_rect(rect: RectF, width: f64) -> RectF {
    let width = width
        .min(rect.size.w)
        .max(f64::from(mc::LABEL_RADIUS) * 2.0);
    let x = rect.loc.x + (rect.size.w - width) / 2.0;
    let y = rect.loc.y + rect.size.h + f64::from(mc::LABEL_GAP);
    Rectangle::new((x, y).into(), (width, f64::from(mc::LABEL_HEIGHT)).into())
}

impl State {
    pub fn overview_open(&self) -> bool {
        self.overview.open
    }

    /// The view is drawn at `now`: open, held by the fingers, or on its way back.
    pub fn overview_shown(&self, now: Duration) -> bool {
        self.overview.shown(now)
    }

    /// The view is moving at `now`: a frame is due at every vertical blank.
    pub fn overview_animating(&self, now: Duration) -> bool {
        self.overview.animating(now)
    }

    /// How far the view is open at `now`, from 0 to 1.
    pub fn overview_progress(&self, now: Duration) -> f64 {
        self.overview.progress(now)
    }

    pub fn toggle_overview(&mut self) {
        if self.overview.open {
            self.close_overview();
        } else {
            self.open_overview();
        }
    }

    /// Opens the view, unless a window is being moved or a surface keeps the keyboard (lock).
    pub fn open_overview(&mut self) {
        if self.overview.open {
            return;
        }
        self.overview.only = None;
        if self.enter_overview() {
            self.overview.animate(self.now(), 1.0);
        }
    }

    /// App Expose (lot D-21c2): the view with only the windows of `ids` that it would show,
    /// without the strip of the workspaces. Nothing when none of them is shown; while the
    /// view is open, it shows these instead.
    pub fn show_windows(&mut self, ids: Vec<u32>) {
        let shown = ids.iter().any(|id| {
            self.window_by_id(*id)
                .is_some_and(|w| self.is_visible(&w) && !is_override_redirect(&w))
        });
        if !shown {
            tracing::debug!(?ids, "app expose: no window to show");
            return;
        }
        let was = self.overview.only.replace(ids);
        if self.overview.open {
            self.refresh_overview_pointer();
        } else if self.enter_overview() {
            self.overview.animate(self.now(), 1.0);
        } else {
            self.overview.only = was;
        }
    }

    /// What the pointer is over in the open view, or the keyboard selected.
    pub fn overview_selected(&self) -> Option<&Target> {
        self.overview.hovered.as_ref()
    }

    /// The window ids App Expose shows, if the view is App Expose.
    pub fn overview_only(&self) -> Option<&[u32]> {
        self.overview.only.as_deref()
    }

    /// Closes the view: the windows go back to their place, the keyboard to the window that
    /// had it last.
    pub fn close_overview(&mut self) {
        if self.overview.open {
            self.leave_overview(None);
        }
    }

    /// Marks the view open and takes the keyboard and the pointer from the clients. `false` if
    /// it cannot open, or already is.
    fn enter_overview(&mut self) -> bool {
        if self.overview.open
            || self.gesture_active()
            || self.exclusive_layer().is_some()
            || self.screens.is_empty()
        {
            return false;
        }
        tracing::debug!("mission control opens");
        self.overview.open = true;
        self.dismiss_popups();
        self.hover_decoration(None);
        self.forget_decoration_press();
        self.set_keyboard_focus(None);
        self.refresh_overview_pointer();
        true
    }

    /// Closes the view; `chosen` gets the keyboard and comes on top, or else the window that
    /// had it last.
    fn leave_overview(&mut self, chosen: Option<Window>) {
        tracing::debug!(?chosen, "mission control closes");
        let now = self.now();
        let overview = &mut self.overview;
        overview.open = false;
        overview.hover(None, now);
        overview.press = None;
        // Closed by a key while the fingers held it: they no longer move it.
        overview.swipe_closed |= overview.swipe.is_some() && overview.held.is_some();
        overview.animate(now, 0.0);
        match chosen {
            Some(window) => self.activate(&window),
            None => self.focus(self.top_window()),
        }
        self.refresh_overview_pointer();
    }

    /// The pointer goes to what is under it: nothing while the view is open.
    fn refresh_overview_pointer(&mut self) {
        self.queue_redraw();
        if let Some(pointer) = self.seat.get_pointer() {
            let time = self.now().as_millis() as u32;
            self.pointer_motion(pointer.current_location(), time);
        }
    }

    /// The windows the view shows on `output`, from the lowest to the topmost: those of the
    /// active workspace, not minimized, whose center is on that screen. With `now`, they leave
    /// from where they are drawn at that time; without, from their real place.
    pub fn overview_tiles(&self, output: &Output, now: Option<Duration>) -> Vec<Tile> {
        let Some(work) = self.work_area(output) else {
            return Vec::new();
        };
        let mut tiles: Vec<Tile> = self
            .space
            .elements()
            .filter(|w| self.is_visible(w) && !is_override_redirect(w))
            .filter(|w| self.window_output(w).as_ref() == Some(output))
            .filter(|w| {
                let only = self.overview.only.as_ref();
                only.is_none_or(|ids| ids.contains(&crate::shell::window_id(w)))
            })
            .filter_map(|window| {
                let real = decorations::frame(window, self.space.element_geometry(window)?);
                let real = real.to_f64();
                let (home, opacity, decoration) = match now {
                    Some(now) => {
                        let visual = self.window_visual(window, now)?;
                        let inset = f64::from(decorations::inset(window));
                        let mut home = visual.rect;
                        home.loc.y -= inset;
                        home.size.h += inset;
                        (home, visual.opacity, visual.decoration)
                    }
                    None => (real, 1.0, 1.0),
                };
                let slot = real;
                let window = window.clone();
                Some(Tile {
                    window,
                    home,
                    slot,
                    opacity,
                    decoration,
                })
            })
            .collect();
        let mut area = work.to_f64();
        // App Expose has no strip: the windows take its room.
        let margin = f64::from(mc::MARGIN);
        let top = match self.overview.only {
            Some(_) => margin,
            None => strip_height(),
        };
        area.loc.y += top;
        area.size.h -= top + margin;
        area.loc.x += margin;
        area.size.w -= 2.0 * margin;
        let spacing = layout::Spacing {
            gap: f64::from(mc::GAP),
            label: f64::from(mc::LABEL_GAP + mc::LABEL_HEIGHT),
            max_scale: mc::MAX_SCALE,
        };
        let frames: Vec<RectF> = tiles.iter().map(|t| t.slot).collect();
        for (tile, slot) in tiles
            .iter_mut()
            .zip(layout::arrange(&frames, area, spacing))
        {
            tile.slot = slot;
        }
        tiles
    }

    /// The thumbnails of the workspaces on `output`, in their order, once the view is open:
    /// centered at the top of the work area, as wide as the screen is for their height.
    pub fn overview_strip(&self, output: &Output) -> Vec<RectF> {
        if self.overview.only.is_some() {
            return Vec::new();
        }
        let (Some(work), Some(screen)) =
            (self.work_area(output), self.space.output_geometry(output))
        else {
            return Vec::new();
        };
        let n = self.workspaces().list().len() as f64;
        let height = f64::from(mc::STRIP_HEIGHT);
        let width = height * f64::from(screen.size.w) / f64::from(screen.size.h.max(1));
        let gap = f64::from(mc::THUMBNAIL_GAP);
        let total = n * width + (n - 1.0) * gap;
        let room = f64::from(work.size.w) - 2.0 * f64::from(mc::MARGIN);
        let k = (room / total).min(1.0);
        let (width, height, gap) = (width * k, height * k, gap * k);
        let x = f64::from(work.loc.x) + (f64::from(work.size.w) - total * k) / 2.0;
        let y = f64::from(work.loc.y) + f64::from(mc::STRIP_MARGIN);
        (0..n as usize)
            .map(|i| {
                let x = x + i as f64 * (width + gap);
                Rectangle::new((x, y).into(), (width, height).into())
            })
            .collect()
    }

    /// The « + » after the thumbnails of `output`: none past 9 workspaces, nor in App Expose.
    pub fn overview_add_button(&self, output: &Output) -> Option<RectF> {
        if self.workspaces().list().len() >= crate::shell::MAX_WORKSPACES {
            return None;
        }
        let last = *self.overview_strip(output).last()?;
        let side = f64::from(mc::ADD_SIZE);
        let x = last.loc.x + last.size.w + f64::from(mc::ADD_GAP);
        let y = last.loc.y + (last.size.h - side) / 2.0;
        Some(Rectangle::new((x, y).into(), (side, side).into()))
    }

    /// What is at `at` in the open view: a thumbnail, or a window in its slot.
    pub fn overview_target(&self, at: Point<f64, Logical>) -> Option<Target> {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        // The « × » of the window under the pointer, over its corner.
        let shown = self.overview.hovered.as_ref().and_then(Target::window);
        if let Some(window) = shown.filter(|_| self.overview.press.is_none()) {
            let slot = outputs.iter().find_map(|o| {
                let tiles = self.overview_tiles(o, None);
                tiles
                    .into_iter()
                    .find(|t| &t.window == window)
                    .map(|t| t.slot)
            });
            if let Some(slot) = slot {
                let (x, y) = close_center(slot);
                if (at.x - x).hypot(at.y - y) <= f64::from(mc::CLOSE_SIZE) / 2.0 {
                    return Some(Target::Close(window.clone()));
                }
            }
        }
        for output in &outputs {
            let strip = self.overview_strip(output);
            // The « × » of the thumbnail under the pointer, when another workspace remains.
            let hovered = match self.overview.hovered {
                Some(Target::Workspace(i) | Target::RemoveWorkspace(i)) => Some(i),
                _ => None,
            };
            let thumb = hovered.and_then(|i| Some((i, *strip.get(i)?)));
            if let Some((index, thumb)) = thumb.filter(|_| strip.len() > 1) {
                let (x, y) = close_center(thumb);
                if (at.x - x).hypot(at.y - y) <= f64::from(mc::CLOSE_SIZE) / 2.0 {
                    return Some(Target::RemoveWorkspace(index));
                }
            }
            if let Some(index) = strip.iter().position(|r| r.contains(at)) {
                return Some(Target::Workspace(index));
            }
            if self
                .overview_add_button(output)
                .is_some_and(|r| r.contains(at))
            {
                return Some(Target::AddWorkspace);
            }
        }
        outputs.iter().find_map(|output| {
            let tiles = self.overview_tiles(output, None);
            let tile = tiles.into_iter().rev().find(|t| t.slot.contains(at))?;
            Some(Target::Window(tile.window))
        })
    }

    /// Ctrl + Down (lot D-33c): App Expose for the application in front, its windows of the
    /// active workspace; closes the view when it is open.
    pub fn toggle_app_expose(&mut self) {
        if self.overview.open {
            self.close_overview();
            return;
        }
        let Some(front) = self.focused_window() else {
            return;
        };
        let app = crate::shell::app_id(&front);
        let ids = self
            .space
            .elements()
            .filter(|w| crate::shell::app_id(w) == app)
            .map(crate::shell::window_id)
            .collect();
        self.show_windows(ids);
    }

    /// Everything the keyboard can select in the open view, on every screen: the thumbnails
    /// of the strip, then the windows.
    fn overview_choices(&self) -> Vec<(Target, RectF)> {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let strip = outputs.iter().flat_map(|o| {
            let thumbs = self.overview_strip(o).into_iter().enumerate();
            thumbs.map(|(i, r)| (Target::Workspace(i), r))
        });
        let tiles = outputs.iter().flat_map(|o| {
            let tiles = self.overview_tiles(o, None).into_iter();
            tiles.map(|t| (Target::Window(t.window), t.slot))
        });
        strip.chain(tiles).collect()
    }

    /// An arrow, Enter or Return in the open view (lot D-33c). An arrow moves the selection
    /// (drawn as under the pointer) to the nearest choice that way; the first arrow selects
    /// the window in front. Enter chooses it, as a click would.
    pub(crate) fn overview_key(&mut self, sym: Keysym) {
        self.queue_redraw();
        let choices = self.overview_choices();
        // The « × » under the pointer stands for its window.
        let current = (self.overview.hovered.as_ref()).map(|t| match t {
            Target::Close(window) => Target::Window(window.clone()),
            Target::RemoveWorkspace(index) => Target::Workspace(*index),
            other => other.clone(),
        });
        let at = current
            .as_ref()
            .and_then(|c| choices.iter().find(|(t, _)| t == c))
            .map(|(_, r)| center(*r));
        let direction = match sym {
            Keysym::Left => (-1.0, 0.0),
            Keysym::Right => (1.0, 0.0),
            Keysym::Up => (0.0, -1.0),
            Keysym::Down => (0.0, 1.0),
            _ => {
                match current {
                    Some(Target::Window(window) | Target::Close(window)) => {
                        self.leave_overview(Some(window));
                    }
                    Some(Target::Workspace(index)) => {
                        if let Some(id) = self.workspaces().id_at(index).map(str::to_owned) {
                            self.activate_workspace(&id);
                            self.stop_workspace_slide();
                        }
                        self.leave_overview(None);
                    }
                    Some(Target::AddWorkspace | Target::RemoveWorkspace(_)) | None => {}
                }
                return;
            }
        };
        let Some(at) = at else {
            // Nothing selected yet: the window in front, or the first choice.
            let front = self.top_window().map(Target::Window);
            let first = choices.iter().find(|(t, _)| matches!(t, Target::Window(_)));
            let chosen = front
                .filter(|f| choices.iter().any(|(t, _)| t == f))
                .or_else(|| first.or(choices.first()).map(|(t, _)| t.clone()));
            self.overview.hover(chosen, self.now());
            return;
        };
        if let Some(next) = nearest(&choices, at, direction) {
            self.overview.hover(Some(next), self.now());
        }
    }

    /// The pointer moved to `at` in the open view: what it is over, and the window it drags.
    pub(crate) fn overview_motion(&mut self, at: Point<f64, Logical>) {
        self.hover_decoration(None);
        if let Some(press) = &mut self.overview.press {
            press.at = at;
            let delta = at - press.start;
            let far = delta.x.hypot(delta.y) >= f64::from(ui_window::DRAG_THRESHOLD);
            press.dragging |= far && matches!(press.target, Some(Target::Window(_)));
        }
        let target = self.overview_target(at);
        if target != self.overview.hovered {
            self.queue_redraw();
            self.overview.hover(target, self.now());
        }
    }

    /// A press or release of `button` at `at` in the open view. A click on a window chooses it,
    /// on a thumbnail goes to that workspace, on nothing closes the view; a window dropped on a
    /// thumbnail moves to that workspace. The clients see none of it.
    pub(crate) fn overview_button(&mut self, button: u32, pressed: bool, at: Point<f64, Logical>) {
        if button != crate::input::BTN_LEFT {
            return;
        }
        if pressed {
            let target = self.overview_target(at);
            let start = at;
            self.overview.press = Some(Press {
                target,
                start,
                at,
                dragging: false,
            });
            return;
        }
        let Some(press) = self.overview.press.take() else {
            return;
        };
        let target = self.overview_target(at);
        match (press.target, press.dragging, target) {
            (Some(Target::Window(window)), true, Some(Target::Workspace(index))) => {
                let id = self.workspaces().id_at(index).map(str::to_owned);
                if let Some(id) = id {
                    self.move_to_workspace(&window, &id);
                }
            }
            (Some(Target::Window(window)), false, Some(Target::Window(over))) if over == window => {
                self.leave_overview(Some(window));
            }
            (Some(Target::Workspace(index)), _, Some(Target::Workspace(over))) if over == index => {
                if let Some(id) = self.workspaces().id_at(index).map(str::to_owned) {
                    self.activate_workspace(&id);
                    self.stop_workspace_slide();
                }
                self.leave_overview(None);
            }
            // The « × »: the window is asked to close; the view stays open, and the window
            // leaves the grid once gone.
            (Some(Target::Close(window)), _, Some(Target::Close(over))) if over == window => {
                State::close(&window);
            }
            (Some(Target::AddWorkspace), _, Some(Target::AddWorkspace)) => {
                if self.add_workspace() {
                    let index = self.workspaces().list().len() - 1;
                    self.overview.arrived = Some((index, self.now()));
                }
            }
            (Some(Target::RemoveWorkspace(index)), _, Some(Target::RemoveWorkspace(over)))
                if over == index =>
            {
                self.remove_workspace(index);
            }
            (None, _, None) => self.leave_overview(None),
            _ => {}
        }
        if self.overview.open {
            let target = self.overview_target(at);
            self.overview.hover(target, self.now());
        }
    }

    /// Three fingers moved `travel` units up (negative) or down since the swipe started: the
    /// view follows them, from where it was, a whole opening per `OVERVIEW_DISTANCE`. Down on
    /// the desktop opens nothing.
    pub(crate) fn follow_overview(&mut self, travel: f64) {
        if self.overview.swipe_closed {
            return;
        }
        let start = if self.overview.open { 1.0 } else { 0.0 };
        let base = *self.overview.swipe.get_or_insert(start);
        let progress = (base - travel / crate::gestures::OVERVIEW_DISTANCE).clamp(0.0, 1.0);
        if !self.overview.open && progress > 0.0 && !self.enter_overview() {
            return;
        }
        if self.overview.open {
            self.queue_redraw();
            self.overview.held = Some(progress);
        }
    }

    /// The three fingers lifted, moving at `velocity` (units per second, down positive): the
    /// view goes where that velocity carries it, open past half way, closed short of it, and
    /// leaves at the fingers' speed, without a break (lot D-33c). A quick short flick is
    /// enough; fingers that came back past their start close it, whatever direction they
    /// first took. Cancelled by libinput, it goes back to where it was. `false` if the swipe
    /// did not move the view.
    pub(crate) fn release_overview(&mut self, cancelled: bool, velocity: f64) -> bool {
        let base = self.overview.swipe.take();
        if std::mem::take(&mut self.overview.swipe_closed) {
            return true;
        }
        let (Some(base), Some(held)) = (base, self.overview.held) else {
            return false;
        };
        // In openings per second, up opening.
        let speed = -velocity / crate::gestures::OVERVIEW_DISTANCE;
        let projected = held + speed * crate::gestures::PROJECTION;
        let open = match cancelled {
            true => base >= 0.5,
            false if speed.abs() >= FLICK => speed > 0.0,
            false => projected >= 0.5,
        };
        let now = self.now();
        if open {
            self.overview.fling(now, 1.0, speed);
        } else {
            self.leave_overview(None);
            self.overview.fling(now, 0.0, speed);
        }
        true
    }
}

/// The items of Mission Control on the screen `output`, in place of the windows, from front to
/// back, and whether something moves; `None` when the view is not shown. The dragged window,
/// the strip of the workspaces, the windows from the topmost down, then the dim over the
/// wallpaper.
pub(crate) fn push_items(
    items: &mut Vec<Item>,
    state: &State,
    (output, view): (&Output, &View),
    now: Duration,
) -> Option<bool> {
    let overview = &state.overview;
    if !overview.shown(now) {
        return None;
    }
    let progress = overview.progress(now);
    let fade = progress.clamp(0.0, 1.0) as f32;
    let colors = Colors::of(color::palette());
    let mut animating = overview.animating(now) || overview.titles_fading(now);
    let tiles = state.overview_tiles(output, Some(now));
    let dragged = overview
        .press
        .as_ref()
        .and_then(|press| match &press.target {
            Some(Target::Window(window)) if press.dragging => {
                Some((window, press.at - press.start))
            }
            _ => None,
        });
    let mut pushed = Vec::new();
    for tile in tiles.iter().rev() {
        animating |= window_data(&tile.window).motion.borrow().is_animating();
        let mut frame = lerp(tile.home, tile.slot, progress);
        let drag = dragged.filter(|(w, _)| **w == tile.window);
        if let Some((_, delta)) = drag {
            frame.loc += delta;
        }
        let hovered = overview.hovered.as_ref().and_then(Target::window) == Some(&tile.window);
        let title = overview.title(&tile.window, now);
        let close = overview.hovered == Some(Target::Close(tile.window.clone()));
        let look = TileLook {
            frame,
            slot: tile.slot,
            highlight: hovered && dragged.is_none(),
            // Titles on hover only, as in Sequoia (lot D-33c).
            label: if drag.is_some() || dragged.is_some() {
                0.0
            } else {
                fade * title
            },
            close,
            opacity: tile.opacity,
            decoration: tile.decoration + (1.0 - tile.decoration) * fade,
        };
        let mut own = Vec::new();
        push_tile(&mut own, state, tile, look, view, colors);
        // The dragged window goes in front of everything, the strip included.
        if drag.is_some() {
            items.append(&mut own);
        } else {
            pushed.append(&mut own);
        }
    }
    let target = dragged.and(overview.hovered.clone());
    let hovered = dragged
        .is_none()
        .then(|| overview.hovered.clone())
        .flatten();
    push_strip(
        items,
        state,
        (output, view),
        (progress, target, hovered, now),
        colors,
    );
    items.append(&mut pushed);
    if let Some(screen) = state.space.output_geometry(output) {
        let dim = colors.dim.premultiplied();
        items.push(Item::Shape {
            key: Key::of(None, Part::Dim),
            rect: view.rect(screen.to_f64()),
            fill: dim,
            border: dim,
            border_width: 0.0,
            radius: 0.0,
            alpha: fade,
        });
    }
    Some(animating)
}

/// How a window is drawn in the view at a given time.
#[derive(Debug, Clone, Copy)]
struct TileLook {
    /// The whole window, title bar included, in the space.
    frame: RectF,
    /// Its place in the grid once open: its title is drawn at the size it has there, so that
    /// the texture is made once, not at every frame of the animation (lot D-33c).
    slot: RectF,
    highlight: bool,
    /// Opacity of its title below it, and of its « × ».
    label: f32,
    /// The pointer is on its « × ».
    close: bool,
    opacity: f32,
    decoration: f32,
}

/// A window of the view: its title below, its accent border under the pointer, its border, its
/// title bar, its content and its shadow, all scaled with it. The content is the client's own
/// surface, stretched by the renderer; the title bar texture is the one of the desktop.
fn push_tile(
    items: &mut Vec<Item>,
    state: &State,
    tile: &Tile,
    look: TileLook,
    view: &View,
    colors: Colors,
) {
    let window = &tile.window;
    let (Some(surface), geometry) = (window.wl_surface(), window.geometry()) else {
        return;
    };
    let real = decorations::frame(window, Rectangle::from_size(geometry.size));
    if real.size.is_empty() || geometry.size.is_empty() {
        return;
    }
    let owner = Some(surface.id());
    let frame = look.frame;
    let zoom = frame.size.w / f64::from(real.size.w);
    let scale = view.scale as f32;
    let rect = view.rect(frame);
    let radius = corner_radius(real.size) * scale * zoom as f32 * look.decoration;
    let title = State::title(window).unwrap_or_default();
    if look.label > 0.0 {
        let side = f64::from(mc::CLOSE_SIZE);
        let (x, y) = close_center(frame);
        let at = view.rect(Rectangle::new(
            (x - side / 2.0, y - side / 2.0).into(),
            (side, side).into(),
        ));
        let pixels = (side * view.scale).round().max(1.0) as u32;
        // Under the pointer, the red of the close buttons of the title bars.
        let background = match look.close {
            true => BarColors::of(color::palette()).close_hover,
            false => colors.label,
        };
        items.push(Item::Label {
            key: Key::of(owner.clone(), Part::CloseButton),
            rect: at,
            label: Label {
                text: String::new(),
                size: (pixels, pixels),
                scale,
                background,
                color: colors.label_text,
                icon: Some(ui_decorations::Button::Close.icon(false)),
            },
            alpha: look.label,
        });
    }
    if look.label > 0.0 && !title.is_empty() {
        let width = f64::from(label::width(&title))
            .min(look.slot.size.w)
            .max(f64::from(mc::LABEL_RADIUS) * 2.0);
        let x = frame.loc.x + (frame.size.w - width) / 2.0;
        let y = frame.loc.y + frame.size.h + f64::from(mc::LABEL_GAP);
        let height = f64::from(mc::LABEL_HEIGHT);
        let at = view.rect(Rectangle::new((x, y).into(), (width, height).into()));
        let label = Label {
            text: title,
            size: (
                (width * view.scale).round().max(1.0) as u32,
                (height * view.scale).round().max(1.0) as u32,
            ),
            scale,
            background: colors.label,
            color: colors.label_text,
            icon: None,
        };
        items.push(Item::Label {
            key: Key::of(owner.clone(), Part::Label),
            rect: at,
            label,
            alpha: look.label,
        });
    }
    if look.highlight {
        let width = f64::from(mc::HIGHLIGHT_WIDTH);
        let mut outer = frame;
        outer.loc -= Point::from((width, width));
        outer.size += (2.0 * width, 2.0 * width).into();
        items.push(Item::Shape {
            key: Key::of(owner.clone(), Part::Highlight),
            rect: view.rect(outer),
            fill: NO_FILL,
            border: colors.highlight.premultiplied(),
            border_width: mc::HIGHLIGHT_WIDTH * scale,
            radius: radius + mc::HIGHLIGHT_WIDTH * scale,
            alpha: look.opacity,
        });
    }
    items.push(Item::Shape {
        key: Key::of(owner.clone(), Part::Border),
        rect,
        fill: NO_FILL,
        border: color::palette().border.premultiplied(),
        border_width: metrics::BORDER_WIDTH * scale,
        radius,
        alpha: look.opacity * look.decoration,
    });
    let inset = f64::from(decorations::inset(window)) * zoom;
    if let Some(bar) = state.decoration_bar(window) {
        // The texture of the desktop, at the window's own size, stretched by the renderer.
        let size = (
            (f64::from(real.size.w) * view.scale).round().max(1.0) as u32,
            (f64::from(ui_decorations::INSET) * view.scale)
                .round()
                .max(1.0) as u32,
        );
        let look_bar = Look {
            bar,
            size,
            scale,
            radius: corner_radius(real.size) * scale * look.decoration,
            colors: BarColors::of(color::palette()),
        };
        let top = Rectangle::new(frame.loc, (frame.size.w, inset).into());
        items.push(Item::Decoration {
            key: Key::of(owner.clone(), Part::TitleBar),
            rect: view.rect(top),
            look: look_bar,
            alpha: look.opacity,
        });
    }
    let content = Rectangle::new(
        (frame.loc.x, frame.loc.y + inset).into(),
        (frame.size.w, frame.size.h - inset).into(),
    );
    items.push(Item::Window {
        surface: surface.into_owned(),
        rect: view.rect(content),
        clip: rect,
        geometry,
        alpha: look.opacity,
        radius,
    });
    items.push(Item::Shadow {
        key: Key::of(owner, Part::Shadow),
        shadow: Shadow::of(rect, view.scale * zoom),
        color: color::palette().shadow.premultiplied(),
        radius,
        alpha: look.opacity * look.decoration,
    });
}

/// The strip of the workspaces on a screen: each thumbnail with its name, its border (accent
/// for the active one and for the one a dragged window is over), the windows of its workspace
/// in small, the wallpaper and its background. It comes down from above the screen as the view
/// opens.
fn push_strip(
    items: &mut Vec<Item>,
    state: &State,
    (output, view): (&Output, &View),
    (progress, target, hovered, now): (f64, Option<Target>, Option<Target>, Duration),
    colors: Colors,
) {
    let Some(screen) = state.space.output_geometry(output) else {
        return;
    };
    let (scale, fade) = (view.scale as f32, progress.clamp(0.0, 1.0) as f32);
    // A round button of the strip: a disc and an icon, at the view's scale.
    let push_button = |items: &mut Vec<Item>, key, rect: RectF, icon, background| {
        let pixels = (rect.size.w * view.scale).round().max(1.0) as u32;
        items.push(Item::Label {
            key,
            rect: view.rect(rect),
            label: Label {
                text: String::new(),
                size: (pixels, pixels),
                scale,
                background,
                color: colors.label_text,
                icon: Some(icon),
            },
            alpha: fade,
        });
    };
    let drop = |i: usize| target == Some(Target::Workspace(i));
    let rise = (1.0 - progress) * (strip_height() + f64::from(mc::STRIP_MARGIN));
    let active = state.workspaces().active_index();
    let list = state.workspaces().list();
    // The « + » after the thumbnails: menu glass, accent under the pointer.
    if let Some(mut add) = state.overview_add_button(output) {
        add.loc.y -= rise;
        let background = match hovered == Some(Target::AddWorkspace) {
            true => colors.highlight,
            false => colors.label,
        };
        push_button(
            items,
            Key::of(None, Part::AddWorkspace),
            add,
            mc::ADD_ICON,
            background,
        );
    }
    let fade_all = fade;
    for (index, mut thumb) in state.overview_strip(output).into_iter().enumerate() {
        thumb.loc.y -= rise;
        let fade = fade_all * state.overview.arrival(index, now);
        // The « × » of the thumbnail under the pointer, when another workspace remains.
        let shown = matches!(
            hovered,
            Some(Target::Workspace(i) | Target::RemoveWorkspace(i)) if i == index
        );
        if shown && list.len() > 1 {
            let side = f64::from(mc::CLOSE_SIZE);
            let (x, y) = close_center(thumb);
            let rect = Rectangle::new((x - side / 2.0, y - side / 2.0).into(), (side, side).into());
            let background = match hovered == Some(Target::RemoveWorkspace(index)) {
                true => BarColors::of(color::palette()).close_hover,
                false => colors.label,
            };
            let icon = ui_decorations::Button::Close.icon(false);
            push_button(
                items,
                Key::of(None, Part::ThumbnailClose(index)),
                rect,
                icon,
                background,
            );
        }
        let k = thumb.size.w / f64::from(screen.size.w);
        let radius = mc::THUMBNAIL_RADIUS * scale;
        let name = strings::mission_control::workspace(index + 1);
        let width = f64::from(label::width(&name));
        let at = view.rect(label_rect(thumb, width));
        items.push(Item::Label {
            key: Key::of(None, Part::ThumbnailLabel(index)),
            rect: at,
            label: Label {
                text: name,
                size: (at.size.w.max(1) as u32, at.size.h.max(1) as u32),
                scale,
                background: colors.label,
                color: colors.label_text,
                icon: None,
            },
            alpha: fade,
        });
        let marked = index == active || drop(index);
        let (border, width) = match marked {
            true => (colors.active, mc::THUMBNAIL_ACTIVE_BORDER),
            false => (colors.thumbnail_border, mc::THUMBNAIL_BORDER),
        };
        items.push(Item::Shape {
            key: Key::of(None, Part::ThumbnailBorder(index)),
            rect: view.rect(thumb),
            fill: NO_FILL,
            border: border.premultiplied(),
            border_width: width * scale,
            radius,
            alpha: fade,
        });
        let id = list.get(index).map(|w| w.id.as_str());
        let windows = state.space.elements().rev().filter(|w| {
            Some(state.workspace_of(w).as_str()) == id
                && !state.is_minimized(w)
                && !is_override_redirect(w)
                && state.window_output(w).as_ref() == Some(output)
        });
        // The live windows in the thumbnails, but in the light mode and without effects
        // (« Effets visuels », lot D-26p): the wallpaper alone then.
        let live = sherlock_ui::motion::effects() == sherlock_ui::motion::Effects::Full;
        for window in windows.filter(|_| live) {
            push_miniature(items, state, window, (thumb, screen, k), view, fade);
        }
        items.push(Item::Thumbnail {
            rect: view.rect(thumb),
            radius,
        });
        let fill = colors.thumbnail.premultiplied();
        items.push(Item::Shape {
            key: Key::of(None, Part::Thumbnail(index)),
            rect: view.rect(thumb),
            fill,
            border: fill,
            border_width: 0.0,
            radius,
            alpha: fade,
        });
    }
}

/// A window in a thumbnail: its content, cut to the thumbnail, and a band of the title bar
/// color for the bar the compositor draws, at the scale `k` of the thumbnail.
fn push_miniature(
    items: &mut Vec<Item>,
    state: &State,
    window: &Window,
    (thumb, screen, k): (RectF, Rectangle<i32, Logical>, f64),
    view: &View,
    alpha: f32,
) {
    let (Some(surface), Some(content)) =
        (window.wl_surface(), state.space.element_geometry(window))
    else {
        return;
    };
    let in_thumb = |r: Rectangle<i32, Logical>| {
        let loc = (r.loc - screen.loc).to_f64().upscale(k);
        Rectangle::new(thumb.loc + loc, r.size.to_f64().upscale(k))
    };
    if content.size.is_empty() || window.geometry().size.is_empty() {
        return;
    }
    let frame = in_thumb(decorations::frame(window, content));
    let Some(clip) = frame.intersection(thumb) else {
        return;
    };
    let radius = metrics::WINDOW_RADIUS * view.scale as f32 * k as f32;
    let inset = f64::from(decorations::inset(window)) * k;
    if inset > 0.0 {
        let bar: Color = color::palette().title_bar;
        let top = Rectangle::new(frame.loc, (frame.size.w, inset).into());
        if let Some(top) = top.intersection(thumb) {
            items.push(Item::Shape {
                key: Key::of(Some(surface.id()), Part::MiniBar),
                rect: view.rect(top),
                fill: bar.premultiplied(),
                border: bar.premultiplied(),
                border_width: 0.0,
                radius: 0.0,
                alpha,
            });
        }
    }
    items.push(Item::Window {
        surface: surface.into_owned(),
        rect: view.rect(in_thumb(content)),
        clip: view.rect(clip),
        geometry: window.geometry(),
        alpha,
        radius,
    });
}
