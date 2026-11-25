//! Rendu du bureau avec le moteur GLES de Smithay et trois programmes maison (`src/shaders`).
//!
//! Les éléments vont de l'avant vers l'arrière, comme Smithay les attend : l'aperçu du calage,
//! puis chaque fenêtre, de la plus haute à la plus basse, avec ses menus, sa bordure, son
//! contenu découpé en rectangle arrondi et son ombre, enfin les disques du fond. La couleur du
//! bureau remplit le reste. Voir « Rendu » dans `docs/compositeur.md`.

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
    desktop::{PopupManager, Window},
    utils::{Buffer, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
    wayland::seat::WaylandFocus,
};

use crate::{
    State,
    anim::{RectF, Visual},
    handlers::is_fullscreen,
    is_override_redirect, window_data,
};

/// Pas de remplissage : seule la bordure d'une forme est dessinée.
const NO_FILL: [f32; 4] = [0.0; 4];

render_elements! {
    /// Tout ce qu'une image du bureau contient.
    pub SceneElement<=GlesRenderer>;
    Shape=PixelShaderElement,
    Window=RoundedSurface,
    Popup=WaylandSurfaceRenderElement<GlesRenderer>,
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

    /// Éléments d'une image de l'écran `output` à l'instant `now`, et s'il y a une animation
    /// en cours. Les coordonnées sont celles de l'écran, à l'échelle 1.
    pub fn elements(
        &self,
        renderer: &mut GlesRenderer,
        state: &State,
        output: Rectangle<i32, Logical>,
        preview: Option<Visual>,
        now: Duration,
    ) -> (Vec<SceneElement>, bool) {
        let mut elements = Vec::new();
        let mut animating = preview.is_some();
        if let Some(preview) = preview {
            let rect = snap(preview.rect);
            elements.push(SceneElement::Shape(self.shape(
                rect,
                ui::SNAP_PREVIEW_FILL.premultiplied(),
                ui::SNAP_PREVIEW_BORDER,
                ui::SNAP_PREVIEW_BORDER_WIDTH as f32,
                corner_radius(rect.size),
                preview.opacity,
            )));
        }
        let output_size = output.size.to_physical(1);
        for window in state.space.elements().rev() {
            let Some(mut visual) = state.window_visual(window, now) else {
                continue;
            };
            visual.rect.loc -= output.loc.to_f64();
            animating |= window_data(window).motion.borrow().is_animating();
            self.push_window(renderer, &mut elements, window, visual, output_size);
        }
        let discs = desktop_discs(output.size);
        for (rect, color) in discs.into_iter().rev() {
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

    fn push_window(
        &self,
        renderer: &mut GlesRenderer,
        elements: &mut Vec<SceneElement>,
        window: &Window,
        visual: Visual,
        output_size: Size<i32, Physical>,
    ) {
        let (Some(surface), rect) = (window.wl_surface(), snap(visual.rect)) else {
            return;
        };
        let geometry = window.geometry();
        if rect.size.is_empty() || geometry.size.is_empty() {
            return;
        }
        let surface = surface.as_ref();
        // Menu ou bulle X11 : ni découpe, ni bordure, ni ombre, comme un menu Wayland.
        if is_override_redirect(window) {
            elements.extend(
                render_elements_from_surface_tree(
                    renderer,
                    surface,
                    rect.loc.to_physical(1),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(SceneElement::Popup),
            );
            return;
        }
        // Les menus passent devant, sans découpe ni mise à l'échelle.
        for (popup, offset) in PopupManager::popups_for_surface(surface) {
            let loc = rect.loc + offset - popup.geometry().loc;
            elements.extend(
                render_elements_from_surface_tree(
                    renderer,
                    popup.wl_surface(),
                    loc.to_physical(1),
                    1.0,
                    visual.opacity,
                    Kind::Unspecified,
                )
                .into_iter()
                .map(SceneElement::Popup),
            );
        }
        // Une fenêtre en plein écran n'a ni coins, ni bordure, ni ombre. Elle les garde tant
        // qu'elle glisse vers l'écran entier, pour qu'ils ne disparaissent pas avant.
        let decorated = !is_fullscreen(window) || visual.moving;
        let radius = if decorated {
            corner_radius(rect.size)
        } else {
            0.0
        };
        if decorated {
            elements.push(SceneElement::Shape(self.shape(
                rect,
                NO_FILL,
                color::BORDER,
                metrics::BORDER_WIDTH,
                radius,
                visual.opacity,
            )));
        }
        // Le contenu est dessiné dans `rect`, la géométrie dessinée : en dehors d'une animation,
        // c'est sa vraie place à l'échelle 1.
        let scale = Scale {
            x: visual.rect.size.w / f64::from(geometry.size.w),
            y: visual.rect.size.h / f64::from(geometry.size.h),
        };
        let origin = rect.loc.to_physical(1);
        let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                surface,
                (rect.loc - geometry.loc).to_physical(1),
                1.0,
                visual.opacity,
                Kind::Unspecified,
            );
        elements.extend(surfaces.into_iter().map(|surface| {
            SceneElement::Window(RoundedSurface {
                inner: RescaleRenderElement::from_element(surface, origin, scale),
                program: self.clip.clone(),
                clip: rect.to_f64().to_physical(1.0),
                radius,
                output_size,
            })
        }));
        if decorated {
            let shadow = Shadow::of(rect);
            elements.push(SceneElement::Shape(PixelShaderElement::new(
                self.shadow.clone(),
                shadow.area,
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
        rect: Rectangle<i32, Logical>,
        fill: [f32; 4],
        border: Color,
        border_width: f32,
        radius: f32,
        alpha: f32,
    ) -> PixelShaderElement {
        PixelShaderElement::new(
            self.shape.clone(),
            rect,
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

/// Rectangle animé arrondi au pixel près.
fn snap(rect: RectF) -> Rectangle<i32, Logical> {
    Rectangle::new(rect.loc.to_i32_round(), rect.size.to_i32_round())
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
    pub area: Rectangle<i32, Logical>,
    pub caster: [f32; 4],
    pub window: [f32; 4],
    pub sigma: f32,
}

impl Shadow {
    pub fn of(window: Rectangle<i32, Logical>) -> Self {
        let blur = metrics::SHADOW_BLUR.round() as i32;
        let offset = metrics::SHADOW_OFFSET.round() as i32;
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
            sigma: metrics::SHADOW_BLUR / 2.0,
        }
    }
}

/// Rectangle `clip` de l'écran dans l'image où l'on dessine, là où `gl_FragCoord` compte ses
/// pixels : après la transformation de l'écran. Le backend fenêtré retourne l'image
/// (`Flipped180`) : le haut de l'écran est en bas de l'image.
fn clip_in_frame(
    clip: Rectangle<f64, Physical>,
    transform: Transform,
    output_size: Size<i32, Physical>,
) -> [f32; 4] {
    let clip = transform.transform_rect_in(clip, &output_size.to_f64());
    [clip.loc.x, clip.loc.y, clip.size.w, clip.size.h].map(|v| v as f32)
}

/// Une surface de client découpée par le rectangle arrondi de sa fenêtre. Le programme de
/// texture remplace celui de Smithay le temps de dessiner la surface.
pub struct RoundedSurface {
    inner: RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    program: GlesTexProgram,
    /// Rectangle de découpe, dans l'écran, avant sa transformation.
    clip: Rectangle<f64, Physical>,
    radius: f32,
    output_size: Size<i32, Physical>,
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
        let clip = clip_in_frame(self.clip, frame.transformation(), self.output_size);
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
        let shadow = Shadow::of(rect(36, 64, 836, 652));
        // Décalée de 28 vers le bas, étendue de 70 de chaque côté.
        assert_eq!(
            shadow.area,
            rect(36 - 70, 64 + 28 - 70, 836 + 140, 652 + 140)
        );
        assert_eq!(shadow.caster, [70.0, 70.0, 836.0, 652.0]);
        // Dans l'élément, la fenêtre est 28 pixels au-dessus de son ombre.
        assert_eq!(shadow.window, [70.0, 42.0, 836.0, 652.0]);
        assert_eq!(shadow.sigma, 35.0);
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
