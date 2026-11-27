//! Blur behind glass (lot D-37, ADR 0027). A client asks for it over a region of its surface
//! (`background_effect.rs`); the scene puts a `BlurElement` just behind that surface.
//!
//! When what is behind changes in its area, the element is damaged whole, so the damage tracker
//! redraws what is behind; the element then copies that part of the frame at half its size and
//! blurs it with a dual Kawase filter (`sherlock_ui::blur::PASSES` halvings, as many doublings)
//! into textures of its own. Otherwise it draws its textures again: nothing is computed at rest.
//! Without GLES 3, on the software renderer or after a GL error, it lays a veil instead. See
//! "Blur behind glass" in `docs/compositor.md`.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::CStr,
    rc::Rc,
    time::Instant,
};

use sherlock_ui::{blur as tokens, color};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Frame as _, Offscreen as _, Texture as _,
            element::{Element, Id, Kind, RenderElement, UnderlyingStorage},
            gles::{
                Capability, GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture,
                Uniform, UniformName, UniformType, ffi,
            },
            utils::{CommitCounter, DamageSet, OpaqueRegions},
        },
    },
    utils::{Buffer, Physical, Point, Rectangle, Scale, Size, Transform},
};

use crate::{
    damage::Key,
    output::View,
    render::{SceneElement, clip_in_frame},
};

/// What glass gets behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Blur,
    /// A veil of the desktop color.
    Veil,
    /// Nothing, as before lot D-37: the before of the boards.
    Off,
}

/// Environment variable that chooses what glass gets behind it, whatever the renderer:
/// `blur`, `veil` or `off`. For trials and boards only.
pub const ENV: &str = "SHERLOCKOS_BLUR";

impl Mode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "blur" => Some(Self::Blur),
            "veil" => Some(Self::Veil),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

/// One pass of the filter: a GL program of our own, and where its uniforms are.
struct Pass {
    program: ffi::types::GLuint,
    tex: ffi::types::GLint,
    half_texel: ffi::types::GLint,
}

/// The programs of the filter. They live as long as the renderer.
struct Passes {
    down: Pass,
    up: Pass,
    /// A GL error came up: no more blur in this session.
    failed: Cell<bool>,
    /// The fence set after the last blur not judged yet, and when (lot D-37b).
    fence: Cell<Option<(ffi::types::GLsync, Instant)>>,
    pace: Cell<Pace>,
    /// Whether the pace is judged: not once the tests chose the mode.
    judged: Cell<bool>,
    /// The chip was too slow: the veil for the rest of the session.
    slow: Cell<bool>,
}

/// How many blurs in a row the graphics chip finished late (lot D-37b).
#[derive(Debug, Default, Clone, Copy)]
struct Pace {
    late: u32,
}

impl Pace {
    /// One more blur judged, `late` or in time: whether the chip is now too slow for the blur.
    fn judge(&mut self, late: bool) -> bool {
        self.late = if late { self.late + 1 } else { 0 };
        self.late >= tokens::LATE_IN_A_ROW
    }
}

/// The blur of one surface on one screen, kept from frame to frame.
struct Cache {
    id: Id,
    commit: Cell<CommitCounter>,
    /// What is behind changed: the textures are to be computed again.
    stale: Cell<bool>,
    /// Drawn once without the whole area redrawn behind it: computed at the next draw anyway.
    retried: Cell<bool>,
    /// What was behind at the last frame, by element.
    seen: RefCell<HashMap<Id, Seen>>,
    /// Half of the area, then each halving.
    textures: RefCell<Vec<GlesTexture>>,
    /// The area and the region, as drawn at the last frame.
    shape: RefCell<Shape>,
    used: Cell<bool>,
}

/// An area in the frame, and the region inside it.
type Shape = (Rectangle<i32, Physical>, Vec<Rectangle<i32, Physical>>);

#[derive(Debug, PartialEq)]
struct Seen {
    commit: CommitCounter,
    geometry: Rectangle<i32, Physical>,
    alpha: u32,
}

