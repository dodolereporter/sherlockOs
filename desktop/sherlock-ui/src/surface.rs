//! The drop shadow of floating surfaces (menus, dialogs), drawn for the renderer in use.
//!
//! On the GPU (wgpu), the shadow is the design board's: one blurred quad shadow. iced 0.14's
//! software renderer (tiny-skia) draws such a shadow as a separate image, without the clip mask
//! of the area being redrawn: when a frame redraws several damaged rectangles, that shadow is
//! painted again over what the other rectangles already drew, and a menu became a black block
//! hiding its own entries. In software, `Shadowed` draws its shadow as a few plain, slightly
//! larger and offset rounded rectangles instead: ordinary quads, clipped like everything else.

use std::sync::atomic::{AtomicU8, Ordering};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay};
use iced::{
    Background, Element, Event, Length, Padding, Rectangle, Renderer, Shadow, Size, Theme, Vector,
    border,
};

use crate::color::{Color, Palette};
use crate::metrics;

/// How high a surface floats, which sets its shadow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    /// No shadow: only tells which renderer draws (see `gpu_drawn`).
    Flat,
    /// Menus and submenus: `0 22px 54px rgba(3,8,18,0.6)`.
    Menu,
    /// Floating panels and dialogs: `0 28px 70px rgba(3,8,18,0.55)`.
    Panel,
}

impl Elevation {
    /// Vertical offset and blur of the design board's shadow.
    fn geometry(self) -> Option<(f32, f32)> {
        match self {
            Self::Flat => None,
            Self::Menu => Some((metrics::MENU_SHADOW_OFFSET, metrics::MENU_SHADOW_BLUR)),
            Self::Panel => Some((metrics::SHADOW_OFFSET, metrics::SHADOW_BLUR)),
        }
    }

    /// The design board's shadow, drawn on the GPU, in the colour of the theme's `palette`.
    pub fn blurred(self, palette: &Palette) -> Option<Shadow> {
        let color = match self {
            Self::Flat => return None,
            Self::Menu => palette.menu_shadow,
            Self::Panel => palette.shadow,
        };
        self.geometry().map(|(offset, blur)| Shadow {
            color: color.into(),
            offset: Vector::new(0.0, offset),
            blur_radius: blur,
        })
    }

    /// Steps of (vertical offset, spread, opacity) of the crisp shadow drawn in software. Each
    /// opacity is scaled by that of the theme's `shadow_base`.
    pub fn crisp(self) -> &'static [(f32, f32, f32)] {
        match self {
            Self::Flat => &[],
            Self::Menu => &metrics::MENU_SHADOW_LAYERS,
            Self::Panel => &metrics::PANEL_SHADOW_LAYERS,
        }
    }

    /// Room the blurred shadow takes around the surface, for a surface of its own (a popup)
    /// that must hold it: the blur on every side, moved down by the offset.
    pub fn margins(self) -> Padding {
        self.geometry()
            .map_or(Padding::ZERO, |(offset, blur)| Padding {
                top: (blur - offset).max(0.0),
                right: blur,
                bottom: blur + offset,
                left: blur,
            })
    }
}

/// Content drawn over its shadow.
pub struct Shadowed<'a, Message> {
    content: Element<'a, Message>,
    radius: f32,
    elevation: Elevation,
}

/// Lays the shadow of `elevation` under `content`, whose corners have radius `radius`.
pub fn shadowed<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    radius: f32,
    elevation: Elevation,
) -> Shadowed<'a, Message> {
    Shadowed {
        content: content.into(),
        radius,
        elevation,
    }
}

/// Renderer seen by the last `Shadowed` drawn: unknown, GPU, software.
static DRAWN: AtomicU8 = AtomicU8::new(0);
const GPU: u8 = 1;
const SOFTWARE: u8 = 2;

/// Whether the GPU draws this program, as seen by the last `Shadowed` drawn; `None` before
/// the first one. iced picks the renderer when it starts and falls back to software by
/// itself: the program cannot know beforehand. A program that sizes a surface for a blurred
/// shadow (the bar's menu) wraps its first surface in a `Flat` one to learn it.
pub fn gpu_drawn() -> Option<bool> {
    match DRAWN.load(Ordering::Relaxed) {
        GPU => Some(true),
        SOFTWARE => Some(false),
        _ => None,
    }
}

fn is_gpu(renderer: &Renderer) -> bool {
    matches!(renderer, iced_renderer::fallback::Renderer::Primary(_))
}

impl<Message> Widget<Message, Theme, Renderer> for Shadowed<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        // A menu or a panel is glass: it asks for the blur behind it (lot D-37b). `Flat` only
        // learns the renderer, over content that may not be glass.
        #[cfg(feature = "layer-shell")]
        if self.elevation != Elevation::Flat {
            crate::glass::declare(operation, layout.bounds(), self.radius);
        }
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let gpu = is_gpu(renderer);
        let palette = crate::theme::palette(theme);
        DRAWN.store(if gpu { GPU } else { SOFTWARE }, Ordering::Relaxed);
        crate::color::set_opaque_glass(!gpu);
        if gpu {
            if let Some(shadow) = self.elevation.blurred(palette) {
                // A transparent quad that only casts the shadow; the content covers it.
                let quad = Quad {
                    bounds,
                    border: border::rounded(self.radius),
                    shadow,
                    ..Quad::default()
                };
                iced::advanced::Renderer::fill_quad(
                    renderer,
                    quad,
                    Background::Color(iced::Color::TRANSPARENT),
                );
            }
        } else {
            for &(offset, spread, alpha) in self.elevation.crisp() {
                let quad = Quad {
                    bounds: Rectangle {
                        x: bounds.x - spread,
                        y: bounds.y + offset - spread,
                        width: bounds.width + 2.0 * spread,
                        height: bounds.height + 2.0 * spread,
                    },
                    border: border::rounded(self.radius + spread),
                    ..Quad::default()
                };
                let fill = Color {
                    a: alpha * palette.shadow_base.a,
                    ..palette.shadow_base
                };
                iced::advanced::Renderer::fill_quad(renderer, quad, Background::Color(fill.into()));
            }
        }
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<Shadowed<'a, Message>> for Element<'a, Message> {
    fn from(widget: Shadowed<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blurred_shadow_follows_the_theme() {
        let color = |elevation: Elevation, palette| elevation.blurred(palette).map(|s| s.color);
        assert_eq!(
            color(Elevation::Menu, &Palette::DARK),
            Some(Palette::DARK.menu_shadow.into())
        );
        // A menu drawn on the GPU in the light theme gets the light theme's shadow.
        assert_eq!(
            color(Elevation::Menu, &Palette::LIGHT),
            Some(Palette::LIGHT.menu_shadow.into())
        );
        assert_eq!(
            color(Elevation::Panel, &Palette::LIGHT),
            Some(Palette::LIGHT.shadow.into())
        );
        assert_ne!(
            color(Elevation::Menu, &Palette::DARK),
            color(Elevation::Menu, &Palette::LIGHT)
        );
        assert_eq!(color(Elevation::Flat, &Palette::DARK), None);
    }
}
