//! Desktop rendering with Smithay's GLES renderer and three in-house programs (`src/shaders`).
//!
//! Elements go from front to back, as Smithay expects them: the anchored surfaces of the
//! overlay and top layers, the snap preview, then each window, from the topmost to the lowest,
//! with its menus, its border, its title bar when the compositor draws it (`decorations`), its
//! content clipped to a rounded rectangle and its shadow, the anchored surfaces of the bottom
//! and background layers, and last the wallpaper, or the background discs without one. The
//! desktop color fills the rest. Each screen has its own elements, in frame pixels, at its
//! scale. See "Rendering" in `docs/compositor.md`.

use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

use sherlock_ui::{
    color::{self, Color},
    decorations::{self as ui_decorations, Colors},
    metrics,
    motion::{self, Fade},
    window as ui,
};
use smithay::{
    backend::renderer::Frame as _,
    backend::renderer::{
        element::{
            Element, Id, Kind, RenderElement, UnderlyingStorage, render_elements,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            texture::TextureRenderElement,
            utils::RescaleRenderElement,
        },
        gles::{
            GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, GlesTexture,
            Uniform, UniformName, UniformType, element::PixelShaderElement,
        },
        utils::{CommitCounter, DamageSet, OpaqueRegions},
    },
    desktop::{PopupManager, Window, layer_map_for_output},
    output::Output,
    reexports::wayland_server::{Resource as _, protocol::wl_surface::WlSurface},
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{compositor::with_states, seat::WaylandFocus, shell::wlr_layer::Layer},
};

use crate::{
    State,
    anim::Visual,
    blur,
    damage::{Ids, Key, Part, Stamped, params_hash},
    decorations::{self, Bar, Look},
    genie, is_override_redirect, mission_control, osd,
    output::View,
    wallpaper, window_anim, window_data,
};

/// No fill: only the border of a shape is drawn.
const NO_FILL: [f32; 4] = [0.0; 4];

render_elements! {
    /// Everything a desktop frame contains.
    pub SceneElement<=GlesRenderer>;
    Shape=Stamped<PixelShaderElement>,
    Window=RoundedSurface,
    Surface=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Wallpaper=TextureRenderElement<GlesTexture>,
    Genie=Stamped<genie::GenieElement>,
    Decoration=Stamped<TextureRenderElement<GlesTexture>>,
    Thumbnail=RoundedSurface<TextureRenderElement<GlesTexture>>,
    Blur=blur::BlurElement,
}

/// Programs compiled once and for all when rendering starts.
pub struct Scene {
    shape: GlesPixelProgram,
    shadow: GlesPixelProgram,
    clip: GlesTexProgram,
    genie: genie::Painter,
    /// Textures of the title bars the compositor draws (lot D-08).
    bars: RefCell<decorations::Textures>,
    /// Textures of the titles of Mission Control (lot D-33).
    labels: RefCell<mission_control::Labels>,
    /// Texture of the on-screen display of the media keys (lot D-35).
    osd: RefCell<osd::Texture>,
    /// The blur behind glass surfaces (lot D-37).
    pub blur: blur::Painter,
}

/// What a screen shows, from front to back, before it becomes render elements: plain data,
/// built without a renderer. The tests and the bench turn it into elements of their own.
#[derive(Debug)]
pub enum Item {
    /// A surface tree drawn as is (anchored surface, menu, X11 menu): built at scale 1 from
    /// `origin`, then scaled by `scale` around it.
    Surface {
        surface: WlSurface,
        origin: Point<i32, Physical>,
        scale: f64,
        alpha: f32,
    },
    /// The surface tree of a window, whose geometry starts at `geometry` in its buffer,
    /// stretched into `rect` and clipped to the rounded rectangle `clip`: the whole window,
    /// with its title bar when the compositor draws one.
    Window {
        surface: WlSurface,
        rect: Rectangle<i32, Physical>,
        clip: Rectangle<i32, Physical>,
        geometry: Rectangle<i32, Logical>,
        alpha: f32,
        radius: f32,
    },
    /// A rounded rectangle: `fill` (premultiplied) inside, `border` over `border_width` pixels
    /// along the edge, inside the rectangle like a CSS border.
    Shape {
        key: Key,
        rect: Rectangle<i32, Physical>,
        fill: [f32; 4],
        border: [f32; 4],
        border_width: f32,
        radius: f32,
        alpha: f32,
    },
    /// The shadow of a window, of `color` (premultiplied).
    Shadow {
        key: Key,
        shadow: Shadow,
        color: [f32; 4],
        radius: f32,
        alpha: f32,
    },
    /// The wallpaper of the screen.
    Wallpaper,
    /// A window minimized into its dock icon or coming back from it (lot D-19).
    Genie(genie::Frame),
    /// The title bar the compositor draws above a window, in `rect` (lot D-08).
    Decoration {
        key: Key,
        rect: Rectangle<i32, Physical>,
        look: Look,
        alpha: f32,
    },
    /// A title of Mission Control in `rect` (lot D-33).
    Label {
        key: Key,
        rect: Rectangle<i32, Physical>,
        label: mission_control::Label,
        alpha: f32,
    },
    /// The on-screen display of the media keys in `rect` (lot D-35).
    Osd {
        key: Key,
        rect: Rectangle<i32, Physical>,
        look: osd::Look,
        alpha: f32,
    },
    /// The wallpaper of the screen in `rect`, cut to its rounded corners: a workspace thumbnail
    /// of Mission Control.
    Thumbnail {
        rect: Rectangle<i32, Physical>,
        radius: f32,
    },
    /// The blur behind the glass surface `key`, over `rects` of the frame (lot D-37).
    Blur {
        key: Key,
        rects: Vec<Rectangle<i32, Physical>>,
        alpha: f32,
    },
    /// The last image of a window that closes, stretched into `rect` (lot D-34).
    Snapshot {
        key: Key,
        texture: GlesTexture,
        rect: Rectangle<i32, Physical>,
        alpha: f32,
    },
}

