//! Minimize and restore effect (lots D-19, D-19b). Minimized, a window is sucked into its dock
//! icon, the rectangle the dock gives with `set_rectangle` (`State::icon_rectangle`), as by
//! macOS's genie: its edge nearest the dock is pulled first, the sides close in a funnel with
//! curved sides, and the rest of the window flows after it, faster and faster, into the icon.
//! It goes behind the dock: what has crossed the edge of the dock's glass is hidden. Activated,
//! it comes back the same way, backwards. Without an icon, it shrinks and fades toward the
//! bottom of its screen. `Effect` chooses between the two, or nothing.
//!
//! The deformation is a fragment shader (`shaders/genie.frag`), not a mesh. The window is first
//! drawn as at rest into a texture of its size, corners and border included; then a single
//! rectangle covers the area the deformed window takes, and each of its pixels computes the
//! point of the window it shows: the exact inverse of the deformation. Smithay's GLES renderer
//! takes fragment programs of our own but no vertices, and a mesh of trapezoids drawn as
//! triangles would bend the texture along their diagonals. `Genie` holds the same computation
//! for the tests and for the area to draw and damage.
//!
//! See "Minimize effect" in `docs/compositor.md`.

use std::{cell::RefCell, str::FromStr, time::Duration};

use sherlock_ui::{decorations as ui_decorations, dock, metrics, motion};
use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        Bind as _, Frame as _, Offscreen as _, Renderer as _, Texture as _,
        element::{Element, Id, Kind, RenderElement},
        gles::{
            GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
            UniformType,
        },
        utils::CommitCounter,
    },
    desktop::Window,
    output::Output,
    reexports::wayland_server::Resource as _,
    utils::{Buffer, Logical, Physical, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus as _,
};

use crate::{
    State,
    anim::{RectF, Visual},
    damage::{Ids, Key, Part, Stamped, params_hash},
    decorations::Bar,
    handlers::Arrangement,
    output::View,
    render::{self, Item, SceneElement, Shadow},
};

/// What minimizing and restoring a window shows: a setting of the compositor
/// (`--minimize-effect`, `State::set_minimize_effect`), for Control Room.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Effect {
    /// Into the dock icon; the scale effect for a window without one.
    #[default]
    Genie,
    /// Shrinks and fades toward the bottom of its screen.
    Scale,
    /// Hidden and shown at once.
    None,
}

/// A name `Effect` does not know.
#[derive(Debug, thiserror::Error)]
#[error("unknown minimize effect {0:?}: genie, scale or none")]
pub struct UnknownEffect(String);

impl FromStr for Effect {
    type Err = UnknownEffect;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "genie" => Ok(Self::Genie),
            "scale" => Ok(Self::Scale),
            "none" => Ok(Self::None),
            _ => Err(UnknownEffect(name.to_owned())),
        }
    }
}

/// Share of the way by which the window's far edge leaves after its edge nearest the dock: the
/// near edge is pulled first, the rest of the window flows after it. `genie.frag` has the same.
const DELAY: f64 = 0.4;
/// Share of the way over which the sides bend into their funnel.
const BEND: f64 = 0.4;
/// Share of the way over which the shadow fades out.
const SHADOW_FADE: f64 = 0.3;
/// Share of the way over which the hidden part of a window that reaches into the dock's glass
/// at rest grows up to the edge of the glass.
const SWEEP: f64 = 0.25;
/// From this share on, what is left of the window fades out.
const FADE_FROM: f64 = 0.9;

/// Distance from an icon of the dock to the edge of its glass toward the screen's middle, in
/// logical pixels: the bar's padding on that side, less its border, which is inside the bar
/// (see `Geometry::bar_thickness` in the dock). The same for any icon size.
pub const GLASS_MARGIN: f64 = (dock::PADDING.1 - metrics::BORDER_WIDTH) as f64;

/// Slow start, slow landing (smoothstep), on `x` clamped to 0..1.
fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Slow start, then faster and faster, on `x` clamped to 0..1: each row of the window speeds
/// up toward the icon. `genie.frag` has the same.
fn ease_in(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x
}

/// The side of the funnel, from 0 at its mouth to 1 at its end: half a period of a sinusoid,
/// level at both ends. `genie.frag` has the same.
fn sine(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    (1.0 - (std::f64::consts::PI * x).cos()) / 2.0
}

fn mix(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// How far a window is between its place (0) and its icon (1), at a constant pace: the whole
/// way takes `motion::MINIMIZE`. Turned around halfway, it leaves from where it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    from: f64,
    start: Duration,
    minimizing: bool,
}

impl Progress {
    /// From the window's place toward its icon (`minimizing`), or back from the icon.
    pub fn new(now: Duration, minimizing: bool) -> Self {
        let from = if minimizing { 0.0 } else { 1.0 };
        Self {
            from,
            start: now,
            minimizing,
        }
    }