/// Programs, mode and caches of the blur: one per renderer.
pub struct Painter {
    passes: Option<Rc<Passes>>,
    composite: Option<GlesTexProgram>,
    mode: Cell<Mode>,
    caches: RefCell<HashMap<(String, Key), Rc<Cache>>>,
    /// The screen whose elements are being built.
    screen: RefCell<String>,
}

impl Painter {
    /// Compiles the programs. Blur needs GLES 3 (`glBlitFramebuffer`, `gl_VertexID`) and a
    /// graphics chip: on the software renderer the veil is cheaper and reads as well.
    pub fn new(renderer: &mut GlesRenderer) -> Self {
        let gles3 = renderer.capabilities().contains(&Capability::Blit);
        let name = renderer.with_context(renderer_name).unwrap_or_default();
        let software = ["llvmpipe", "softpipe", "swrast"]
            .iter()
            .any(|s| name.contains(s));
        let passes = gles3
            .then(|| renderer.with_context(compile_passes).ok().flatten())
            .flatten();
        let composite = renderer
            .compile_custom_texture_shader(
                include_str!("shaders/blur.frag"),
                &[
                    UniformName::new("area", UniformType::_4f),
                    UniformName::new("saturation", UniformType::_1f),
                ],
            )
            .inspect_err(|err| tracing::warn!("blur behind glass: {err}"))
            .ok();
        let ready = passes.is_some() && composite.is_some();
        let mode = if ready && !software {
            Mode::Blur
        } else {
            Mode::Veil
        };
        tracing::info!(?mode, renderer = name, "behind glass");
        let painter = Self {
            passes: passes.filter(|_| ready).map(Rc::new),
            composite,
            mode: Cell::new(mode),
            caches: RefCell::default(),
            screen: RefCell::default(),
        };
        // A trial on the software renderer may ask for the blur all the same (lot D-37b).
        let asked = std::env::var(ENV).ok().and_then(|v| Mode::parse(&v));
        if let Some(mode) = asked {
            tracing::info!(?mode, "behind glass, asked by {ENV}");
            painter.set_mode(mode);
        }
        painter
    }

    pub fn mode(&self) -> Mode {
        match self
            .passes
            .as_ref()
            .is_some_and(|p| p.failed.get() || p.slow.get())
        {
            true => Mode::Veil,
            false => self.mode.get(),
        }
    }

    /// Blur, if the renderer can, or the veil: the tests blur on the software renderer.
    pub fn set_mode(&self, mode: Mode) {
        if let Some(passes) = &self.passes {
            passes.judged.set(false);
            passes.slow.set(false);
        }
        let possible = self.passes.is_some() || mode != Mode::Blur;
        self.mode.set(if possible { mode } else { Mode::Veil });
    }

    pub(crate) fn begin_screen(&self, name: String) {
        *self.screen.borrow_mut() = name;
    }