impl Item {
    /// Hash of what a shape, a shadow or a title bar draws, for `Ids::stamp`. The opacity is
    /// in it too, although the damage tracker sees it: an element of the tests, which has none,
    /// still changes with it.
    pub fn params(&self) -> u64 {
        match self {
            Item::Shape {
                fill,
                border,
                border_width,
                radius,
                alpha,
                ..
            } => params_hash(&[&fill[..], border, &[*border_width, *radius, *alpha]].concat()),
            Item::Shadow {
                shadow,
                color,
                radius,
                alpha,
                ..
            } => {
                let scalars = [*radius, shadow.sigma, *alpha];
                params_hash(&[&shadow.caster[..], &shadow.window, color, &scalars].concat())
            }
            Item::Genie(frame) => frame.params(),
            Item::Decoration { look, alpha, .. } => look.hash() ^ params_hash(&[*alpha]),
            Item::Label { label, alpha, .. } => label.hash() ^ params_hash(&[*alpha]),
            Item::Osd { look, alpha, .. } => look.hash() ^ params_hash(&[*alpha]),
            _ => 0,
        }
    }
}

impl Scene {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let uniform = |name, type_| UniformName::new(name, type_);
        let shape = renderer.compile_custom_pixel_shader(
            include_str!("shaders/shape.frag"),
            &[
                uniform("fill", UniformType::_4f),
                uniform("border", UniformType::_4f),
                uniform("radius", UniformType::_1f),
                uniform("border_width", UniformType::_1f),
            ],
        )?;
        let shadow = renderer.compile_custom_pixel_shader(
            include_str!("shaders/shadow.frag"),
            &[
                uniform("color", UniformType::_4f),
                uniform("caster", UniformType::_4f),
                uniform("window", UniformType::_4f),
                uniform("radius", UniformType::_1f),
                uniform("sigma", UniformType::_1f),
            ],
        )?;
        let clip = renderer.compile_custom_texture_shader(
            include_str!("shaders/clip.frag"),
            &[
                uniform("clip_rect", UniformType::_4f),
                uniform("radius", UniformType::_1f),
            ],
        )?;
        Ok(Self {
            shape,
            shadow,
            clip,
            genie: genie::Painter::new(renderer)?,
            bars: RefCell::default(),
            labels: RefCell::default(),
            osd: RefCell::default(),
            blur: blur::Painter::new(renderer),
        })
    }

    /// Elements of each screen of `views`, in its own frame pixels, at time `now`, and whether
    /// an animation is running: a fade of the theme or of the wallpaper counts. The wallpaper
    /// of each screen is uploaded once as a texture per variant; the textures no screen of
    /// `state` uses any more are freed. The elements the
    /// compositor draws itself take their identity from `ids`. Shared by the windowed backend
    /// and the real session.
    #[allow(clippy::too_many_arguments)]
    pub fn screens(
        &self,
        renderer: &mut GlesRenderer,
        ids: &mut Ids,
        state: &mut State,
        textures: &mut wallpaper::Textures,
        views: &[(Output, View)],
        preview: Option<Visual>,
        now: Duration,
    ) -> (Vec<Vec<SceneElement>>, bool) {
        let mut animating = state.tick_theme();
        textures.update(renderer, &mut state.wallpaper);
        let mut screens = Vec::new();
        for (output, view) in views {
            let image = state.wallpaper.image_for(view.frame.size);
            let (wallpaper, fading) = textures.layers(&output.name(), image.as_deref(), now);
            let shown = !wallpaper.is_empty();
            let (items, moving) = items(state, (output, view), preview, shown, now);
            animating |= moving || fading;
            let mode = state.wallpaper.mode;
            self.blur.begin_screen(output.name());
            let elements: Vec<_> = items
                .into_iter()
                .flat_map(|item| self.elements(renderer, ids, item, view, &wallpaper, mode))
                .collect();
            blur::track(&elements);
            screens.push(elements);
        }
        ids.sweep();
        self.blur.sweep();
        self.genie.sweep();
        self.bars.borrow_mut().sweep();
        self.labels.borrow_mut().sweep();
        let all = state.views();
        let used: Vec<_> = all
            .iter()
            .filter_map(|(_, view)| state.wallpaper.image_for(view.frame.size))
            .collect();
        let names: Vec<_> = all.iter().map(|(output, _)| output.name()).collect();
        textures.retain(&used, &names);
        (screens, animating)
    }

    /// The render elements of one item.
    pub(crate) fn elements(
        &self,
        renderer: &mut GlesRenderer,
        ids: &mut Ids,
        item: Item,
        view: &View,
        wallpaper: &[(wallpaper::Texture, f32)],
        mode: desktop_config::wallpaper::Mode,
    ) -> Vec<SceneElement> {
        let params = item.params();
        match item {
            Item::Surface {
                surface,
                origin,
                scale,
                alpha,
            } => {
                let tree: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        origin,
                        1.0,
                        alpha,
                        Kind::Unspecified,
                    );
                tree.into_iter()
                    .map(|element| {
                        SceneElement::Surface(RescaleRenderElement::from_element(
                            element, origin, scale,
                        ))
                    })
                    .collect()
            }
            Item::Window {
                surface,
                rect,
                clip,
                geometry,
                alpha,
                radius,
            } => {
                let stretch = Scale {
                    x: f64::from(rect.size.w) / f64::from(geometry.size.w),
                    y: f64::from(rect.size.h) / f64::from(geometry.size.h),
                };
                let tree: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        &surface,
                        rect.loc - geometry.loc.to_physical(1),
                        1.0,
                        alpha,
                        Kind::Unspecified,
                    );
                tree.into_iter()
                    .map(|surface| {
                        SceneElement::Window(RoundedSurface {
                            inner: RescaleRenderElement::from_element(surface, rect.loc, stretch),
                            program: self.clip.clone(),
                            clip: clip.to_f64(),
                            radius,
                            frame_size: view.frame_size,
                        })
                    })
                    .collect()
            }
            Item::Shape {
                key,
                rect,
                fill,
                border,
                border_width,
                radius,
                alpha,
            } => {
                let shape = PixelShaderElement::new(
                    self.shape.clone(),
                    frame_area(rect),
                    None,
                    alpha,
                    vec![
                        Uniform::new("fill", fill),
                        Uniform::new("border", border),
                        Uniform::new("radius", radius),
                        Uniform::new("border_width", border_width),
                    ],
                    Kind::Unspecified,
                );
                vec![SceneElement::Shape(ids.stamp(key, params, shape))]
            }
            Item::Shadow {
                key,
                shadow,
                color,
                radius,
                alpha,
            } => {
                let element = PixelShaderElement::new(
                    self.shadow.clone(),
                    frame_area(shadow.area),
                    None,
                    alpha,
                    vec![
                        Uniform::new("color", color),
                        Uniform::new("caster", shadow.caster),
                        Uniform::new("window", shadow.window),
                        Uniform::new("radius", radius),
                        Uniform::new("sigma", shadow.sigma),
                    ],
                    Kind::Unspecified,
                );
                vec![SceneElement::Shape(ids.stamp(key, params, element))]
            }
            Item::Wallpaper => wallpaper
                .iter()
                .map(|(texture, alpha)| {
                    SceneElement::Wallpaper(wallpaper::element(texture, *alpha, mode, view))
                })
                .collect(),
            Item::Genie(mut frame) => {
                // Its content goes into a texture: identities of its own, out of the screen's.
                let (inside, mut own) = (frame.view(), Ids::default());
                let content: Vec<_> = std::mem::take(&mut frame.content)
                    .into_iter()
                    .flat_map(|item| self.elements(renderer, &mut own, item, &inside, &[], mode))
                    .collect();
                let element = self.genie.element(renderer, ids, &frame, &content);
                element.map(SceneElement::Genie).into_iter().collect()
            }
            Item::Decoration {
                key,
                rect,
                look,
                alpha,
            } => {
                let mut bars = self.bars.borrow_mut();
                let bar = bars.element(renderer, key.clone(), (&look, look.hash()), rect, alpha);
                bar.map(|bar| SceneElement::Decoration(ids.stamp(key, params, bar)))
                    .into_iter()
                    .collect()
            }
            Item::Label {
                key,
                rect,
                label,
                alpha,
            } => {
                let mut labels = self.labels.borrow_mut();
                let label = labels.element(renderer, key.clone(), &label, rect, alpha);
                label
                    .map(|label| SceneElement::Decoration(ids.stamp(key, params, label)))
                    .into_iter()
                    .collect()
            }
            Item::Osd {
                key,
                rect,
                look,
                alpha,
            } => {
                let element = self.osd.borrow_mut().element(renderer, &look, rect, alpha);
                element
                    .map(|element| SceneElement::Decoration(ids.stamp(key, params, element)))
                    .into_iter()
                    .collect()
            }
            Item::Thumbnail { rect, radius } => wallpaper
                .iter()
                .map(|(texture, alpha)| {
                    let inside = View {
                        frame: rect,
                        ..*view
                    };
                    SceneElement::Thumbnail(RoundedSurface {
                        inner: wallpaper::element(texture, *alpha, mode, &inside),
                        program: self.clip.clone(),
                        clip: rect.to_f64(),
                        radius,
                        frame_size: view.frame_size,
                    })
                })
                .collect(),
            Item::Blur { key, rects, alpha } => {
                let element = self.blur.element(renderer, key, &rects, alpha, view);
                element.map(SceneElement::Blur).into_iter().collect()
            }
            Item::Snapshot {
                key,
                texture,
                rect,
                alpha,
            } => {
                let element = window_anim::element(renderer, texture, rect, alpha);
                vec![SceneElement::Decoration(ids.stamp(key, params, element))]
            }
        }
    }
}

