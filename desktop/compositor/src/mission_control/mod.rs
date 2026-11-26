//! Mission Control (lot D-33): every window of the active workspace side by side.
//!
//! - Opens with three fingers up, the gesture key alone, or the gesture key + Tab; closes with
//!   three fingers down, Escape, the same keys, or a click on nothing.
//! - A click on a window gives it the keyboard and closes the view.
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
    color,
    decorations::{self as ui_decorations, Colors as BarColors},
    metrics,
    mission_control::{self as mc, Colors},
    motion::{self, Animation},
};
use smithay::{
    desktop::Window,
    input::keyboard::Keysym,
    output::Output,
    reexports::wayland_server::Resource as _,
    utils::{Logical, Point, Rectangle},
    wayland::seat::WaylandFocus as _,
};

pub use label::{Label, Labels};
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

/// Half a pixel on a screen 4096 wide, as a share of the opening.
const PRECISION: f64 = motion::GEOMETRY_PRECISION / 4096.0;
const NO_FILL: [f32; 4] = [0.0; 4];

/// The state of the view: open or not, how far, and what the pointer does in it.
#[derive(Debug, Default)]
pub struct Overview {
    open: bool,
    /// Progress from 0 (the desktop) to 1 (the grid), on `mc::SPRING`.
    motion: Option<Animation>,
    /// The window under the pointer.
    hovered: Option<Window>,
    /// The left button was pressed on this window (`Some(None)`: on nothing).
    press: Option<Option<Window>>,
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
        match self.motion {
            Some(m) if !m.is_done(now, PRECISION) => m.value(now),
            // Where the last motion went: a view marked open by `enter_overview` has not left
            // the desktop yet.
            Some(m) => m.target(),
            None => 0.0,
        }
    }

    fn animating(&self, now: Duration) -> bool {
        self.motion.is_some_and(|m| !m.is_done(now, PRECISION))
    }

    fn shown(&self, now: Duration) -> bool {
        self.open || self.animating(now)
    }

    /// Leaves from where the view is, towards `to`, without a jolt.
    fn animate(&mut self, now: Duration, to: f64) {
        let from = self.progress(now);
        self.motion = match self.motion {
            Some(mut m) if !m.is_done(now, PRECISION) => {
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

    /// The view is drawn at `now`: open, or on its way back.
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
        if self.enter_overview() {
            self.overview.animate(self.now(), 1.0);
        }
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
        overview.hovered = None;
        overview.press = None;
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
        let margin = f64::from(mc::MARGIN);
        area.loc += Point::from((margin, margin));
        area.size -= (2.0 * margin, 2.0 * margin).into();
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

    /// The window whose slot is at `at` in the open view.
    pub fn overview_window_at(&self, at: Point<f64, Logical>) -> Option<Window> {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        outputs.iter().find_map(|output| {
            let tiles = self.overview_tiles(output, None);
            let tile = tiles.into_iter().rev().find(|t| t.slot.contains(at))?;
            Some(tile.window)
        })
    }

    /// The pointer moved to `at` in the open view.
    pub(crate) fn overview_motion(&mut self, at: Point<f64, Logical>) {
        self.hover_decoration(None);
        self.overview.hovered = self.overview_window_at(at);
    }

    /// A press or release of `button` at `at` in the open view. A click on a window chooses it,
    /// on nothing closes the view. The clients see none of it.
    pub(crate) fn overview_button(&mut self, button: u32, pressed: bool, at: Point<f64, Logical>) {
        if button != crate::input::BTN_LEFT {
            return;
        }
        let over = self.overview_window_at(at);
        if pressed {
            self.overview.press = Some(over);
            return;
        }
        match (self.overview.press.take(), over) {
            (Some(Some(window)), Some(over)) if over == window => {
                self.leave_overview(Some(window));
            }
            (Some(None), None) => self.leave_overview(None),
            _ => {}
        }
    }
}

/// The items of Mission Control on the screen `output`, in place of the windows, from front to
/// back, and whether something moves; `None` when the view is not shown. The windows from the
/// topmost down, then the dim over the wallpaper.
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
    let mut animating = overview.animating(now);
    let tiles = state.overview_tiles(output, Some(now));
    for tile in tiles.iter().rev() {
        animating |= window_data(&tile.window).motion.borrow().is_animating();
        let look = TileLook {
            frame: lerp(tile.home, tile.slot, progress),
            highlight: overview.hovered.as_ref() == Some(&tile.window),
            label: fade,
            opacity: tile.opacity,
            decoration: tile.decoration + (1.0 - tile.decoration) * fade,
        };
        push_tile(items, state, tile, look, view, colors);
    }
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
    highlight: bool,
    /// Opacity of its title below it.
    label: f32,
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
    if look.label > 0.0 && !title.is_empty() {
        let width = f64::from(label::width(&title));
        let at = view.rect(label_rect(frame, width));
        let label = Label {
            text: title,
            size: (at.size.w.max(1) as u32, at.size.h.max(1) as u32),
            scale,
            background: colors.label,
            color: colors.label_text,
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
        border: color::BORDER.premultiplied(),
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
        radius,
        alpha: look.opacity * look.decoration,
    });
}