    pub fn value(&self, now: Duration) -> f64 {
        let run = now.saturating_sub(self.start).as_secs_f64() / motion::MINIMIZE.as_secs_f64();
        if self.minimizing {
            (self.from + run).min(1.0)
        } else {
            (self.from - run).max(0.0)
        }
    }

    /// Goes the other way from where it is at `now`.
    pub fn turn(&mut self, now: Duration, minimizing: bool) {
        *self = Self {
            from: self.value(now),
            start: now,
            minimizing,
        };
    }

    pub fn is_done(&self, now: Duration) -> bool {
        let value = self.value(now);
        if self.minimizing {
            value >= 1.0
        } else {
            value <= 0.0
        }
    }
}

/// A rectangle in frame pixels, unrounded.
pub type FrameRect = Rectangle<f64, Physical>;

/// Edge of the screen the dock is on: the window goes toward it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Bottom,
    Top,
    Left,
    Right,
}

impl Side {
    /// The edge of `screen` nearest to `icon`.
    pub fn of<K>(icon: Rectangle<f64, K>, screen: Rectangle<f64, K>) -> Self {
        let distances = [
            (
                Self::Bottom,
                screen.loc.y + screen.size.h - icon.loc.y - icon.size.h,
            ),
            (Self::Top, icon.loc.y - screen.loc.y),
            (Self::Left, icon.loc.x - screen.loc.x),
            (
                Self::Right,
                screen.loc.x + screen.size.w - icon.loc.x - icon.size.w,
            ),
        ];
        distances
            .into_iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(Self::Bottom, |(side, _)| side)
    }

    /// The dock is on the left or right edge: the window goes sideways.
    fn sideways(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// Toward the dock is toward lower coordinates.
    fn backwards(self) -> bool {
        matches!(self, Self::Top | Self::Left)
    }

    /// A point of the frame as (along, across): `along` grows toward the dock.
    fn orient(self, (x, y): (f64, f64)) -> (f64, f64) {
        let (along, across) = if self.sideways() { (x, y) } else { (y, x) };
        (if self.backwards() { -along } else { along }, across)
    }

    /// The point of the frame at (`along`, `across`): `orient` undone.
    fn unorient(self, (along, across): (f64, f64)) -> (f64, f64) {
        let along = if self.backwards() { -along } else { along };
        if self.sideways() {
            (along, across)
        } else {
            (across, along)
        }
    }

    /// `rect` seen along the way: its ends along, the far one first, then across.
    fn span(self, rect: FrameRect) -> Span {
        let a = self.orient((rect.loc.x, rect.loc.y));
        let b = self.orient((rect.loc.x + rect.size.w, rect.loc.y + rect.size.h));
        Span {
            along: (a.0.min(b.0), a.0.max(b.0)),
            across: (a.1.min(b.1), a.1.max(b.1)),
        }
    }
}

/// A rectangle seen along the way to the dock.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Span {
    along: (f64, f64),
    across: (f64, f64),
}

/// The genie at `progress`, between `window` and `icon`, in frame pixels. `genie.frag` does the
/// same computation, backwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Genie {
    pub window: FrameRect,
    pub icon: FrameRect,
    /// The screen edge of the dock.
    pub side: Side,
    /// Distance from the icon to the edge of the dock's glass on the window's side, in frame
    /// pixels: what of the window has crossed that edge is hidden behind the dock.
    pub glass: f64,
    pub progress: f64,
}

/// The genie's shape at its progress, seen along the way.
#[derive(Debug, Clone, Copy)]
struct Shape {
    window: Span,
    icon: Span,
    /// Where the funnel starts, the window's far edge at rest, and where it ends, the edge of
    /// the glass: there the sides have the icon's width.
    funnel: (f64, f64),
    /// How far the sides are bent into the funnel, from 0 to 1.
    bend: f64,
    /// Nothing past this shows: the edge of the glass, once a window that reached into the
    /// glass at rest has left it.
    clip: f64,
    progress: f64,
}

impl Shape {
    /// Where row `r` of the window is along, 0 its far edge, 1 its edge nearest the dock: from
    /// its place at rest to the same row of the icon, the near rows leaving first.
    fn along(&self, r: f64) -> f64 {
        let rest = mix(self.window.along.0, self.window.along.1, r);
        let end = mix(self.icon.along.0, self.icon.along.1, r);
        mix(
            rest,
            end,
            ease_in((self.progress - (1.0 - r) * DELAY) / (1.0 - DELAY)),
        )
    }

    /// The two ends, across, of what is drawn at `along`.
    fn across(&self, along: f64) -> (f64, f64) {
        let (start, end) = self.funnel;
        let f = self.bend * sine((along - start) / (end - start));
        (
            mix(self.window.across.0, self.icon.across.0, f),
            mix(self.window.across.1, self.icon.across.1, f),
        )
    }

    /// The part of the way that shows: from the far edge to the near one or the clip.
    fn shown(&self) -> Option<(f64, f64)> {
        let (far, near) = (self.along(0.0), self.along(1.0).min(self.clip));
        (far < near).then_some((far, near))
    }
}

