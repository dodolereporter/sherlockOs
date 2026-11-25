//! Desktop rendering with Smithay's GLES renderer and three in-house programs (`src/shaders`).
//!
//! Elements go from front to back, as Smithay expects them: the anchored surfaces of the
//! overlay and top layers, the snap preview, then each window, from the topmost to the lowest,
//! with its menus, its border, its content clipped to a rounded rectangle and its shadow, the
//! anchored surfaces of the bottom and background layers, and last the wallpaper, or the
//! background discs without one. The desktop color fills the rest. Each screen has its own elements, in frame pixels, at its
//! scale. See "Rendering" in `docs/compositor.md`.

use std::time::Duration;

use sherlock_ui::{
    color::{self, Color},
    metrics, window as ui,
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
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::{seat::WaylandFocus, shell::wlr_layer::Layer},
};

use crate::{
    State, anim::Visual, handlers::is_fullscreen, is_override_redirect, output::View, wallpaper,
    window_data,
};

/// No fill: only the border of a shape is drawn.
const NO_FILL: [f32; 4] = [0.0; 4];

render_elements! {
    /// Everything a desktop frame contains.
    pub SceneElement<=GlesRenderer>;
    Shape=PixelShaderElement,
    Window=RoundedSurface,
    Surface=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Wallpaper=TextureRenderElement<GlesTexture>,
}

/// Programs compiled once and for all when rendering starts.
pub struct Scene {
    shape: GlesPixelProgram,
    shadow: GlesPixelProgram,
    clip: GlesTexProgram,
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
        })
    }

    /// Elements of the screen `output`, seen through `view`, at time `now`, and whether an
    /// animation is running. Elements are in frame pixels, at the screen's scale: corners,
    /// borders and shadows follow that scale. `preview` is in the space. `wallpaper`: the
    /// texture of this screen, once loaded; the discs of the board otherwise.
    pub fn elements(
        &self,
        renderer: &mut GlesRenderer,
        state: &State,
        (output, view): (&Output, &View),
        preview: Option<Visual>,
        wallpaper: Option<&wallpaper::Texture>,
        now: Duration,
    ) -> (Vec<SceneElement>, bool) {
        let mut elements = Vec::new();
        let mut animating = preview.is_some();
        let scale = view.scale as f32;
        self.push_layers(
            renderer,
            &mut elements,
            output,
            view,
            &[Layer::Overlay, Layer::Top],
        );
        if let Some(preview) = preview {
            elements.push(SceneElement::Shape(self.shape(
                view.rect(preview.rect),
                ui::SNAP_PREVIEW_FILL.premultiplied(),
                ui::SNAP_PREVIEW_BORDER,
                ui::SNAP_PREVIEW_BORDER_WIDTH as f32 * scale,
                corner_radius(preview.rect.size.to_i32_round()) * scale,
                preview.opacity,
            )));
        }
        for window in state.space.elements().rev() {
            let Some(visual) = state.window_visual(window, now) else {
                continue;
            };
            animating |= window_data(window).motion.borrow().is_animating();
            // The shadow stays on the window's own screen: a maximized window does not cast it
            // on the next one.
            let shadow = state.window_output(window).as_ref() == Some(output);
            self.push_window(renderer, &mut elements, window, visual, view, shadow);
        }
        let below = [Layer::Bottom, Layer::Background];
        self.push_layers(renderer, &mut elements, output, view, &below);
        // Last, so drawn first: what the blur behind glass surfaces (lot D-11b) will read from
        // the frame, like the rest of the desktop.
        if let Some(texture) = wallpaper {
            let mode = state.wallpaper.mode;
            elements.push(SceneElement::Wallpaper(wallpaper::element(
                texture, mode, view,
            )));
            return (elements, animating);
        }
        let discs = desktop_discs(view.area.size);
        for (rect, color) in discs.into_iter().rev() {
            let rect = view.rect(Rectangle::new(rect.loc + view.area.loc, rect.size).to_f64());
            let radius = rect.size.w as f32 / 2.0;
            elements.push(SceneElement::Shape(self.shape(
                rect,
                color.premultiplied(),
                color,
                0.0,
                radius,
                1.0,
            )));
        }
        (elements, animating)
    }

    /// Anchored surfaces of `layers`, the front one first, each with its menus in front.
    fn push_layers(
        &self,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
        output: &Output,
        view: &View,
        layers: &[Layer],
    ) {
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
                self.push_surface(renderer, elements, surface.wl_surface(), at, true);
                self.push_backdrop(elements, view.rect(geometry.to_f64()));
            }
        }
    }

    /// Placeholder for the blur behind glass surfaces (lot D-11b): what is already drawn
    /// behind `rect` gets blurred here, just behind the surface. Nothing yet: the surface is
    /// drawn as is, translucent over a sharp background.
    fn push_backdrop(&self, _elements: &mut [SceneElement], _rect: Rectangle<i32, Physical>) {}

    /// A surface tree, untouched (no clip, border or shadow), placed by `at`. `popups`: its
    /// menus too, in front of it (smithay lists nested menus with the root surface, so a menu
    /// never has menus of its own here).
    fn push_surface(
        &self,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
        surface: &WlSurface,
        at: Placement<'_>,
        popups: bool,
    ) {
        if popups {
            for (popup, offset) in PopupManager::popups_for_surface(surface) {
                let loc = at.loc + (offset - popup.geometry().loc).to_f64();
                let at = Placement { loc, ..at };
                self.push_surface(renderer, elements, popup.wl_surface(), at, false);
            }
        }
        let Placement { view, loc, alpha } = at;
        // Built at scale 1 from the origin, then scaled around it: subsurfaces keep their
        // logical offsets, multiplied by the screen's scale.
        let origin = view.point(loc).to_i32_round();
        let tree: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                surface,
                origin,
                1.0,
                alpha,
                Kind::Unspecified,
            );
        elements.extend(tree.into_iter().map(|element| {
            SceneElement::Surface(RescaleRenderElement::from_element(
                element, origin, view.scale,
            ))
        }));
    }

    fn push_window(
        &self,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
        window: &Window,
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
            self.push_surface(renderer, elements, surface, at, false);
            return;
        }
        // Menus go in front, without clipping or scaling.
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let at = Placement {
                view,
                loc: visual.rect.loc + (offset - popup.geometry().loc).to_f64(),
                alpha: visual.opacity,
            };
            self.push_surface(renderer, elements, popup.wl_surface(), at, false);
        }
        // A fullscreen window has no corners, border or shadow. It keeps them while it slides
        // towards the whole screen, so that they do not vanish before it gets there.
        let decorated = !is_fullscreen(window) || visual.moving;
        let scale = view.scale as f32;
        let radius = if decorated {
            corner_radius(visual.rect.size.to_i32_round()) * scale
        } else {
            0.0
        };
        if decorated {
            elements.push(SceneElement::Shape(self.shape(
                rect,
                NO_FILL,
                color::BORDER,
                metrics::BORDER_WIDTH * scale,
                radius,
                visual.opacity,
            )));
        }
        // The content is drawn in `rect`, the drawn geometry, in frame pixels: the screen's
        // scale, and the animation's if there is one.
        let stretch = Scale {
            x: f64::from(rect.size.w) / f64::from(geometry.size.w),
            y: f64::from(rect.size.h) / f64::from(geometry.size.h),
        };
        let origin = rect.loc;
        let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                surface,
                origin - geometry.loc.to_physical(1),
                1.0,
                visual.opacity,
                Kind::Unspecified,
            );
        elements.extend(surfaces.into_iter().map(|surface| {
            SceneElement::Window(RoundedSurface {
                inner: RescaleRenderElement::from_element(surface, origin, stretch),
                program: self.clip.clone(),
                clip: rect.to_f64(),
                radius,
                frame_size: view.frame_size,
            })
        }));
        if decorated && shadow {
            let shadow = Shadow::of(rect, view.scale);
            elements.push(SceneElement::Shape(PixelShaderElement::new(
                self.shadow.clone(),
                frame_area(shadow.area),
                None,
                visual.opacity,
                vec![
                    Uniform::new("color", color::SHADOW.premultiplied()),
                    Uniform::new("caster", shadow.caster),
                    Uniform::new("window", shadow.window),
                    Uniform::new("radius", radius),
                    Uniform::new("sigma", shadow.sigma),
                ],
                Kind::Unspecified,
            )));
        }
    }

    /// Rounded rectangle: `fill` (premultiplied) inside, `border` over `border_width` pixels
    /// along the edge, inside the rectangle like a CSS border.
    fn shape(
        &self,
        rect: Rectangle<i32, Physical>,
        fill: [f32; 4],
        border: Color,
        border_width: f32,
        radius: f32,
        alpha: f32,
    ) -> PixelShaderElement {
        PixelShaderElement::new(
            self.shape.clone(),
            frame_area(rect),
            None,
            alpha,
            vec![
                Uniform::new("fill", fill),
                Uniform::new("border", border.premultiplied()),
                Uniform::new("radius", radius),
                Uniform::new("border_width", border_width),
            ],
            Kind::Unspecified,
        )
    }
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

