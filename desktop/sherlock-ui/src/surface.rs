//! A crisp drop shadow for floating surfaces (menus, dialogs).
//!
//! iced 0.14's software renderer draws a quad's blurred shadow as a separate image, without
//! the clip mask of the area being redrawn. When a frame redraws several damaged rectangles,
//! that shadow is painted again over what the other rectangles already drew: a menu became a
//! black block hiding its own entries, and cut the submenu beside it. `Shadowed` draws its
//! shadow as a few plain, slightly larger and offset rounded rectangles instead: ordinary
//! quads, clipped like everything else, and cheap.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay};
use iced::{Background, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, border};

use crate::color::{self, Color};

/// Content drawn over a stepped, crisp shadow.
pub struct Shadowed<'a, Message> {
    content: Element<'a, Message>,
    radius: f32,
    layers: &'static [(f32, f32, f32)],
}

/// Lays the shadow `layers` (vertical offset, spread, opacity of `color::SHADOW_BASE`) under
/// `content`, whose corners have radius `radius`.
pub fn shadowed<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    radius: f32,
    layers: &'static [(f32, f32, f32)],
) -> Shadowed<'a, Message> {
    Shadowed {
        content: content.into(),
        radius,
        layers,
    }
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
        for &(offset, spread, alpha) in self.layers {
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
                a: alpha,
                ..color::SHADOW_BASE
            };
            iced::advanced::Renderer::fill_quad(renderer, quad, Background::Color(fill.into()));
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