/// What the screen `output` shows, seen through `view`, at time `now`, from front to back, and
/// whether an animation is running. Items are in frame pixels, at the screen's scale: corners,
/// borders and shadows follow that scale. `preview` is in the space. `wallpaper`: the screen's
/// wallpaper is loaded; the discs of the board are drawn otherwise.
pub fn items(
    state: &State,
    (output, view): (&Output, &View),
    preview: Option<Visual>,
    wallpaper: bool,
    now: Duration,
) -> (Vec<Item>, bool) {
    // Locked: the lock surfaces only, or the plain background (lot D-25a).
    if state.is_locked() {
        return crate::lock::items(state, (output, view));
    }
    let mut items = Vec::new();
    let mut animating = preview.is_some();
    let scale = view.scale as f32;
    // The area of a screenshot being chosen, then the on-screen display of the media keys, in
    // front of everything (lot D-35).
    // The rings of the clicks shown, in front of everything (lot C-01).
    animating |= crate::clicks::push_rings(&mut items, state, (output, view), now);
    crate::capture::push_area(&mut items, state, (output, view));
    animating |= osd::push_item(&mut items, state, (output, view));
    // The shell's menus closing (lot UI-06), in front of their anchored surfaces.
    animating |= crate::popup_anim::push_closing(&mut items, state, (output, view), now);
    push_layers(&mut items, output, view, &[Layer::Overlay, Layer::Top]);
    // A window minimized or restored goes under the dock, into its icon (lots D-19, Q-14).
    animating |= genie::push_effects(&mut items, state, (output, view), now);
    if let Some(preview) = preview {
        items.push(Item::Shape {
            key: Key::of(None, Part::Preview),
            rect: view.rect(preview.rect),
            fill: ui::snap_preview_fill().premultiplied(),
            border: ui::snap_preview_border().premultiplied(),
            border_width: ui::SNAP_PREVIEW_BORDER_WIDTH as f32 * scale,
            radius: corner_radius(preview.rect.size.to_i32_round()) * scale,
            alpha: preview.opacity,
        });
    }
    animating |= state.workspaces_sliding(now);
    // Mission Control draws the windows of its own (lot D-33).
    let overview = mission_control::push_items(&mut items, state, (output, view), now);
    animating |= overview.unwrap_or(false);
    // Windows closing (lot D-34): their last image, in front of the others.
    animating |= window_anim::push_closing(&mut items, state, (output, view), now);
    // The buttons of a title bar, hovered or pressed (lot C-01).
    animating |= state.decorations_animating(now);
    for window in state.space.elements().rev().filter(|_| overview.is_none()) {
        // Minimized, or on a workspace out of view.
        let Some(shift) = state.workspace_shift(window, now) else {
            continue;
        };
        // Restored, it is drawn by its effect, in front.
        if state.minimize_progress(window, now).is_some() {
            continue;
        }
        // The shadow stays on the window's own screen: a maximized window does not cast it
        // on the next one. While sliding, the window itself does not reach the next screen.
        let own = state.window_output(window).as_ref() == Some(output);
        let Some(mut visual) = state
            .window_visual(window, now)
            .filter(|_| own || shift == 0.0)
        else {
            continue;
        };
        visual.rect.loc.x += shift;
        animating |= window_data(window).motion.borrow().is_animating();
        animating |= popups_appearing(window);
        let bar = state.decoration_bar_at(window, now);
        push_window(&mut items, (window, bar), visual, view, own);
    }
    push_layers(
        &mut items,
        output,
        view,
        &[Layer::Bottom, Layer::Background],
    );
    // Last, so drawn first: what the blur behind glass surfaces (lot D-37) reads from the
    // frame, like the rest of the desktop.
    if wallpaper {
        items.push(Item::Wallpaper);
        return (items, animating);
    }
    let discs = desktop_discs(view.area.size);
    for (i, (rect, color)) in discs.into_iter().enumerate().rev() {
        let rect = view.rect(Rectangle::new(rect.loc + view.area.loc, rect.size).to_f64());
        items.push(Item::Shape {
            key: Key::of(None, Part::Disc(i)),
            rect,
            fill: color.premultiplied(),
            border: color.premultiplied(),
            border_width: 0.0,
            radius: rect.size.w as f32 / 2.0,
            alpha: 1.0,
        });
    }
    (items, animating)
}