/// Background discs for a screen of `size`, with their color: the places of the Main board,
/// stretched with the screen. The diameter follows the larger of the two ratios: the discs stay
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
        (rect, color::DESKTOP_DISCS[i])
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
fn clip_in_frame(
    clip: Rectangle<f64, Physical>,
    transform: Transform,
    frame_size: Size<i32, Physical>,
) -> [f32; 4] {
    let clip = transform.transform_rect_in(clip, &frame_size.to_f64());
    [clip.loc.x, clip.loc.y, clip.size.w, clip.size.h].map(|v| v as f32)
}

/// A client surface clipped by the rounded rectangle of its window. The texture program
/// replaces Smithay's while the surface is drawn.
pub struct RoundedSurface {
    inner: RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    program: GlesTexProgram,
    /// Clip rectangle, in the frame, before its transform.
    clip: Rectangle<f64, Physical>,
    radius: f32,
    frame_size: Size<i32, Physical>,
}

impl Element for RoundedSurface {
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

    // No opaque regions: the clipped corners show what is underneath. The windows below are
    // therefore always drawn.
    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for RoundedSurface {
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
            (rect(-260, 340, 1100, 1100), color::DESKTOP_DISCS[0])
        );
        assert_eq!(
            discs[1],
            (rect(860, -420, 900, 900), color::DESKTOP_DISCS[1])
        );
        assert_eq!(
            discs[2],
            (rect(980, 520, 520, 520), color::DESKTOP_DISCS[2])
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
    fn the_corners_have_the_window_radius() {
        assert_eq!(corner_radius((836, 652).into()), 16.0);
        // A tiny window becomes a pill, never a negative radius.
        assert_eq!(corner_radius((100, 20).into()), 10.0);
        assert_eq!(corner_radius((0, 0).into()), 0.0);
    }
}
