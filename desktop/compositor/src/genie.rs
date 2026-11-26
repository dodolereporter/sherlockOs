//! Minimize and restore effect (lot D-19). Minimized, a window is sucked into its dock icon,
//! the rectangle the dock gives with `set_rectangle` (`State::icon_rectangle`): its sides bend
//! toward the icon, then it slides into it, squeezed. Activated, it comes back the same way,
//! backwards. Without an icon, it shrinks and fades toward the bottom of its screen. `Effect`
//! chooses between the two, or nothing.
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

use sherlock_ui::{decorations as ui_decorations, motion};
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

/// Share of the genie spent bending the sides toward the icon, before the window slides in.
const BEND: f64 = 0.4;
/// From this share on, the window fades into the icon it covers at the end.
const FADE_FROM: f64 = 0.85;

/// Slow start, slow landing (smoothstep), on `x` clamped to 0..1.
fn ease(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
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

/// The genie at `progress`, between `window` and `icon`, in frame pixels. `genie.frag` does the
/// same computation, backwards.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Genie {
    pub window: FrameRect,
    pub icon: FrameRect,
    pub progress: f64,
}

impl Genie {
    /// The icon is below the window: it goes down. Otherwise everything is mirrored.
    fn down(&self) -> bool {
        let center = |r: FrameRect| r.loc.y + r.size.h / 2.0;
        center(self.icon) >= center(self.window)
    }

    /// Window and icon as the computation sees them: mirrored when going up, so that the window
    /// always goes down.
    fn oriented(&self) -> (FrameRect, FrameRect) {
        let mirror = |r: FrameRect| Rectangle::new((r.loc.x, -r.loc.y - r.size.h).into(), r.size);
        if self.down() {
            (self.window, self.icon)
        } else {
            (mirror(self.window), mirror(self.icon))
        }
    }

    /// Where row `v` of the window (0 its top, 1 its bottom) is drawn: its height, then its left
    /// and right ends. The sides bend first, the window then slides into the icon: what is
    /// under the window's top keeps its width, what reaches the icon's top takes the icon's.
    pub fn row(&self, v: f64) -> (f64, f64, f64) {
        let (w, i) = self.oriented();
        let slide = ease((self.progress - BEND) / (1.0 - BEND));
        let top = mix(w.loc.y, i.loc.y, slide);
        let bottom = mix(w.loc.y + w.size.h, i.loc.y + i.size.h, slide);
        let down = self.down();
        let y = mix(top, bottom, if down { v } else { 1.0 - v });
        let bend = ease(self.progress / BEND);
        let reach = ease((y - w.loc.y) / (i.loc.y - w.loc.y).max(1.0));
        let f = bend * reach;
        let left = mix(w.loc.x, i.loc.x, f);
        let right = mix(w.loc.x + w.size.w, i.loc.x + i.size.w, f);
        (if down { y } else { -y }, left, right)
    }

    /// Area the deformed window covers, one pixel more around for the smoothing of its edges.
    /// The ends of each side move monotonically with the height: the first and last rows hold
    /// the extremes.
    pub fn bounds(&self) -> Rectangle<i32, Physical> {
        let (y0, l0, r0) = self.row(0.0);
        let (y1, l1, r1) = self.row(1.0);
        let (x0, x1) = (l0.min(l1).floor() - 1.0, r0.max(r1).ceil() + 1.0);
        let (top, bottom) = (y0.min(y1).floor() - 1.0, y0.max(y1).ceil() + 1.0);
        Rectangle::new(
            (x0 as i32, top as i32).into(),
            ((x1 - x0) as i32, (bottom - top) as i32).into(),
        )
    }

    /// Opacity: the window fades into its icon at the end.
    pub fn alpha(&self) -> f32 {
        (1.0 - ease((self.progress - FADE_FROM) / (1.0 - FADE_FROM))) as f32
    }