/// Anchored surfaces of `layers`, the front one first, each with its menus in front.
fn push_layers(items: &mut Vec<Item>, output: &Output, view: &View, layers: &[Layer]) {
    let map = layer_map_for_output(output);
    for layer in layers {
        for surface in map.layers_on(*layer).rev() {
            let Some(geometry) = map.layer_geometry(surface) else {
                continue;
            };
            let loc = (geometry.loc - surface.bbox().loc + view.area.loc).to_f64();
            let at = Placement {
                view,
                loc,
                alpha: 1.0,
            };
            push_surface(items, surface.wl_surface(), at, true);
        }
    }
}

/// The rectangles of the frame that cover `rect` rounded by `radius` (lot D-37b).
pub(crate) fn rounded_rects(
    rect: Rectangle<i32, Physical>,
    radius: f32,
) -> Vec<Rectangle<i32, Physical>> {
    let Rectangle { loc, size } = rect;
    sherlock_ui::blur::rounded_region(loc.x, loc.y, size.w, size.h, radius)
        .into_iter()
        .map(|[x, y, w, h]| Rectangle::new((x, y).into(), (w, h).into()))
        .collect()
}

/// The blur `surface` asks for behind its window drawn in `rect` from `geometry`, cut to the
/// window's rounded `frame` (lot D-37b): Qt's translucent windows, our glass windows.
fn window_blur(
    surface: &WlSurface,
    (rect, geometry): (Rectangle<i32, Physical>, Rectangle<i32, Logical>),
    (frame, radius): (Rectangle<i32, Physical>, f32),
) -> Vec<Rectangle<i32, Physical>> {
    let region = crate::background_effect::region(surface);
    if region.is_empty() {
        return Vec::new();
    }
    let shape = rounded_rects(frame, radius);
    let sx = f64::from(rect.size.w) / f64::from(geometry.size.w);
    let sy = f64::from(rect.size.h) / f64::from(geometry.size.h);
    let x = |v: i32| (f64::from(rect.loc.x) + f64::from(v - geometry.loc.x) * sx).round() as i32;
    let y = |v: i32| (f64::from(rect.loc.y) + f64::from(v - geometry.loc.y) * sy).round() as i32;
    region
        .into_iter()
        .flat_map(|r| {
            let (left, top) = (x(r.loc.x), y(r.loc.y));
            let (right, bottom) = (x(r.loc.x + r.size.w), y(r.loc.y + r.size.h));
            let r = Rectangle::new((left, top).into(), (right - left, bottom - top).into());
            shape.iter().filter_map(move |s| s.intersection(r))
        })
        .collect()
}