    /// The element behind the surface of `key`, over `rects` of the frame, cut to `view`.
    pub(crate) fn element(
        &self,
        renderer: &mut GlesRenderer,
        key: Key,
        rects: &[Rectangle<i32, Physical>],
        alpha: f32,
        view: &View,
    ) -> Option<BlurElement> {
        if self.mode() == Mode::Off {
            return None;
        }
        let rects: Vec<_> = rects
            .iter()
            .filter_map(|r| r.intersection(view.frame))
            .collect();
        let area = rects.iter().copied().reduce(|a, b| a.merge(b))?;
        let rects: Vec<_> = rects
            .iter()
            .map(|r| Rectangle::new(r.loc - area.loc, r.size))
            .collect();
        let screen = self.screen.borrow().clone();
        let mut caches = self.caches.borrow_mut();
        let cache = caches.entry((screen, key)).or_insert_with(|| {
            Rc::new(Cache {
                id: Id::new(),
                commit: Cell::default(),
                stale: Cell::new(true),
                retried: Cell::new(false),
                seen: RefCell::default(),
                textures: RefCell::default(),
                shape: RefCell::default(),
                used: Cell::new(false),
            })
        });
        cache.used.set(true);
        let mode = self.mode();
        let shape = (area, rects.clone());
        if *cache.shape.borrow() != shape {
            let resized = cache.shape.borrow().0.size != area.size;
            *cache.shape.borrow_mut() = shape;
            cache.stale.set(true);
            bump(cache);
            if resized || cache.textures.borrow().is_empty() {
                cache.textures.borrow_mut().clear();
            }
        }
        if mode == Mode::Blur && cache.textures.borrow().is_empty() {
            match textures(renderer, area.size) {
                Ok(textures) => *cache.textures.borrow_mut() = textures,
                Err(err) => {
                    tracing::warn!("blur behind glass, textures: {err}");
                    return None;
                }
            }
        }
        let veil = color::palette().desktop.premultiplied();
        Some(BlurElement {
            cache: cache.clone(),
            area,
            rects,
            alpha,
            mode,
            frame_size: view.frame_size,
            passes: self.passes.clone(),
            composite: self.composite.clone(),
            veil: veil.map(|c| c * tokens::VEIL_OPACITY),
        })
    }

    /// End of a frame: the blurs no screen drew any more are freed.
    pub(crate) fn sweep(&self) {
        self.caches
            .borrow_mut()
            .retain(|_, cache| cache.used.replace(false));
    }
}

fn bump(cache: &Cache) {
    let mut commit = cache.commit.get();
    commit.increment();
    cache.commit.set(commit);
}

/// Sizes of the textures of an area: its half, then each halving.
fn levels(size: Size<i32, Physical>) -> Vec<(i32, i32)> {
    let half = |v: i32| ((v + 1) / 2).max(1);
    let first = (half(size.w), half(size.h));
    std::iter::successors(Some(first), |&(w, h)| Some((half(w), half(h))))
        .take(tokens::PASSES + 1)
        .collect()
}