impl Genie {
    fn shape(&self) -> Shape {
        let window = self.side.span(self.window);
        let icon = self.side.span(self.icon);
        let edge = icon.along.0 - self.glass;
        let funnel = (window.along.0, edge.max(window.along.0 + 1.0));
        // A window that reaches behind the glass at rest is not cut at once.
        let overlap = (window.along.1 + 1.0 - edge).max(0.0);
        Shape {
            window,
            icon,
            funnel,
            bend: ease(self.progress / BEND),
            clip: edge + overlap * (1.0 - ease(self.progress / SWEEP)),
            progress: self.progress,
        }
    }

    /// Where the point (`u`, `v`) of the window, 0 to 1 across its width and height, is drawn,
    /// in frame pixels; `None` once it is hidden behind the dock's glass.
    pub fn point(&self, (u, v): (f64, f64)) -> Option<(f64, f64)> {
        let shape = self.shape();
        let rest = (
            self.window.loc.x + u * self.window.size.w,
            self.window.loc.y + v * self.window.size.h,
        );
        let (along, across) = self.side.orient(rest);
        let share = |x: f64, (a, b): (f64, f64)| (x - a) / (b - a).max(f64::EPSILON);
        let along = shape.along(share(along, shape.window.along));
        if along > shape.clip {
            return None;
        }
        let ends = shape.across(along);
        let across = mix(ends.0, ends.1, share(across, shape.window.across));
        Some(self.side.unorient((along, across)))
    }

    /// Area the deformed window covers, one pixel more around for the smoothing of its edges;
    /// `None` when all of it is behind the glass. The ends of each side move monotonically
    /// along the way: the first and last rows shown hold the extremes.
    pub fn bounds(&self) -> Option<Rectangle<i32, Physical>> {
        let shape = self.shape();
        let (far, near) = shape.shown()?;
        let (a, b) = (shape.across(far), shape.across(near));
        let corners = [
            self.side.unorient((far, a.0.min(b.0))),
            self.side.unorient((near, a.1.max(b.1))),
        ];
        let x0 = (corners[0].0.min(corners[1].0)).floor() - 1.0;
        let x1 = (corners[0].0.max(corners[1].0)).ceil() + 1.0;
        let y0 = (corners[0].1.min(corners[1].1)).floor() - 1.0;
        let y1 = (corners[0].1.max(corners[1].1)).ceil() + 1.0;
        Some(Rectangle::new(
            (x0 as i32, y0 as i32).into(),
            ((x1 - x0) as i32, (y1 - y0) as i32).into(),
        ))
    }

    /// Opacity: what is left of the window fades at the very end.
    pub fn alpha(&self) -> f32 {
        (1.0 - ease((self.progress - FADE_FROM) / (1.0 - FADE_FROM))) as f32
    }

    /// Uniforms of `genie.frag` for the element covering `area`.
    fn uniforms(&self, area: Rectangle<i32, Physical>) -> Vec<Uniform<'static>> {
        let shape = self.shape();
        let span = |s: Span| [s.along.0, s.along.1, s.across.0, s.across.1].map(|v| v as f32);
        let rect = |r: FrameRect| [r.loc.x, r.loc.y, r.size.w, r.size.h].map(|v| v as f32);
        let area = [area.loc.x, area.loc.y, area.size.w, area.size.h].map(|v| v as f32);
        let funnel = [shape.funnel.0, shape.funnel.1, shape.bend, shape.clip].map(|v| v as f32);
        let sideways = if self.side.sideways() { 1.0 } else { 0.0 };
        let sign = if self.side.backwards() { -1.0 } else { 1.0 };
        vec![
            Uniform::new("window_rect", rect(self.window)),
            Uniform::new("window_span", span(shape.window)),
            Uniform::new("icon_span", span(shape.icon)),
            Uniform::new("funnel", funnel),
            Uniform::new("area", area),
            Uniform::new("axis", [sideways, sign]),
            Uniform::new("progress", self.progress as f32),
        ]
    }
}

/// The scale effect at `progress`: `window` shrinks toward the bottom of `screen`, centered
/// under itself, and fades.
pub fn scale_visual(window: RectF, screen: RectF, progress: f64, decoration: f32) -> Visual {
    let e = ease(progress);
    let size = window.size.upscale(motion::MINIMIZE_SCALE);
    let end_x = window.loc.x + (window.size.w - size.w) / 2.0;
    let end_y = screen.loc.y + screen.size.h - size.h;
    let rect = Rectangle::new(
        (mix(window.loc.x, end_x, e), mix(window.loc.y, end_y, e)).into(),
        (mix(window.size.w, size.w, e), mix(window.size.h, size.h, e)).into(),
    );
    Visual {
        rect,
        opacity: (1.0 - e) as f32,
        moving: true,
        decoration,
        scale: 1.0,
        shadow: 1.0,
    }
}

/// The effect chosen for a window when it starts.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Way {
    Genie,
    Scale,
}