/// A surface tree, untouched (no clip, border or shadow), placed by `at`, with the blur it asks
/// for just behind it (lot D-37). `popups`: its menus too, in front of it (smithay lists nested
/// menus with the root surface, so a menu never has menus of its own here).
fn push_surface(items: &mut Vec<Item>, surface: &WlSurface, at: Placement<'_>, popups: bool) {
    if popups {
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let loc = at.loc + (offset - popup.geometry().loc).to_f64();
            let at = Placement { loc, ..at };
            push_surface(items, popup.wl_surface(), at, false);
        }
    }
    let Placement { view, loc, alpha } = at;
    // Built at scale 1 from the origin, then scaled around it: subsurfaces keep their logical
    // offsets, multiplied by the screen's scale.
    items.push(Item::Surface {
        surface: surface.clone(),
        origin: view.point(loc).to_i32_round(),
        scale: view.scale,
        alpha,
    });
    let region = crate::background_effect::region(surface);
    if !region.is_empty() {
        let rects = region
            .into_iter()
            .map(|r| view.rect(Rectangle::new(loc + r.loc.to_f64(), r.size.to_f64())))
            .collect();
        let key = Key::of(Some(surface.id()), Part::Blur);
        items.push(Item::Blur { key, rects, alpha });
    }
}

