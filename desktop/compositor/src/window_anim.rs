//! Opening and closing of the windows (lots D-34, C-01), as on macOS. Opening, from its first
//! content, a window grows from `motion::WINDOW_OPEN_SCALE` to 1 around its center and fades in
//! (`anim::WindowMotion`). Closing, its last image shrinks to `motion::WINDOW_CLOSE_SCALE` and
//! fades out over `motion::WINDOW_CLOSE` on an ease-in, its shadow with it, then is freed.
//!
//! The client has already destroyed its window when the closing starts: its buffer goes away
//! with its surface. So, when its toplevel goes (`toplevel_destroyed`, or an X11 unmap), the
//! window is drawn as at rest into a texture of its own, through the offscreen path of the
//! genie (`genie::offscreen`), by the backend, which has the renderer (`Snapshotter`). That
//! texture is the ghost drawn during the closing. The ghost is out of the space: it takes no
//! input, the pointer goes through it.
//!
//! No animation for menus, X11 override-redirect windows, anchored surfaces, fullscreen
//! windows, nor with `--window-animations off`. See "Opening and closing" in
//! `docs/compositor.md`.

use std::time::Duration;

use sherlock_ui::{decorations as ui_decorations, motion};
use smithay::{
    backend::allocator::Fourcc,
    backend::renderer::{
        Bind as _, Frame as _, Offscreen as _, Renderer as _, Texture as _,
        element::texture::TextureRenderElement,
        element::{Element as _, Kind, RenderElement as _, texture::TextureBuffer},
        gles::{GlesError, GlesRenderer, GlesTexture},
    },
    desktop::Window,
    output::Output,
    reexports::wayland_server::{Resource as _, backend::ObjectId},
    utils::{Logical, Physical, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus as _,
};

use crate::{
    State,
    anim::{self, Visual},
    damage::{Ids, Key, Part},
    genie,
    handlers::Arrangement,
    is_override_redirect,
    output::View,
    render::{self, Item, Scene, SceneElement, Shadow},
};

/// Draws a window as at rest into a texture: given by the backend, which has the renderer.
pub type Snapshotter = Box<dyn FnMut(&State, &Window) -> Option<GlesTexture>>;

/// The last image of a closed window, while it closes.
struct Ghost {
    owner: ObjectId,
    /// The whole window, title bar included, in the space.
    rect: Rectangle<i32, Logical>,
    /// `None` without a renderer (the tests): nothing is drawn, the closing still runs.
    texture: Option<GlesTexture>,
    fade: motion::Fade,
    decoration: f32,
    workspace: String,
}

/// The setting, the windows closing, and the backend's way to keep their last image.
#[derive(Default)]
pub struct WindowAnims {
    off: bool,
    ghosts: Vec<Ghost>,
    snapshotter: Option<Snapshotter>,
}

/// Scale and opacity of a closing window at `fade`, the opacity going from 1 to 0.
fn closing(fade: &motion::Fade, now: Duration) -> (f64, f32) {
    let alpha = fade.value(now).clamp(0.0, 1.0);
    let end = motion::WINDOW_CLOSE_SCALE;
    let scale = end + (1.0 - end) * f64::from(alpha);
    (scale, alpha)
}

/// The fade of a closing that starts at `now`.
fn close_fade(now: Duration) -> motion::Fade {
    let (duration, ease) = (motion::WINDOW_CLOSE, motion::WINDOW_CLOSE_EASE);
    motion::Fade::eased(1.0, 0.0, now, duration, ease)
}

impl State {
    /// Turns the opening and closing animations on or off (`--window-animations`), for
    /// Control Room. A window already closing finishes.
    pub fn set_window_animations(&mut self, on: bool) {
        self.window_anims.off = !on;
    }

    pub fn window_animations(&self) -> bool {
        !self.window_anims.off
    }

    /// Plugs in the backend's way to draw a window into a texture.
    pub fn set_snapshotter(&mut self, snapshotter: Snapshotter) {
        self.window_anims.snapshotter = Some(snapshotter);
    }

    /// Windows whose closing is drawn, and how many of them still hold a texture.
    pub fn closing_windows(&self) -> (usize, usize) {
        let ghosts = &self.window_anims.ghosts;
        let textures = ghosts.iter().filter(|g| g.texture.is_some()).count();
        (ghosts.len(), textures)
    }

    /// The window opens at once, without its animation: fullscreen, or animations off.
    pub(crate) fn opens_at_once(&self, window: &Window) -> bool {
        self.window_anims.off || Arrangement::of(window).fullscreen
    }

    /// `window` goes away: its client destroyed it, or unmapped it (X11). Keeps its last image
    /// and starts its closing, if it is shown and animated. Call it before unmapping it.
    pub(crate) fn start_closing(&mut self, window: &Window) {
        // Its buttons stop moving, and the pointer forgets them (lot C-01).
        self.forget_decoration(window);
        let animated = !self.window_anims.off
            && !is_override_redirect(window)
            && crate::window_data(window).shown.get()
            && self.is_visible(window)
            && self.minimize_progress(window, self.now()).is_none()
            && !self.overview.is_open()
            && !Arrangement::of(window).fullscreen;
        if !animated {
            return;
        }
        let (Some(surface), Some(geometry)) = (window.wl_surface(), whole_geometry(self, window))
        else {
            return;
        };
        let texture = match self.window_anims.snapshotter.take() {
            Some(mut snapshot) => {
                let texture = snapshot(self, window);
                self.window_anims.snapshotter = Some(snapshot);
                // Nothing left to draw: the window goes at once.
                let Some(texture) = texture else { return };
                Some(texture)
            }
            None => None,
        };
        let arrangement = Arrangement::of(window);
        let ghost = Ghost {
            owner: surface.id(),
            rect: geometry,
            texture,
            fade: close_fade(self.now()),
            decoration: if arrangement.maximized { 0.0 } else { 1.0 },
            workspace: self.workspace_of(window),
        };
        self.window_anims.ghosts.push(ghost);
        self.queue_redraw();
    }

    /// Frees the closings over at this time, and their textures. Called after each loop
    /// iteration.
    pub(crate) fn end_closings(&mut self) {
        let now = self.now();
        self.window_anims.ghosts.retain(|g| !g.fade.is_done(now));
    }
}

/// The window and its title bar when the compositor draws one, in the space.
fn whole_geometry(state: &State, window: &Window) -> Option<Rectangle<i32, Logical>> {
    let mut geometry = state.space.element_geometry(window)?;
    if state.decoration_bar(window).is_some() {
        let inset = ui_decorations::INSET.round() as i32;
        geometry.loc.y -= inset;
        geometry.size.h += inset;
    }
    (!geometry.size.is_empty()).then_some(geometry)
}

/// Items of the windows closing at `now`, seen on `output` through `view`, the last closed in
/// front, and whether one runs. They go in front of the other windows.
pub(crate) fn push_closing(
    items: &mut Vec<Item>,
    state: &State,
    (_output, view): (&Output, &View),
    now: Duration,
) -> bool {
    let mut running = false;
    let active = state.workspaces().active();
    for ghost in state.window_anims.ghosts.iter().rev() {
        // Over: `end_closings` frees it; this frame clears its place.
        if ghost.fade.is_done(now) || ghost.workspace != active {
            continue;
        }
        running = true;
        let Some(texture) = &ghost.texture else {
            continue;
        };
        let (scale, alpha) = closing(&ghost.fade, now);
        let rect = view.rect(anim::scaled(ghost.rect.to_f64(), scale));
        if !rect.overlaps(view.frame) {
            continue;
        }
        let owner = Some(ghost.owner.clone());
        items.push(Item::Snapshot {
            key: Key::of(owner.clone(), Part::Closing),
            texture: texture.clone(),
            rect,
            alpha,
        });
        if ghost.decoration > 0.0 {
            // The shadow shrinks with the window: its offset and blur too.
            let radius = render::corner_radius(ghost.rect.size)
                * (view.scale * scale) as f32
                * ghost.decoration;
            items.push(Item::Shadow {
                key: Key::of(owner, Part::Shadow),
                shadow: Shadow::of(rect, view.scale * scale),
                color: sherlock_ui::color::palette().shadow.premultiplied(),
                radius,
                alpha: alpha * ghost.decoration,
            });
        }
    }
    running
}

/// The element of a ghost's texture, stretched into `rect`.
pub(crate) fn element(
    renderer: &mut GlesRenderer,
    texture: GlesTexture,
    rect: Rectangle<i32, Physical>,
    alpha: f32,
) -> TextureRenderElement<GlesTexture> {
    let whole = texture.size();
    let buffer = TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
    let size = Size::<i32, Logical>::from((rect.size.w, rect.size.h));
    // The whole texture, stretched into `rect`: without `src`, smithay takes the part of the
    // texture of the size of `rect`, and a ghost smaller than at rest was cut, not shrunk
    // (its right and bottom edges, corners and border gone, seen in lot C-01).
    let src = Rectangle::from_size((f64::from(whole.w), f64::from(whole.h)).into());
    TextureRenderElement::from_texture_buffer(
        rect.loc.to_f64(),
        &buffer,
        Some(alpha),
        Some(src),
        Some(size),
        Kind::Unspecified,
    )
}

/// `window` drawn as at rest, title bar, border and corners included, into a texture of its
/// size in pixels at the scale of its screen, as the genie does. The backends call it from
/// their `Snapshotter`.
pub fn snapshot(
    scene: &Scene,
    renderer: &mut GlesRenderer,
    state: &State,
    window: &Window,
) -> Option<GlesTexture> {
    let content = state.space.element_geometry(window)?;
    let whole = whole_geometry(state, window)?;
    let scale = state
        .window_output(window)
        .map_or(1.0, |o| o.current_scale().fractional_scale());
    let size = whole.size.to_f64().to_physical(scale).to_i32_round();
    let view = genie::offscreen(whole, size, scale);
    let arrangement = Arrangement::of(window);
    let bare = arrangement.maximized || arrangement.fullscreen;
    let rest = Visual {
        rect: content.to_f64(),
        opacity: 1.0,
        moving: false,
        decoration: if bare { 0.0 } else { 1.0 },
        scale: 1.0,
    };
    let mut items = Vec::new();
    let bar = state.decoration_bar(window);
    render::push_window(&mut items, (window, bar), rest, &view, false);
    let mut ids = Ids::default();
    let mode = state.wallpaper.mode;
    let elements: Vec<SceneElement> = items
        .into_iter()
        .flat_map(|item| scene.elements(renderer, &mut ids, item, &view, &[], mode))
        .collect();
    draw(renderer, size, &elements, [0.0; 4])
        .inspect_err(|err| tracing::warn!("closing: drawing the window: {err}"))
        .ok()
}

/// Draws `elements`, from front to back, into a new texture of `size` filled with `clear`
/// (premultiplied).
pub(crate) fn draw(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
    elements: &[SceneElement],
    clear: [f32; 4],
) -> Result<GlesTexture, GlesError> {
    let buffer_size = (size.w.max(1), size.h.max(1)).into();
    let mut texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, buffer_size)?;
    let mut target = renderer.bind(&mut texture)?;
    let mut frame = renderer.render(&mut target, size, Transform::Normal)?;
    frame.clear(clear.into(), &[Rectangle::from_size(size)])?;
    for element in elements.iter().rev() {
        let dst = element.geometry(Scale::from(1.0));
        let damage = [Rectangle::from_size(dst.size)];
        element.draw(&mut frame, element.src(), dst, &damage, &[])?;
    }
    // Sampled next in the same GL context, which keeps the order: nothing to wait for.
    let _ = frame.finish()?;
    drop(target);
    Ok(texture)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_closing_window_shrinks_and_fades_out_in_180_ms_on_an_ease_in() {
        let fade = close_fade(ms(1000));
        assert_eq!(closing(&fade, ms(1000)), (1.0, 1.0));
        let mut last = (1.0, 1.0);
        let mut steps = Vec::new();
        for t in [1030, 1060, 1090, 1120, 1150] {
            let (scale, alpha) = closing(&fade, ms(t));
            assert!(scale < last.0 && alpha < last.1, "{t}");
            assert!(scale > motion::WINDOW_CLOSE_SCALE && alpha > 0.0, "{t}");
            steps.push(last.1 - alpha);
            last = (scale, alpha);
        }
        // Ease-in: each step takes more than the one before.
        assert!(steps.windows(2).all(|w| w[1] > w[0]), "{steps:?}");
        let (scale, alpha) = closing(&fade, ms(1180));
        assert!((scale - motion::WINDOW_CLOSE_SCALE).abs() < 1e-9 && alpha == 0.0);
        assert!(fade.is_done(ms(1180)) && !fade.is_done(ms(1179)));
    }
}