/// The effect running on a window, kept in its user data, and the last icon rectangle seen,
/// in the space, with the screen edge of its dock: the dock may forget it on the way.
#[derive(Default)]
struct Track {
    running: std::cell::Cell<Option<(Progress, Way)>>,
    icon: std::cell::Cell<Option<(Rectangle<i32, Logical>, Side)>>,
}

fn track(window: &Window) -> &Track {
    window.user_data().get_or_insert(Track::default)
}

impl State {
    /// Chooses the effect of the windows minimized or restored from now on.
    pub fn set_minimize_effect(&mut self, effect: Effect) {
        self.minimize_effect = effect;
    }

    /// Starts the effect of `window` being minimized (`minimizing`) or restored. One already
    /// running turns around from where it is, without a jump.
    pub(crate) fn start_minimize_effect(&mut self, window: &Window, minimizing: bool) {
        let now = self.now();
        let track = track(window);
        let running = track.running.get().filter(|(p, _)| !p.is_done(now));
        let started = match (running, self.minimize_effect) {
            (Some((mut progress, way)), _) => {
                progress.turn(now, minimizing);
                Some((progress, way))
            }
            (None, Effect::None) => None,
            (None, effect) => {
                let icon = self.icon_in_space(window);
                track.icon.set(icon);
                let genie = effect == Effect::Genie && icon.is_some();
                let way = if genie { Way::Genie } else { Way::Scale };
                Some((Progress::new(now, minimizing), way))
            }
        };
        track.running.set(started);
        self.queue_redraw();
    }

    /// Progress of the effect of `window` at `now`, `None` when none runs: while it runs, the
    /// window is drawn by it and the pointer does not reach it.
    pub fn minimize_progress(&self, window: &Window, now: Duration) -> Option<f64> {
        let (progress, _) = track(window).running.get()?;
        (!progress.is_done(now)).then(|| progress.value(now))
    }

    /// Forgets the effects over at this time. The pointer did not reach their windows while
    /// they ran: it goes to what is under it now, as after a move, so that a click made without
    /// moving reaches a window back from its icon. Called after each loop iteration.
    pub(crate) fn end_minimize_effects(&mut self) {
        let now = self.now();
        let mut ended = false;
        for window in self.space.elements() {
            let track = track(window);
            if track.running.get().is_some_and(|(p, _)| p.is_done(now)) {
                track.running.set(None);
                ended = true;
            }
        }
        if ended {
            self.refresh_pointer_focus();
        }
    }

    /// The icon rectangle of `window` in the space, and the edge of its screen the dock is on:
    /// the nearest one.
    fn icon_in_space(&self, window: &Window) -> Option<(Rectangle<i32, Logical>, Side)> {
        let (name, rect) = self.icon_rectangle(window)?;
        let output = self.space.outputs().find(|o| o.name() == name)?;
        let screen = self.space.output_geometry(output)?;
        let icon = Rectangle::new(rect.loc + screen.loc, rect.size);
        Some((icon, Side::of(icon.to_f64(), screen.to_f64())))
    }
}

/// Items of the windows whose effect runs at `now`, seen on `output` through `view`, from front
/// to back, and whether one runs. They go in front of the other windows, behind the bar and
/// the dock: the window is sucked into its icon.
pub(crate) fn push_effects(
    items: &mut Vec<Item>,
    state: &State,
    (output, view): (&Output, &View),
    now: Duration,
) -> bool {
    let mut running = false;
    let active = state.workspaces().active();
    for window in state.space.elements().rev() {
        let track = track(window);
        let Some((progress, way)) = track.running.get() else {
            continue;
        };
        // Over: `end_minimize_effects` forgets it and gives the pointer back.
        if progress.is_done(now) {
            continue;
        }
        let geometry = state.space.element_geometry(window);
        let Some(geometry) = geometry.filter(|g| !g.size.is_empty()) else {
            continue;
        };
        if state.workspace_of(window) != active {
            continue;
        }
        running = true;
        let value = progress.value(now);
        let arrangement = Arrangement::of(window);
        let decoration = if arrangement.maximized || arrangement.fullscreen {
            0.0
        } else {
            1.0
        };
        let icon = state.icon_in_space(window).or(track.icon.get());
        track.icon.set(icon);
        let screen = state.window_output(window);
        let bar = state.decoration_bar(window);
        match (way, icon) {
            (Way::Genie, Some(icon)) => {
                let window = (window, bar);
                push_genie(items, window, (geometry, icon), value, decoration, view);
            }
            _ => {
                let area = screen.as_ref().and_then(|o| state.space.output_geometry(o));
                let area = area.unwrap_or(geometry).to_f64();
                let visual = scale_visual(geometry.to_f64(), area, value, decoration);
                let own = screen.as_ref() == Some(output);
                render::push_window(items, (window, bar), visual, view, own);
            }
        }
    }
    running
}