pub(crate) fn push_window(
    items: &mut Vec<Item>,
    (window, bar): (&Window, Option<Bar>),
    visual: Visual,
    view: &View,
    shadow: bool,
) {
    let Some(surface) = window.wl_surface() else {
        return;
    };
    let surface = surface.as_ref();
    let (rect, geometry) = (view.rect(visual.rect), window.geometry());
    if rect.size.is_empty() || geometry.size.is_empty() {
        return;
    }
    // X11 menu or tooltip: no clip, no border, no shadow, like a Wayland menu.
    if is_override_redirect(window) {
        let at = Placement {
            view,
            loc: visual.rect.loc,
            alpha: 1.0,
        };
        push_surface(items, surface, at, false);
        return;
    }
    // Menus go in front, without clipping or scaling. They appear with a short fade, without
    // moving (lot C-01).
    for (popup, offset) in PopupManager::popups_for_surface(surface) {
        let at = Placement {
            view,
            loc: visual.rect.loc + (offset - popup.geometry().loc).to_f64(),
            alpha: visual.opacity * popup_alpha(popup.wl_surface()).0,
        };
        push_surface(items, popup.wl_surface(), at, false);
    }
    // A maximized or fullscreen window has no corners, border or shadow: they shrink and fade
    // on a spring as it gets there, and come back as it leaves (`Visual::decoration`).
    let decoration = visual.decoration;
    let decorated = decoration > 0.0;
    let scale = view.scale as f32;
    // While it opens, the whole window is scaled: its bar, corners and shadow with it.
    let opening = visual.scale;
    // The title bar the compositor draws sits above the content: the corners, the border, the
    // shadow and the clip go around both.
    let inset = bar
        .as_ref()
        .map_or(0.0, |_| f64::from(ui_decorations::INSET) * opening);
    let mut whole = visual.rect;
    whole.loc.y -= inset;
    whole.size.h += inset;
    let frame = view.rect(whole);
    let at_rest = whole.size.to_f64().downscale(opening).to_i32_round();
    let radius = corner_radius(at_rest) * scale * opening as f32 * decoration;
    let owner = Some(surface.id());
    if decorated {
        items.push(Item::Shape {
            key: Key::of(owner.clone(), Part::Border),
            rect: frame,
            fill: NO_FILL,
            border: color::palette().border.premultiplied(),
            border_width: metrics::BORDER_WIDTH * scale,
            radius,
            alpha: visual.opacity * decoration,
        });
    }
    if let Some(bar) = bar {
        whole.size.h = inset;
        let rect = view.rect(whole);
        // Drawn at its size at rest, stretched while the window opens: one texture, not one
        // per frame.
        let size = match opening == 1.0 {
            true => rect.size,
            false => whole
                .size
                .downscale(opening)
                .to_physical(view.scale)
                .to_i32_round(),
        };
        let look = Look {
            bar,
            size: (size.w.max(1) as u32, size.h.max(1) as u32),
            scale,
            radius: radius / opening as f32,
            colors: Colors::of(color::palette()),
        };
        items.push(Item::Decoration {
            key: Key::of(owner.clone(), Part::TitleBar),
            rect,
            look,
            alpha: visual.opacity,
        });
    }
    // The content is drawn in `rect`, the drawn geometry, in frame pixels: the screen's scale,
    // and the animation's if there is one.
    items.push(Item::Window {
        surface: surface.clone(),
        rect,
        clip: frame,
        geometry,
        alpha: visual.opacity,
        radius,
    });
    let rects = window_blur(surface, (rect, geometry), (frame, radius));
    if !rects.is_empty() {
        let key = Key::of(owner.clone(), Part::Blur);
        items.push(Item::Blur {
            key,
            rects,
            alpha: visual.opacity,
        });
    }
    if decorated && shadow {
        items.push(Item::Shadow {
            key: Key::of(owner, Part::Shadow),
            shadow: Shadow::of(frame, view.scale * opening),
            color: color::palette().shadow.premultiplied(),
            radius,
            alpha: visual.opacity * decoration,
        });
    }
}

/// When a menu or popup of a window was first drawn with content.
struct PopupShown(Instant);

/// The opacity of the menu or popup `surface` now, and whether it is still appearing: from
/// transparent to opaque over `motion::MENU_FADE` from its first drawing with content.
fn popup_alpha(surface: &WlSurface) -> (f32, bool) {
    if !crate::layer::has_content(surface) {
        return (1.0, false);
    }
    let shown = with_states(surface, |states| {
        states
            .data_map
            .get_or_insert(|| PopupShown(Instant::now()))
            .0
    });
    let since = shown.elapsed();
    let fade = Fade::new(0.0, 1.0, Duration::ZERO, motion::MENU_FADE);
    (fade.value(since), since < motion::MENU_FADE)
}

/// A menu or popup of `window` is appearing: a frame at each blank.
fn popups_appearing(window: &Window) -> bool {
    let Some(surface) = window.wl_surface() else {
        return false;
    };
    PopupManager::popups_for_surface(&surface).any(|(popup, _)| popup_alpha(popup.wl_surface()).1)
}

/// Where a surface tree is drawn: the screen it is drawn for, its origin in the space, its
/// opacity.
#[derive(Clone, Copy)]
struct Placement<'a> {
    view: &'a View,
    loc: Point<f64, Logical>,
    alpha: f32,
}

/// Area of a pixel shader element. The frame is drawn at scale 1: its "logical" coordinates
/// are its pixels, and every element is already placed and sized in them.
fn frame_area(rect: Rectangle<i32, Physical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (rect.loc.x, rect.loc.y).into(),
        (rect.size.w, rect.size.h).into(),
    )
}

/// Corner radius of a window, reduced for a window smaller than its two corners.
pub fn corner_radius(size: Size<i32, Logical>) -> f32 {
    let half = size.w.min(size.h) as f32 / 2.0;
    metrics::WINDOW_RADIUS.min(half.max(0.0))
}

/// Background discs for a screen of `size`, with their color in the palette shown: the places of
/// the Main board, stretched with the screen. The diameter follows the larger of the two ratios: the discs stay
/// round and cover the same regions.
pub fn desktop_discs(size: Size<i32, Logical>) -> [(Rectangle<i32, Logical>, Color); 3] {
    let (width, height) = metrics::REFERENCE_SCREEN;
    let (sx, sy) = (size.w as f32 / width, size.h as f32 / height);
    let scale = sx.max(sy);
    std::array::from_fn(|i| {
        let (left, top, diameter) = metrics::DESKTOP_DISC_GEOMETRY[i];
        let d = diameter * scale;
        let center = ((left + diameter / 2.0) * sx, (top + diameter / 2.0) * sy);
        let rect = Rectangle::new(
            (
                (center.0 - d / 2.0).round() as i32,
                (center.1 - d / 2.0).round() as i32,
            )
                .into(),
            (d.round() as i32, d.round() as i32).into(),
        );
        (rect, color::palette().desktop_discs[i])
    })
}