fn textures(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Result<Vec<GlesTexture>, GlesError> {
    let textures = levels(size)
        .into_iter()
        .map(|(w, h)| renderer.create_buffer(Fourcc::Abgr8888, (w, h).into()))
        .collect::<Result<Vec<GlesTexture>, _>>()?;
    let ids: Vec<_> = textures.iter().map(GlesTexture::tex_id).collect();
    renderer.with_context(|gl| {
        for id in ids {
            // SAFETY: the renderer's context is current inside `with_context`, and `id` is a
            // texture of that context, just made.
            unsafe {
                gl.BindTexture(ffi::TEXTURE_2D, id);
                for (name, value) in [
                    (ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE),
                    (ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE),
                    (ffi::TEXTURE_MIN_FILTER, ffi::LINEAR),
                    (ffi::TEXTURE_MAG_FILTER, ffi::LINEAR),
                ] {
                    gl.TexParameteri(ffi::TEXTURE_2D, name, value as i32);
                }
                gl.BindTexture(ffi::TEXTURE_2D, 0);
            }
        }
    })?;
    Ok(textures)
}

fn renderer_name(gl: &ffi::Gles2) -> String {
    // SAFETY: the context is current inside `with_context`; GL returns a static string or null.
    let name = unsafe { gl.GetString(ffi::RENDERER) };
    if name.is_null() {
        return String::new();
    }
    // SAFETY: not null, and GL strings end with a nul.
    unsafe { CStr::from_ptr(name.cast()) }
        .to_string_lossy()
        .into_owned()
}

fn compile_passes(gl: &ffi::Gles2) -> Option<Passes> {
    let vertex = include_str!("shaders/blur_vertex.glsl");
    let down = compile(gl, vertex, include_str!("shaders/blur_down.frag"));
    let up = compile(gl, vertex, include_str!("shaders/blur_up.frag"));
    if down.is_none() || up.is_none() {
        tracing::warn!("blur behind glass: the programs do not compile");
    }
    Some(Passes {
        down: down?,
        up: up?,
        failed: Cell::new(false),
        fence: Cell::new(None),
        pace: Cell::default(),
        judged: Cell::new(true),
        slow: Cell::new(false),
    })
}

/// A program of our own, out of Smithay's, with uniforms `tex` and `half_texel`. Inside
/// `with_context` only.
fn compile(gl: &ffi::Gles2, vertex: &str, fragment: &str) -> Option<Pass> {
    let shader = |kind, source: &str| {
        let (text, length) = (source.as_ptr().cast(), source.len() as i32);
        // SAFETY: the context is current; `text` and `length` describe `source`, alive here.
        unsafe {
            let shader = gl.CreateShader(kind);
            gl.ShaderSource(shader, 1, &text, &length);
            gl.CompileShader(shader);
            shader
        }
    };
    let (vs, fs) = (
        shader(ffi::VERTEX_SHADER, vertex),
        shader(ffi::FRAGMENT_SHADER, fragment),
    );
    // SAFETY: the context is current; the shaders were just made in it.
    unsafe {
        let program = gl.CreateProgram();
        gl.AttachShader(program, vs);
        gl.AttachShader(program, fs);
        gl.LinkProgram(program);
        gl.DetachShader(program, vs);
        gl.DetachShader(program, fs);
        gl.DeleteShader(vs);
        gl.DeleteShader(fs);
        let mut linked = 0;
        gl.GetProgramiv(program, ffi::LINK_STATUS, &mut linked);
        if linked == 0 {
            gl.DeleteProgram(program);
            return None;
        }
        Some(Pass {
            program,
            tex: gl.GetUniformLocation(program, c"tex".as_ptr()),
            half_texel: gl.GetUniformLocation(program, c"half_texel".as_ptr()),
        })
    }
}

impl Passes {
    /// Judges the last blur once it is old enough: if the chip has not finished it yet, its
    /// frame missed its slot. Then sets a fence after the blur being drawn, if none waits.
    /// Smithay's GL bindings have no timer queries; a fence is in GLES 3. Inside
    /// `with_context`, after the passes.
    fn pace(&self, gl: &ffi::Gles2) {
        if !self.judged.get() {
            return;
        }
        if let Some((fence, set)) = self.fence.get() {
            if set.elapsed() < tokens::LATE_AFTER {
                return;
            }
            // SAFETY: the context is current; `fence` was made in it and is deleted once.
            let status = unsafe {
                let status = gl.ClientWaitSync(fence, 0, 0);
                gl.DeleteSync(fence);
                status
            };
            self.fence.set(None);
            let mut pace = self.pace.get();
            if pace.judge(status == ffi::TIMEOUT_EXPIRED) && !self.slow.replace(true) {
                tracing::warn!("blur behind glass: the graphics chip is too slow, the veil now");
            }
            self.pace.set(pace);
        }
        // SAFETY: the context is current; the fence is deleted when it is judged.
        let fence = unsafe { gl.FenceSync(ffi::SYNC_GPU_COMMANDS_COMPLETE, 0) };
        if !fence.is_null() {
            self.fence.set(Some((fence, Instant::now())));
        }
    }

    /// Copies `source` of the frame being drawn (in its buffer, `[x, y, w, h]`) into the first
    /// texture at half its size, then halves it down the textures and doubles it back up. The
    /// GL state Smithay's frame relies on is put back. Inside `with_context` only.
    fn run(&self, gl: &ffi::Gles2, source: [f32; 4], textures: &[(u32, (i32, i32))]) {
        let n = textures.len();
        let [x, y, w, h] = source.map(|v| v.round() as i32);
        let (mut draw, mut read, mut viewport) = (0, 0, [0; 4]);
        let mut fbos = vec![0; n];
        // SAFETY: the context is current inside `with_context`; every name used was made here
        // or read from GL; the frame's framebuffers, viewport, scissor and blending are put back.
        unsafe {
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut draw);
            gl.GetIntegerv(ffi::READ_FRAMEBUFFER_BINDING, &mut read);
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
            let scissor = gl.IsEnabled(ffi::SCISSOR_TEST) == ffi::TRUE;
            gl.Disable(ffi::SCISSOR_TEST);
            gl.Disable(ffi::BLEND);
            gl.GenFramebuffers(n as i32, fbos.as_mut_ptr());
            for (fbo, (texture, _)) in fbos.iter().zip(textures) {
                gl.BindFramebuffer(ffi::FRAMEBUFFER, *fbo);
                let (target, attach) = (ffi::FRAMEBUFFER, ffi::COLOR_ATTACHMENT0);
                gl.FramebufferTexture2D(target, attach, ffi::TEXTURE_2D, *texture, 0);
            }
            let (w0, h0) = textures[0].1;
            gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, read as u32);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbos[0]);
            let (bit, linear) = (ffi::COLOR_BUFFER_BIT, ffi::LINEAR);
            gl.BlitFramebuffer(x, y, x + w, y + h, 0, 0, w0, h0, bit, linear);
            gl.ActiveTexture(ffi::TEXTURE0);
            let step = |pass: &Pass, from: usize, to: usize| {
                let ((texture, (fw, fh)), (tw, th)) = (textures[from], textures[to].1);
                gl.BindFramebuffer(ffi::FRAMEBUFFER, fbos[to]);
                gl.Viewport(0, 0, tw, th);
                gl.BindTexture(ffi::TEXTURE_2D, texture);
                gl.UseProgram(pass.program);
                gl.Uniform1i(pass.tex, 0);
                let half = 0.5 * tokens::OFFSET;
                gl.Uniform2f(pass.half_texel, half / fw as f32, half / fh as f32);
                gl.DrawArrays(ffi::TRIANGLES, 0, 3);
            };
            for i in 1..n {
                step(&self.down, i - 1, i);
            }
            for i in (0..n - 1).rev() {
                step(&self.up, i + 1, i);
            }
            if gl.GetError() != ffi::NO_ERROR {
                tracing::warn!("blur behind glass: GL error, the veil from now on");
                self.failed.set(true);
            }
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            gl.UseProgram(0);
            gl.BindFramebuffer(ffi::READ_FRAMEBUFFER, read as u32);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, draw as u32);
            gl.DeleteFramebuffers(n as i32, fbos.as_ptr());
            let [vx, vy, vw, vh] = viewport;
            gl.Viewport(vx, vy, vw, vh);
            if scissor {
                gl.Enable(ffi::SCISSOR_TEST);
            }
            gl.Enable(ffi::BLEND);
        }
    }
}

