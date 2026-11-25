//! Rendu du bureau avec le moteur GLES de Smithay et trois programmes maison (`src/shaders`).
//!
//! Les éléments vont de l'avant vers l'arrière, comme Smithay les attend : les surfaces ancrées
//! des couches de surimpression et du haut, l'aperçu du calage, puis chaque fenêtre, de la plus
//! haute à la plus basse, avec ses menus, sa bordure, son contenu découpé en rectangle arrondi
//! et son ombre, les surfaces ancrées du bas et du fond, enfin les disques du fond. La couleur
//! du bureau remplit le reste. Chaque écran a ses éléments, en pixels de l'image, à son
//! échelle. Voir « Rendu » dans `docs/compositeur.md`.

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
            utils::RescaleRenderElement,
        },
        gles::{
            GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, Uniform,
            UniformName, UniformType, element::PixelShaderElement,
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
    State, anim::Visual, handlers::is_fullscreen, is_override_redirect, output::View, window_data,
};

/// Pas de remplissage : seule la bordure d'une forme est dessinée.
const NO_FILL: [f32; 4] = [0.0; 4];

render_elements! {
    /// Tout ce qu'une image du bureau contient.
    pub SceneElement<=GlesRenderer>;
    Shape=PixelShaderElement,
    Window=RoundedSurface,
    Surface=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
}