/// Shadow of a window, like `box-shadow: 0 28px 70px` in CSS: the window shifted down
/// (`caster`), blurred by a Gaussian whose standard deviation is half the blur. The element
/// extends by the blur's reach around it. `caster` and `window` are given in the element.
#[derive(Debug, PartialEq)]
pub struct Shadow {
    pub area: Rectangle<i32, Physical>,
    pub caster: [f32; 4],
    pub window: [f32; 4],
    pub sigma: f32,
}

impl Shadow {
    /// Shadow of a window drawn at `rect` in the frame, on a screen at `scale`: offset and blur
    /// follow the scale.
    pub fn of(window: Rectangle<i32, Physical>, scale: f64) -> Self {
        let blur = (f64::from(metrics::SHADOW_BLUR) * scale).round() as i32;
        let offset = (f64::from(metrics::SHADOW_OFFSET) * scale).round() as i32;
        let area = Rectangle::new(
            window.loc + Point::from((-blur, offset - blur)),
            window.size + Size::from((2 * blur, 2 * blur)),
        );
        let (w, h) = (window.size.w as f32, window.size.h as f32);
        let blur = blur as f32;
        Self {
            area,
            caster: [blur, blur, w, h],
            window: [blur, blur - offset as f32, w, h],
            sigma: metrics::SHADOW_BLUR * scale as f32 / 2.0,
        }
    }
}

/// Rectangle `clip` of the frame in the buffer being drawn into, where `gl_FragCoord` counts
/// its pixels: after the frame transform. The windowed backend flips the frame (`Flipped180`):
/// the top of the frame is at the bottom of the buffer.
pub(crate) fn clip_in_frame(
    clip: Rectangle<f64, Physical>,
    transform: Transform,
    frame_size: Size<i32, Physical>,
) -> [f32; 4] {
    let clip = transform.transform_rect_in(clip, &frame_size.to_f64());
    [clip.loc.x, clip.loc.y, clip.size.w, clip.size.h].map(|v| v as f32)
}

/// The opaque part of a window's surface: its `regions`, relative to the element at `loc`, kept
/// inside `clip` and without the rounded corners of `radius`, which show what is underneath.
pub fn rounded_opaque(
    regions: OpaqueRegions<i32, Physical>,
    loc: Point<i32, Physical>,
    clip: Rectangle<f64, Physical>,
    radius: f32,
) -> OpaqueRegions<i32, Physical> {
    if regions.is_empty() {
        return regions;
    }
    let (x0, y0) = (clip.loc.x.ceil() as i32, clip.loc.y.ceil() as i32);
    let x1 = (clip.loc.x + clip.size.w).floor() as i32;
    let y1 = (clip.loc.y + clip.size.h).floor() as i32;
    let inside = Rectangle::new((x0 - loc.x, y0 - loc.y).into(), (x1 - x0, y1 - y0).into());
    // One more pixel for the smoothing of the edge.
    let r = radius.ceil() as i32 + 1;
    let (left, top) = (inside.loc.x, inside.loc.y);
    let (right, bottom) = (left + inside.size.w - r, top + inside.size.h - r);
    let corners = [(left, top), (right, top), (left, bottom), (right, bottom)]
        .map(|(x, y)| Rectangle::new((x, y).into(), (r, r).into()));
    regions
        .into_iter()
        .filter_map(|region| region.intersection(inside))
        .flat_map(|region| region.subtract_rects(corners))
        .collect()
}

/// A client surface clipped by the rounded rectangle of its window, or the wallpaper by that of
/// a workspace thumbnail. The texture program replaces Smithay's while it is drawn.
pub struct RoundedSurface<E = RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>> {
    inner: E,
    program: GlesTexProgram,
    /// Clip rectangle, in the frame, before its transform.
    clip: Rectangle<f64, Physical>,
    radius: f32,
    frame_size: Size<i32, Physical>,
}

impl<E: Element> Element for RoundedSurface<E> {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    // The client's opaque region, inside the clip and without the rounded corners, which show
    // what is underneath (lot P-01). What an opaque window covers is not drawn: the windows
    // below, its own shadow, the wallpaper under a maximized window.
    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        let loc = self.geometry(scale).loc;
        rounded_opaque(
            self.inner.opaque_regions(scale),
            loc,
            self.clip,
            self.radius,
        )
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl<E: RenderElement<GlesRenderer>> RenderElement<GlesRenderer> for RoundedSurface<E> {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        let clip = clip_in_frame(self.clip, frame.transformation(), self.frame_size);
        frame.override_default_tex_program(
            self.program.clone(),
            vec![
                Uniform::new("clip_rect", clip),
                Uniform::new("radius", self.radius),
            ],
        );
        let drawn = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        drawn
    }