/// After the elements of a screen are built, front first: each blur learns whether what is
/// behind it changed in its area since the last frame. If so, or if its last computation is
/// still due, it is damaged whole.
pub(crate) fn track(elements: &[SceneElement]) {
    let one = Scale::from(1.0);
    for (i, element) in elements.iter().enumerate() {
        let SceneElement::Blur(blur) = element else {
            continue;
        };
        if blur.mode != Mode::Blur {
            continue;
        }
        let (cache, area) = (&blur.cache, blur.area);
        let mut before = cache.seen.borrow_mut();
        let mut seen = HashMap::new();
        let mut changed = false;
        for behind in &elements[i + 1..] {
            let geometry = behind.geometry(one);
            if !geometry.overlaps(area) {
                continue;
            }
            let now = Seen {
                commit: behind.current_commit(),
                geometry,
                alpha: behind.alpha().to_bits(),
            };
            changed |= match before.get(behind.id()) {
                Some(old) if old.geometry == geometry && old.alpha == now.alpha => {
                    old.commit != now.commit
                        && behind
                            .damage_since(one, Some(old.commit))
                            .iter()
                            .any(|d| Rectangle::new(d.loc + geometry.loc, d.size).overlaps(area))
                }
                _ => true,
            };
            seen.insert(behind.id().clone(), now);
        }
        // An element gone from behind.
        changed |= seen.len() != before.len();
        *before = seen;
        if changed {
            cache.stale.set(true);
        }
        if cache.stale.get() {
            bump(cache);
        }
    }
}