    /// Uniforms of `genie.frag` for the element covering `area`.
    fn uniforms(&self, area: Rectangle<i32, Physical>) -> Vec<Uniform<'static>> {
        let (w, i) = self.oriented();
        let rect = |r: FrameRect| [r.loc.x, r.loc.y, r.size.w, r.size.h].map(|v| v as f32);
        let area = [area.loc.x, area.loc.y, area.size.w, area.size.h].map(|v| v as f32);
        vec![
            Uniform::new("window_rect", rect(w)),
            Uniform::new("icon_rect", rect(i)),
            Uniform::new("area", area),
            Uniform::new("progress", self.progress as f32),
            Uniform::new("mirror", if self.down() { 1.0f32 } else { -1.0 }),
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Shape {
    Genie,
    Scale,
}

/// The effect running on a window, kept in its user data, and the last icon rectangle seen,
/// in the space: the dock may forget it on the way.
#[derive(Default)]
struct Track {
    running: std::cell::Cell<Option<(Progress, Shape)>>,
    icon: std::cell::Cell<Option<Rectangle<i32, Logical>>>,
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
            (Some((mut progress, shape)), _) => {
                progress.turn(now, minimizing);
                Some((progress, shape))
            }
            (None, Effect::None) => None,
            (None, effect) => {
                let icon = self.icon_in_space(window);
                track.icon.set(icon);
                let genie = effect == Effect::Genie && icon.is_some();
                let shape = if genie { Shape::Genie } else { Shape::Scale };
                Some((Progress::new(now, minimizing), shape))
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

    /// The icon rectangle of `window` in the space.
    fn icon_in_space(&self, window: &Window) -> Option<Rectangle<i32, Logical>> {
        let (name, rect) = self.icon_rectangle(window)?;
        let output = self.space.outputs().find(|o| o.name() == name)?;
        let origin = self.space.output_geometry(output)?.loc;
        Some(Rectangle::new(rect.loc + origin, rect.size))
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
        let Some((progress, shape)) = track.running.get() else {
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
        match (shape, icon) {
            (Shape::Genie, Some(icon)) => {
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
    (content_geometry, icon): (Rectangle<i32, Logical>, Rectangle<i32, Logical>),
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
        progress,
    };
    let area = genie.bounds();
    if !area.overlaps(view.frame) {
        return;
    }
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
    };
    render::push_window(
        &mut content,
        (window, bar),
        rest,
        &offscreen(geometry, size, view.scale),
        false,
    );
    let owner = Some(surface.id());
    items.push(Item::Genie(Frame {
        key: Key::of(owner.clone(), Part::Genie),
        content,
        size,
        scale: view.scale,
        area,
        genie,
        alpha: genie.alpha(),
    }));
    let fade = (1.0 - ease(progress / BEND)) as f32 * decoration;
    if fade > 0.0 {
        let radius = render::corner_radius(geometry.size) * view.scale as f32 * decoration;
        items.push(Item::Shadow {
            key: Key::of(owner, Part::Shadow),
            shadow: Shadow::of(rect, view.scale),
            color: sherlock_ui::color::palette().shadow.premultiplied(),
            radius,
            alpha: fade,
        });
    }
}

/// The view of a texture of `size` pixels showing `area` of the space at `scale`.
fn offscreen(area: Rectangle<i32, Logical>, size: Size<i32, Physical>, scale: f64) -> View {
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
        let values = [
            rects[0],
            rects[1],
            [g.progress, f64::from(self.alpha), 0.0, 0.0],
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
                uniform("icon_rect", UniformType::_4f),
                uniform("area", UniformType::_4f),
                uniform("progress", UniformType::_1f),
                uniform("mirror", UniformType::_1f),
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
        let element = GenieElement::new(&self.program, texture, &frame.genie, frame.alpha);
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
    /// `texture`, the window as at rest, at the step `genie`, with the opacity `alpha`.
    pub fn new(program: &Program, texture: GlesTexture, genie: &Genie, alpha: f32) -> Self {
        let area = genie.bounds();
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

    /// A window of 800 × 600 at (320, 100), its icon of 52 × 52 in the dock at the bottom.
    fn genie(progress: f64) -> Genie {
        Genie {
            window: rect(320.0, 100.0, 800.0, 600.0),
            icon: rect(900.0, 830.0, 52.0, 52.0),
            progress,
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn the_genie_leaves_the_window_and_ends_in_the_icon() {
        // At 0: the window, every row at full width.
        for v in [0.0, 0.5, 1.0] {
            let (y, left, right) = genie(0.0).row(v);
            assert!(close(y, 100.0 + 600.0 * v), "{v}");
            assert!(close(left, 320.0) && close(right, 1120.0), "{v}");
        }
        assert_eq!(genie(0.0).alpha(), 1.0);
        // At 1: exactly the icon's rectangle.
        for v in [0.0, 0.5, 1.0] {
            let (y, left, right) = genie(1.0).row(v);
            assert!(close(y, 830.0 + 52.0 * v), "{v}");
            assert!(close(left, 900.0) && close(right, 952.0), "{v}");
        }
        assert_eq!(
            genie(1.0).bounds(),
            Rectangle::new((899, 829).into(), (54, 54).into())
        );
        // At 0.5: the window slides down, its bottom already narrower than its top, and both
        // within the way from the window to the icon.
        let half = genie(0.5);
        let (top, l0, r0) = half.row(0.0);
        let (bottom, l1, r1) = half.row(1.0);
        assert!(top > 100.0 && top < 830.0 && bottom > 700.0 && bottom < 882.0);
        assert!(r1 - l1 < r0 - l0, "the bottom is narrower");
        assert!(l1 > 320.0 && l1 < 900.0 && r1 > 952.0 && r1 < 1120.0);
        // The rows stay in order: the window is never turned over.
        let rows: Vec<f64> = (0..=10).map(|i| half.row(f64::from(i) / 10.0).0).collect();
        assert!(rows.windows(2).all(|r| r[0] < r[1]));
        let area = half.bounds();
        assert!(
            area.contains((l0 as i32, top as i32)) && area.contains((r1 as i32, bottom as i32))
        );
    }

    #[test]
    fn a_dock_above_the_window_mirrors_the_genie() {
        let up = Genie {
            window: rect(320.0, 200.0, 800.0, 600.0),
            icon: rect(900.0, 10.0, 52.0, 52.0),
            progress: 1.0,
        };
        // The window's top lands on the icon's top, its bottom on the icon's bottom.
        let (top, left, right) = up.row(0.0);
        assert!(close(top, 10.0) && close(left, 900.0) && close(right, 952.0));
        assert!(close(up.row(1.0).0, 62.0));
        // At 0 it is the window, the right way up.
        let rest = Genie {
            progress: 0.0,
            ..up
        };
        assert!(close(rest.row(0.0).0, 200.0) && close(rest.row(1.0).0, 800.0));
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