    // Never put as is on a screen plane: the clip would be lost.
    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    fn frame_rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Physical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn the_discs_follow_the_main_board() {
        let discs = desktop_discs((1440, 900).into());
        assert_eq!(
            discs[0],
            (
                rect(-260, 340, 1100, 1100),
                color::Palette::DARK.desktop_discs[0]
            )
        );
        assert_eq!(
            discs[1],
            (
                rect(860, -420, 900, 900),
                color::Palette::DARK.desktop_discs[1]
            )
        );
        assert_eq!(
            discs[2],
            (
                rect(980, 520, 520, 520),
                color::Palette::DARK.desktop_discs[2]
            )
        );
        // On a screen twice as large, everything doubles.
        let large = desktop_discs((2880, 1800).into());
        assert_eq!(large[2].0, rect(1960, 1040, 1040, 1040));
        // On a wider screen, the discs stay round.
        let wide = desktop_discs((1920, 900).into());
        assert!(wide.iter().all(|(r, _)| r.size.w == r.size.h));
    }

    #[test]
    fn the_shadow_follows_box_shadow() {
        // Window of the Main board: 836 × 652 at (36, 64).
        let shadow = Shadow::of(frame_rect(36, 64, 836, 652), 1.0);
        // Shifted 28 down, extended by 70 on each side.
        assert_eq!(
            shadow.area,
            frame_rect(36 - 70, 64 + 28 - 70, 836 + 140, 652 + 140)
        );
        assert_eq!(shadow.caster, [70.0, 70.0, 836.0, 652.0]);
        // In the element, the window is 28 pixels above its shadow.
        assert_eq!(shadow.window, [70.0, 42.0, 836.0, 652.0]);
        assert_eq!(shadow.sigma, 35.0);
        // At scale 1.5, the same window takes 1.5 times the pixels, and so does its shadow.
        let shadow = Shadow::of(frame_rect(54, 96, 1254, 978), 1.5);
        assert_eq!(
            shadow.area,
            frame_rect(54 - 105, 96 + 42 - 105, 1254 + 210, 978 + 210)
        );
        assert_eq!(shadow.window, [105.0, 63.0, 1254.0, 978.0]);
        assert_eq!(shadow.sigma, 52.5);
    }

    #[test]
    fn the_view_scales_the_space_to_the_screen() {
        // Second screen of the frame, from 1440 pixels on, at scale 2: 720 points wide.
        let view = View {
            area: rect(720, 0, 720, 450),
            frame: frame_rect(1440, 0, 1440, 900),
            scale: 2.0,
            frame_size: (2880, 900).into(),
        };
        assert_eq!(
            view.rect(rect(730, 10, 100, 50).to_f64()),
            frame_rect(1460, 20, 200, 100)
        );
        // Two rectangles that touch in the space touch in the frame, even at 1.25.
        let view = View {
            scale: 1.25,
            ..view
        };
        let left = view.rect(Rectangle::new((720.0, 0.0).into(), (100.3, 10.0).into()));
        let right = view.rect(Rectangle::new((820.3, 0.0).into(), (50.0, 10.0).into()));
        assert_eq!(left.loc.x + left.size.w, right.loc.x);
    }

    #[test]
    fn the_clip_follows_the_frame_flip() {
        let clip = Rectangle::new((36.0, 64.0).into(), (836.0, 652.0).into());
        let size = (1440, 900).into();
        assert_eq!(
            clip_in_frame(clip, Transform::Normal, size),
            [36.0, 64.0, 836.0, 652.0]
        );
        // Flipped: the window at the top of the screen is at the top of the GL frame, whose
        // vertical axis goes up.
        assert_eq!(
            clip_in_frame(clip, Transform::Flipped180, size),
            [36.0, 900.0 - 64.0 - 652.0, 836.0, 652.0]
        );
    }

    #[test]
    fn an_opaque_window_hides_what_is_below_but_its_corners() {
        // A window of 100 × 80 at (10, 20), opaque everywhere, radius 16.
        let opaque = OpaqueRegions::from_slice(&[frame_rect(0, 0, 100, 80)]);
        let clip = Rectangle::new((10.0, 20.0).into(), (100.0, 80.0).into());
        let regions = rounded_opaque(opaque, (10, 20).into(), clip, 16.0);
        let area: i32 = regions.iter().map(|r| r.size.w * r.size.h).sum();
        // The four corners of 17 × 17 are not opaque, nor anything outside.
        assert_eq!(area, 100 * 80 - 4 * 17 * 17);
        assert!(
            regions
                .iter()
                .all(|r| frame_rect(0, 0, 100, 80).contains_rect(*r))
        );
        assert!(!regions.iter().any(|r| r.contains((0, 0))));
        // A translucent client declares nothing: nothing is hidden.
        let none = rounded_opaque(OpaqueRegions::default(), (10, 20).into(), clip, 16.0);
        assert!(none.is_empty());
    }

    #[test]
    fn the_corners_have_the_window_radius() {
        assert_eq!(corner_radius((836, 652).into()), 16.0);
        // A tiny window becomes a pill, never a negative radius.
        assert_eq!(corner_radius((100, 20).into()), 10.0);
        assert_eq!(corner_radius((0, 0).into()), 0.0);
    }
}