/// Whether `damage` covers the whole of `size`.
fn covers(damage: &[Rectangle<i32, Physical>], size: Size<i32, Physical>) -> bool {
    Rectangle::from_size(size)
        .subtract_rects(damage.iter().copied())
        .is_empty()
}

/// The blur behind one surface: its area in the frame, its region inside that area.
pub struct BlurElement {
    cache: Rc<Cache>,
    area: Rectangle<i32, Physical>,
    rects: Vec<Rectangle<i32, Physical>>,
    alpha: f32,
    mode: Mode,
    frame_size: Size<i32, Physical>,
    passes: Option<Rc<Passes>>,
    composite: Option<GlesTexProgram>,
    /// Premultiplied, at its opacity.
    veil: [f32; 4],
}

impl Element for BlurElement {
    fn id(&self) -> &Id {
        &self.cache.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.cache.commit.get()
    }

    fn location(&self, _: Scale<f64>) -> Point<i32, Physical> {
        self.area.loc
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size((self.area.size.w, self.area.size.h).into()).to_f64()
    }

    fn geometry(&self, _: Scale<f64>) -> Rectangle<i32, Physical> {
        self.area
    }

    fn damage_since(
        &self,
        _: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.current_commit()) {
            return DamageSet::default();
        }
        DamageSet::from_slice(&[Rectangle::from_size(self.area.size)])
    }

    // Never opaque: what is behind must be drawn before the blur reads it.
    fn opaque_regions(&self, _: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for BlurElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let region: Vec<_> = damage
            .iter()
            .flat_map(|d| self.rects.iter().filter_map(|r| r.intersection(*d)))
            .collect();
        let (Mode::Blur, Some(passes), Some(program)) = (self.mode, &self.passes, &self.composite)
        else {
            let veil = self.veil.map(|c| c * self.alpha);
            return frame.draw_solid(dst, &region, veil.into());
        };
        let area = clip_in_frame(dst.to_f64(), frame.transformation(), self.frame_size);
        let cache = &self.cache;
        let textures = cache.textures.borrow();
        if cache.stale.get() {
            // The frame behind is only whole where it was redrawn: read it then, or at the
            // second try, whatever covers the rest.
            if covers(damage, dst.size) || cache.retried.get() {
                let levels: Vec<_> = textures
                    .iter()
                    .map(|t| (t.tex_id(), (t.width() as i32, t.height() as i32)))
                    .collect();
                frame.with_context(|gl| {
                    passes.run(gl, area, &levels);
                    passes.pace(gl);
                })?;
                cache.stale.set(false);
                cache.retried.set(false);
            } else {
                cache.retried.set(true);
            }
        }
        let Some(picture) = textures.first() else {
            return Ok(());
        };
        let src = Rectangle::from_size(picture.size()).to_f64();
        let uniforms = [
            Uniform::new("area", area),
            Uniform::new("saturation", tokens::SATURATION),
        ];
        let normal = Transform::Normal;
        frame.render_texture_from_to(
            picture,
            src,
            dst,
            &region,
            &[],
            normal,
            self.alpha,
            Some(program),
            &uniforms,
        )
    }

    // Drawn by the GPU over the frame, never on a plane of its own.
    fn underlying_storage(&self, _: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_level_halves_the_last() {
        assert_eq!(
            levels((700, 77).into()),
            vec![(350, 39), (175, 20), (88, 10)]
        );
        assert_eq!(levels((1, 1).into()).len(), tokens::PASSES + 1);
    }

    #[test]
    fn only_late_blurs_in_a_row_make_the_chip_too_slow() {
        let mut pace = Pace::default();
        for _ in 1..tokens::LATE_IN_A_ROW {
            assert!(!pace.judge(true));
        }
        // One in time starts the count again.
        assert!(!pace.judge(false));
        for _ in 1..tokens::LATE_IN_A_ROW {
            assert!(!pace.judge(true));
        }
        assert!(pace.judge(true));
    }
}