/// Programmes compilés une fois pour toutes au démarrage du rendu.
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

    /// Éléments de l'écran `output`, vu par `view`, à l'instant `now`, et s'il y a une
    /// animation en cours. Les éléments sont en pixels de l'image, à l'échelle de l'écran :
    /// coins, bordures et ombres suivent cette échelle. `preview` est dans l'espace.
    pub fn elements(
        &self,
        renderer: &mut GlesRenderer,
        state: &State,
        output: &Output,
        view: &View,
        preview: Option<Visual>,
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
            self.push_window(renderer, &mut elements, window, visual, view);
        }
        let below = [Layer::Bottom, Layer::Background];
        self.push_layers(renderer, &mut elements, output, view, &below);
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
                self.push_surface(
                    renderer,
                    elements,
                    surface.wl_surface(),
                    loc,
                    view,
                    1.0,
                    true,
                );
                self.push_backdrop(elements, view.rect(geometry.to_f64()));
            }
        }
    }

    /// Placeholder for the blur behind glass surfaces (lot D-11b): what is already drawn
    /// behind `rect` gets blurred here, just behind the surface. Nothing yet: the surface is
    /// drawn as is, translucent over a sharp background.
    fn push_backdrop(&self, _elements: &mut [SceneElement], _rect: Rectangle<i32, Physical>) {}

    /// A surface tree, untouched (no clip, border or shadow), with its origin at `loc` in the
    /// space. `popups`: its menus too, in front of it (smithay lists nested menus with the
    /// root surface, so a menu never has menus of its own here).
    #[allow(clippy::too_many_arguments)]
    fn push_surface(
        &self,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
        surface: &WlSurface,
        loc: Point<f64, Logical>,
        view: &View,
        alpha: f32,
        popups: bool,
    ) {
        if popups {
            for (popup, offset) in PopupManager::popups_for_surface(surface) {
                let loc = loc + (offset - popup.geometry().loc).to_f64();
                self.push_surface(
                    renderer,
                    elements,
                    popup.wl_surface(),
                    loc,
                    view,
                    alpha,
                    false,
                );
            }
        }
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
    ) {
        let Some(surface) = window.wl_surface() else {
            return;
        };
        let surface = surface.as_ref();
        let (rect, geometry) = (view.rect(visual.rect), window.geometry());
        if rect.size.is_empty() || geometry.size.is_empty() {
            return;
        }
        // Menu ou bulle X11 : ni découpe, ni bordure, ni ombre, comme un menu Wayland.
        if is_override_redirect(window) {
            self.push_surface(
                renderer,
                elements,
                surface,
                visual.rect.loc,
                view,
                1.0,
                false,
            );
            return;
        }
        // Les menus passent devant, sans découpe ni mise à l'échelle.
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let loc = visual.rect.loc + (offset - popup.geometry().loc).to_f64();
            let alpha = visual.opacity;
            self.push_surface(
                renderer,
                elements,
                popup.wl_surface(),
                loc,
                view,
                alpha,
                true,
            );
        }
        // Une fenêtre en plein écran n'a ni coins, ni bordure, ni ombre. Elle les garde tant
        // qu'elle glisse vers l'écran entier, pour qu'ils ne disparaissent pas avant.
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
        // Le contenu est dessiné dans `rect`, la géométrie dessinée, en pixels de l'image :
        // l'échelle de l'écran, et celle de l'animation s'il y en a une.
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
        if decorated {
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

    /// Rectangle arrondi : `fill` (prémultiplié) à l'intérieur, `border` sur `border_width`
    /// pixels le long du bord, à l'intérieur du rectangle comme une bordure CSS.
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

/// Area of a pixel shader element. The frame is drawn at scale 1: its "logical" coordinates
/// are its pixels, and every element is already placed and sized in them.
fn frame_area(rect: Rectangle<i32, Physical>) -> Rectangle<i32, Logical> {
    Rectangle::new(
        (rect.loc.x, rect.loc.y).into(),
        (rect.size.w, rect.size.h).into(),
    )
}

/// Rayon des coins d'une fenêtre, réduit pour une fenêtre plus petite que ses deux coins.
pub fn corner_radius(size: Size<i32, Logical>) -> f32 {
    let half = size.w.min(size.h) as f32 / 2.0;
    metrics::WINDOW_RADIUS.min(half.max(0.0))
}

/// Disques du fond pour un écran de `size`, avec leur couleur : les places de la planche Main,
/// étirées avec l'écran. Le diamètre suit le plus grand des deux rapports : les disques restent
/// ronds et couvrent les mêmes régions.
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

/// Ombre d'une fenêtre, comme `box-shadow: 0 28px 70px` en CSS : la fenêtre décalée vers le
/// bas (`caster`), floutée par une gaussienne d'écart type moitié du flou. L'élément s'étend de
/// la portée du flou autour d'elle. `caster` et `window` sont donnés dans l'élément.
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

/// Rectangle `clip` de l'image dans le tampon où l'on dessine, là où `gl_FragCoord` compte ses
/// pixels : après la transformation de l'image. Le backend fenêtré retourne l'image
/// (`Flipped180`) : le haut de l'image est en bas du tampon.
fn clip_in_frame(
    clip: Rectangle<f64, Physical>,
    transform: Transform,
    frame_size: Size<i32, Physical>,
) -> [f32; 4] {
    let clip = transform.transform_rect_in(clip, &frame_size.to_f64());
    [clip.loc.x, clip.loc.y, clip.size.w, clip.size.h].map(|v| v as f32)
}

/// Une surface de client découpée par le rectangle arrondi de sa fenêtre. Le programme de
/// texture remplace celui de Smithay le temps de dessiner la surface.
pub struct RoundedSurface {
    inner: RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    program: GlesTexProgram,
    /// Rectangle de découpe, dans l'image, avant sa transformation.
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

    // Pas de régions opaques : les coins découpés laissent voir ce qui est dessous. Les fenêtres
    // du dessous sont donc toujours dessinées.
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

    // Jamais posée telle quelle sur un plan de l'écran : la découpe serait perdue.
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
    fn les_disques_suivent_la_planche_main() {
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
        // Sur un écran deux fois plus grand, tout double.
        let large = desktop_discs((2880, 1800).into());
        assert_eq!(large[2].0, rect(1960, 1040, 1040, 1040));
        // Sur un écran plus large, les disques restent ronds.
        let wide = desktop_discs((1920, 900).into());
        assert!(wide.iter().all(|(r, _)| r.size.w == r.size.h));
    }

    #[test]
    fn l_ombre_suit_box_shadow() {
        // Fenêtre de la planche Main : 836 × 652 en (36, 64).
        let shadow = Shadow::of(frame_rect(36, 64, 836, 652), 1.0);
        // Décalée de 28 vers le bas, étendue de 70 de chaque côté.
        assert_eq!(
            shadow.area,
            frame_rect(36 - 70, 64 + 28 - 70, 836 + 140, 652 + 140)
        );
        assert_eq!(shadow.caster, [70.0, 70.0, 836.0, 652.0]);
        // Dans l'élément, la fenêtre est 28 pixels au-dessus de son ombre.
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
    fn la_vue_met_l_espace_a_l_echelle_de_l_ecran() {
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
    fn la_decoupe_suit_le_retournement_de_l_image() {
        let clip = Rectangle::new((36.0, 64.0).into(), (836.0, 652.0).into());
        let size = (1440, 900).into();
        assert_eq!(
            clip_in_frame(clip, Transform::Normal, size),
            [36.0, 64.0, 836.0, 652.0]
        );
        // Retournée : la fenêtre en haut de l'écran est en haut de l'image GL, dont l'axe
        // vertical monte.
        assert_eq!(
            clip_in_frame(clip, Transform::Flipped180, size),
            [36.0, 900.0 - 64.0 - 652.0, 836.0, 652.0]
        );
    }

    #[test]
    fn les_coins_ont_le_rayon_des_fenetres() {
        assert_eq!(corner_radius((836, 652).into()), 16.0);
        // Une fenêtre minuscule devient une pilule, jamais un rayon négatif.
        assert_eq!(corner_radius((100, 20).into()), 10.0);
        assert_eq!(corner_radius((0, 0).into()), 0.0);
    }
}