/// The genie of `window`, between its geometry and its icon in the space, and its shadow, which
/// fades out while the sides bend. The title bar the compositor draws above it (lot D-08) goes
/// into the icon with it.
fn push_genie(
    items: &mut Vec<Item>,
    (window, bar): (&Window, Option<Bar>),
    (content_geometry, (icon, side)): (Rectangle<i32, Logical>, (Rectangle<i32, Logical>, Side)),
    progress: f64,
    decoration: f32,
    view: &View,
) {
    let Some(surface) = window.wl_surface() else {
        return;
    };
    let inset = bar
        .as_ref()
        .map_or(0, |_| ui_decorations::INSET.round() as i32);
    let mut geometry = content_geometry;
    geometry.loc.y -= inset;
    geometry.size.h += inset;
    let rect = view.rect(geometry.to_f64());
    let genie = Genie {
        window: rect.to_f64(),
        icon: view.rect(icon.to_f64()).to_f64(),
        side,
        glass: GLASS_MARGIN * view.scale,
        progress,
    };
    let owner = Some(surface.id());
    let fade = (1.0 - ease(progress / SHADOW_FADE)) as f32 * decoration;
    let shadow = (fade > 0.0).then(|| Item::Shadow {
        key: Key::of(owner.clone(), Part::Shadow),
        shadow: Shadow::of(rect, view.scale),
        color: sherlock_ui::color::palette().shadow.premultiplied(),
        radius: render::corner_radius(geometry.size) * view.scale as f32 * decoration,
        alpha: fade,
    });
    // All of it behind the dock's glass: nothing to draw but what is left of its shadow.
    let Some(area) = genie.bounds().filter(|a| a.overlaps(view.frame)) else {
        items.extend(shadow);
        return;
    };
    // The window as at rest, drawn into a texture of its size in pixels.
    let size = geometry
        .size
        .to_f64()
        .to_physical(view.scale)
        .to_i32_round();
    let mut content = Vec::new();
    let rest = Visual {
        rect: content_geometry.to_f64(),
        opacity: 1.0,
        moving: false,
        decoration,
        scale: 1.0,
        shadow: 1.0,
    };
    render::push_window(
        &mut content,
        (window, bar),
        rest,
        &offscreen(geometry, size, view.scale),
        false,
    );
    items.push(Item::Genie(Frame {
        key: Key::of(owner, Part::Genie),
        content,
        size,
        scale: view.scale,
        area,
        genie,
        alpha: genie.alpha(),
    }));
    items.extend(shadow);
}

/// The view of a texture of `size` pixels showing `area` of the space at `scale`.
pub(crate) fn offscreen(
    area: Rectangle<i32, Logical>,
    size: Size<i32, Physical>,
    scale: f64,
) -> View {
    View {
        area,
        frame: Rectangle::from_size(size),
        scale,
        frame_size: size,
    }
}

/// A window during its genie, as plain data (`render::Item::Genie`).
#[derive(Debug)]
pub struct Frame {
    pub key: Key,
    /// The window as at rest: its items, in the pixels of a texture of `size`.
    pub content: Vec<Item>,
    pub size: Size<i32, Physical>,
    pub scale: f64,
    /// Area the deformed window covers, in frame pixels.
    pub area: Rectangle<i32, Physical>,
    pub genie: Genie,
    pub alpha: f32,
}

impl Frame {
    /// Hash of what it draws, for `Ids::stamp`.
    pub fn params(&self) -> u64 {
        let g = self.genie;
        let rects = [g.window, g.icon].map(|r| [r.loc.x, r.loc.y, r.size.w, r.size.h]);
        let side = g.side as u8;
        let values = [
            rects[0],
            rects[1],
            [g.progress, f64::from(self.alpha), g.glass, f64::from(side)],
        ];
        let values: Vec<f32> = values.concat().into_iter().map(|v| v as f32).collect();
        params_hash(&values)
    }

    /// The view its content is drawn through, in the texture.
    pub fn view(&self) -> View {
        let area = Rectangle::from_size(self.size.to_f64().to_logical(self.scale).to_i32_round());
        offscreen(area, self.size, self.scale)
    }
}

/// The genie's texture program, compiled once.
#[derive(Clone)]
pub struct Program(GlesTexProgram);

impl Program {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let uniform = |name, type_| UniformName::new(name, type_);
        let program = renderer.compile_custom_texture_shader(
            include_str!("shaders/genie.frag"),
            &[
                uniform("window_rect", UniformType::_4f),
                uniform("window_span", UniformType::_4f),
                uniform("icon_span", UniformType::_4f),
                uniform("funnel", UniformType::_4f),
                uniform("area", UniformType::_4f),
                uniform("axis", UniformType::_2f),
                uniform("progress", UniformType::_1f),
            ],
        )?;
        Ok(Self(program))
    }
}

/// Draws the genies: the program, and one texture per window and size, kept while it runs.
pub struct Painter {
    program: Program,
    textures: RefCell<Vec<(Key, GlesTexture, bool)>>,
}

impl Painter {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        Ok(Self {
            program: Program::new(renderer)?,
            textures: RefCell::new(Vec::new()),
        })
    }

    /// The element of `frame`, whose content is already turned into `content`, under its
    /// stable identity: its commit moves at every step. `None` if the window could not be
    /// drawn into its texture.
    pub fn element(
        &self,
        renderer: &mut GlesRenderer,
        ids: &mut Ids,
        frame: &Frame,
        content: &[SceneElement],
    ) -> Option<Stamped<GenieElement>> {
        let texture = match self.draw(renderer, &frame.key, frame.size, content) {
            Ok(texture) => texture,
            Err(err) => {
                tracing::warn!("genie: drawing the window: {err}");
                return None;
            }
        };
        let genie = (&frame.genie, frame.area);
        let element = GenieElement::new(&self.program, texture, genie, frame.alpha);
        Some(ids.stamp(frame.key.clone(), frame.params(), element))
    }

    /// Draws `content` into the texture of `key` at `size`.
    fn draw(
        &self,
        renderer: &mut GlesRenderer,
        key: &Key,
        size: Size<i32, Physical>,
        content: &[SceneElement],
    ) -> Result<GlesTexture, GlesError> {
        let mut texture = self.texture(renderer, key, size)?;
        let mut target = renderer.bind(&mut texture)?;
        let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
        frame.clear([0.0f32; 4].into(), &[Rectangle::from_size(size)])?;
        for element in content.iter().rev() {
            let dst = element.geometry(Scale::from(1.0));
            let damage = [Rectangle::from_size(dst.size)];
            element.draw(&mut frame, element.src(), dst, &damage, &[])?;
        }
        // Sampled next in the same GL context, which keeps the order: nothing to wait for.
        let _ = frame.finish()?;
        drop(target);
        Ok(texture)
    }

    /// The texture of `key` at `size`, created when missing.
    fn texture(
        &self,
        renderer: &mut GlesRenderer,
        key: &Key,
        size: Size<i32, Physical>,
    ) -> Result<GlesTexture, GlesError> {
        let mut textures = self.textures.borrow_mut();
        let buffer_size = Size::<i32, Buffer>::from((size.w, size.h));
        let found = textures
            .iter_mut()
            .find(|(k, t, _)| k == key && t.size() == buffer_size);
        if let Some((_, texture, used)) = found {
            *used = true;
            return Ok(texture.clone());
        }
        let texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, buffer_size)?;
        textures.push((key.clone(), texture.clone(), true));
        Ok(texture)
    }

    /// End of a frame: the textures it did not use are freed.
    pub fn sweep(&self) {
        self.textures
            .borrow_mut()
            .retain_mut(|(_, _, used)| std::mem::take(used));
    }
}

/// The window's texture, deformed by `genie.frag` over the area of its genie.
pub struct GenieElement {
    id: Id,
    program: GlesTexProgram,
    texture: GlesTexture,
    area: Rectangle<i32, Physical>,
    uniforms: Vec<Uniform<'static>>,
    alpha: f32,
}

impl GenieElement {
    /// `texture`, the window as at rest, at the step `genie`, with the opacity `alpha`, over
    /// `area` (`Genie::bounds`).
    pub fn new(
        program: &Program,
        texture: GlesTexture,
        (genie, area): (&Genie, Rectangle<i32, Physical>),
        alpha: f32,
    ) -> Self {
        Self {
            id: Id::new(),
            program: program.0.clone(),
            texture,
            area,
            uniforms: genie.uniforms(area),
            alpha,
        }
    }
}

impl Element for GenieElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        CommitCounter::default()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.texture.size()).to_f64()
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.area
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for GenieElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
            Some(&self.program),
            &self.uniforms,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> FrameRect {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// A window of 800 × 600 at (320, 100), its icon of 52 × 52 in the dock at the bottom of a
    /// screen of 900, the glass 11 above it.
    fn genie(progress: f64) -> Genie {
        Genie {
            window: rect(320.0, 100.0, 800.0, 600.0),
            icon: rect(900.0, 816.0, 52.0, 52.0),
            side: Side::Bottom,
            glass: 11.0,
            progress,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// Points of the window, 0 to 1 across its width and height.
    fn grid() -> impl Iterator<Item = (f64, f64)> {
        (0..=10).flat_map(|i| (0..=10).map(move |j| (f64::from(i) / 10.0, f64::from(j) / 10.0)))
    }

    /// The progress values of 120 frames.
    fn steps() -> impl Iterator<Item = f64> {
        (0..=120).map(|i| f64::from(i) / 120.0)
    }

    #[test]
    fn the_genie_leaves_the_window_and_ends_behind_the_glass() {
        // At 0: the window as it is.
        let rest = genie(0.0);
        for (u, v) in grid() {
            let (x, y) = rest.point((u, v)).unwrap();
            assert!(
                close(x, 320.0 + 800.0 * u) && close(y, 100.0 + 600.0 * v),
                "{u} {v}"
            );
        }
        assert_eq!(
            rest.bounds(),
            Some(Rectangle::new((319, 99).into(), (802, 602).into()))
        );
        assert_eq!(rest.alpha(), 1.0);
        // At 1: all of it in the icon, behind the glass.
        assert!(grid().all(|p| genie(1.0).point(p).is_none()));
        assert_eq!(genie(1.0).bounds(), None);
        // Halfway: the bottom has gone in first, the top still shows, narrower than the window,
        // and moved toward the icon.
        let half = genie(0.5);
        assert!(
            half.point((0.5, 1.0)).is_none(),
            "the bottom is behind the glass"
        );
        let (top_left, top_right) = (
            half.point((0.0, 0.0)).unwrap(),
            half.point((1.0, 0.0)).unwrap(),
        );
        assert!(top_left.1 > 100.0 && top_left.1 < 805.0);
        assert!(top_right.0 - top_left.0 < 800.0);
        assert!(top_left.0 > 320.0 && top_right.0 < 1120.0);
        // The rows stay in order: the window is never turned over.
        let rows: Vec<f64> = (0..=10)
            .filter_map(|j| half.point((0.5, f64::from(j) / 10.0)))
            .map(|p| p.1)
            .collect();
        assert!(rows.len() > 2 && rows.windows(2).all(|r| r[0] < r[1]));
        // Every point shown is in its area.
        let area = half.bounds().unwrap();
        for (x, y) in grid().filter_map(|p| half.point(p)) {
            assert!(area.contains((x as i32, y as i32)), "{x} {y}");
        }
    }

    #[test]
    fn nothing_shows_past_the_edge_of_the_glass_nor_beside_the_icon() {
        let edge = 816.0 - 11.0;
        for progress in steps() {
            let genie = genie(progress);
            for (x, y) in grid().filter_map(|p| genie.point(p)) {
                assert!(y <= edge + 1e-9, "{progress}: ({x}, {y}) past the glass");
                // Within the way from the window to the icon, across.
                assert!(
                    (320.0 - 1e-9..=1120.0 + 1e-9).contains(&x),
                    "{progress}: {x}"
                );
            }
            if let Some(area) = genie.bounds() {
                assert!(
                    area.loc.y + area.size.h <= edge as i32 + 2,
                    "{progress}: {area:?}"
                );
            }
        }
        // Where it enters the glass, the window is as wide as the icon: it goes into it.
        let late = genie(0.7);
        let row = (0..=1000)
            .rev()
            .map(|j| f64::from(j) / 1000.0)
            .find_map(|v| Some((late.point((0.0, v))?, late.point((1.0, v))?)))
            .unwrap();
        let ((left, y), (right, _)) = row;
        assert!(y > edge - 3.0, "{y}");
        assert!(
            (left - 900.0).abs() < 3.0 && (right - 952.0).abs() < 3.0,
            "{left} {right}"
        );
    }

    #[test]
    fn each_point_moves_toward_the_icon_without_a_jump() {
        for (u, v) in grid() {
            let mut last = genie(0.0).point((u, v)).unwrap();
            for progress in steps().skip(1) {
                let Some(now) = genie(progress).point((u, v)) else {
                    break;
                };
                assert!(now.1 >= last.1 - 1e-9, "{u} {v} {progress}: goes back up");
                // 120 frames for at most about 900 pixels, faster at the end.
                let step = ((now.0 - last.0).powi(2) + (now.1 - last.1).powi(2)).sqrt();
                assert!(step < 30.0, "{u} {v} {progress}: jumps {step}");
                last = now;
            }
        }
    }

    #[test]
    fn a_window_behind_the_glass_at_rest_is_not_cut_at_once() {
        // Maximized under a dock that does not reserve its room: its bottom behind the glass.
        let tall = Genie {
            window: rect(0.0, 36.0, 1440.0, 864.0),
            ..genie(0.0)
        };
        assert!(tall.point((0.5, 1.0)).is_some());
        assert_eq!(tall.bounds().map(|a| a.loc.y + a.size.h), Some(901));
        // Soon cut at the glass, as the others.
        let later = Genie {
            progress: SWEEP,
            ..tall
        };
        assert!(later.point((0.5, 1.0)).is_none());
        let shown = grid().filter_map(|p| later.point(p));
        assert!(shown.into_iter().all(|(_, y)| y <= 805.0 + 1e-9));
    }

    #[test]
    fn the_dock_on_any_edge_gives_the_same_genie_turned() {
        // The bottom case turned a quarter: a dock on the right of a screen of 1440 × 900.
        let screen = rect(0.0, 0.0, 1440.0, 900.0);
        let icon = rect(1380.0, 424.0, 52.0, 52.0);
        assert_eq!(Side::of(icon, screen), Side::Right);
        assert_eq!(Side::of(rect(8.0, 424.0, 52.0, 52.0), screen), Side::Left);
        assert_eq!(Side::of(rect(700.0, 8.0, 52.0, 52.0), screen), Side::Top);
        assert_eq!(
            Side::of(rect(700.0, 840.0, 52.0, 52.0), screen),
            Side::Bottom
        );
        let bottom = genie(0.6);
        let turn = |side: Side, (x, y): (f64, f64)| match side {
            Side::Bottom => (x, y),
            Side::Right => (y, x),
            Side::Top => (x, -y),
            Side::Left => (-y, x),
        };
        let rect_of = |side: Side, r: FrameRect| {
            let a = turn(side, (r.loc.x, r.loc.y));
            let b = turn(side, (r.loc.x + r.size.w, r.loc.y + r.size.h));
            rect(
                a.0.min(b.0),
                a.1.min(b.1),
                (a.0 - b.0).abs(),
                (a.1 - b.1).abs(),
            )
        };
        for side in [Side::Right, Side::Top, Side::Left] {
            let turned = Genie {
                window: rect_of(side, bottom.window),
                icon: rect_of(side, bottom.icon),
                side,
                ..bottom
            };
            for (u, v) in grid() {
                // The same point of the window, its coordinates turned with it.
                let (x, y) = (320.0 + 800.0 * u, 100.0 + 600.0 * v);
                let (tx, ty) = turn(side, (x, y));
                let w = turned.window;
                let at = ((tx - w.loc.x) / w.size.w, (ty - w.loc.y) / w.size.h);
                let expected = bottom.point((u, v)).map(|p| turn(side, p));
                let got = turned.point(at);
                match (expected, got) {
                    (None, None) => {}
                    (Some(e), Some(g)) => {
                        assert!(
                            (e.0 - g.0).abs() < 1e-6 && (e.1 - g.1).abs() < 1e-6,
                            "{side:?}"
                        )
                    }
                    _ => panic!("{side:?} {u} {v}: {expected:?} {got:?}"),
                }
            }
        }
    }

    #[test]
    fn without_an_icon_the_window_shrinks_toward_the_bottom_of_its_screen() {
        let window = Rectangle::new((320.0, 100.0).into(), (800.0, 600.0).into());
        let screen = Rectangle::new((0.0, 0.0).into(), (1440.0, 900.0).into());
        let start = scale_visual(window, screen, 0.0, 1.0);
        assert_eq!((start.rect, start.opacity), (window, 1.0));
        let end = scale_visual(window, screen, 1.0, 1.0);
        assert_eq!(end.opacity, 0.0);
        assert_eq!(end.rect.size, (160.0, 120.0).into());
        // Centered under the window, its bottom on the screen's bottom edge.
        assert_eq!(end.rect.loc, (640.0, 780.0).into());
        let half = scale_visual(window, screen, 0.5, 1.0);
        assert!(half.rect.size.w < 800.0 && half.rect.size.w > 160.0);
        assert!(half.opacity > 0.0 && half.opacity < 1.0);
    }

    /// `share` of the whole way after `at`.
    fn after(at: Duration, share: f64) -> Duration {
        at + motion::MINIMIZE.mul_f64(share)
    }

    #[test]
    fn minimize_and_restore_take_the_same_time() {
        for minimizing in [true, false] {
            let progress = Progress::new(ms(1000), minimizing);
            assert!(!progress.is_done(after(ms(1000), 0.99)));
            assert!(progress.is_done(after(ms(1000), 1.0)));
        }
    }

    #[test]
    fn turned_around_halfway_it_leaves_from_where_it_is() {
        let mut progress = Progress::new(ms(1000), true);
        assert_eq!(progress.value(ms(1000)), 0.0);
        let half = after(ms(1000), 0.5);
        let at = progress.value(half);
        assert!(close(at, 0.5));
        // Restored halfway: no jump, and the way back takes half the time.
        progress.turn(half, false);
        assert!(close(progress.value(half), at));
        assert!(progress.value(after(half, 0.15)) < at);
        assert!(!progress.is_done(after(half, 0.45)));
        let back = after(half, 0.5);
        assert!(progress.is_done(back) && progress.value(back) == 0.0);
        // Minimized again before the end: back toward 1 from where it is.
        let mut progress = Progress::new(ms(0), false);
        let turn = after(ms(0), 0.2);
        progress.turn(turn, true);
        assert!(close(progress.value(turn), 0.8));
        assert!(!progress.is_done(after(turn, 0.19)));
        assert!(progress.is_done(after(turn, 0.2) + ms(1)));
        // A genie drawn on both sides of the turn is the same.
        let before = genie(at).bounds();
        let after = genie(
            Progress {
                from: at,
                start: ms(0),
                minimizing: false,
            }
            .value(ms(0)),
        );
        assert_eq!(after.bounds(), before);
    }

    #[test]
    fn the_effect_is_read_from_its_name() {
        assert_eq!("genie".parse::<Effect>().ok(), Some(Effect::Genie));
        assert_eq!("scale".parse::<Effect>().ok(), Some(Effect::Scale));
        assert_eq!("none".parse::<Effect>().ok(), Some(Effect::None));
        assert!("lamp".parse::<Effect>().is_err());
        assert_eq!(Effect::default(), Effect::Genie);
    }
}
