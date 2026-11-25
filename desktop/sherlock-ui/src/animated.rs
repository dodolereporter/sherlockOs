//! Animated widgets, on the physics of `motion`: the hover veil that fades in, and the scale
//! of a panel that opens.
//!
//! An animated widget requests the next frame itself (`Shell::request_redraw`) while its
//! animation runs, and nothing afterwards: nothing redraws at rest.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay};
use iced::{
    Background, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Transformation,
    Vector, border, window,
};

use crate::color::Color;
use crate::motion::{self, Fade};

/// Time elapsed since the first call: the common origin of the widgets' animations.
pub fn clock() -> Duration {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed()
}

/// Content that lays a veil under itself on hover, fading over `motion::FADE` (150 ms).
pub struct Hoverable<'a, Message> {
    content: Element<'a, Message>,
    fill: Color,
    radius: f32,
}

/// Wraps `content` in a hover veil `fill`, with corners of radius `radius`.
pub fn hoverable<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    fill: Color,
    radius: f32,
) -> Hoverable<'a, Message> {
    Hoverable {
        content: content.into(),
        fill,
        radius,
    }
}

#[derive(Default)]
struct HoverState {
    hovered: bool,
    fade: Option<Fade>,
}

impl HoverState {
    fn value(&self, now: Duration) -> f32 {
        match &self.fade {
            Some(fade) => fade.value(now),
            None => f32::from(u8::from(self.hovered)),
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Hoverable<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<HoverState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(HoverState::default())
    }

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
        let state = tree.state.downcast_mut::<HoverState>();
        let now = clock();
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let over = cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
                if over != state.hovered {
                    state.hovered = over;
                    let to = f32::from(u8::from(over));
                    match &mut state.fade {
                        Some(fade) => fade.retarget(now, to, motion::FADE),
                        None => state.fade = Some(Fade::new(1.0 - to, to, now, motion::FADE)),
                    }
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(_)) => {
                if let Some(fade) = &state.fade {
                    if fade.is_done(now) {
                        state.fade = None;
                    } else {
                        shell.request_redraw();
                    }
                }
            }
            _ => {}
        }
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
        let value = tree.state.downcast_ref::<HoverState>().value(clock());
        if value > 0.0 {
            let fill = Color {
                a: self.fill.a * value,
                ..self.fill
            };
            let quad = Quad {
                bounds: layout.bounds(),
                border: border::rounded(self.radius),
                ..Quad::default()
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

impl<'a, Message: 'a> From<Hoverable<'a, Message>> for Element<'a, Message> {
    fn from(widget: Hoverable<'a, Message>) -> Self {
        Element::new(widget)
    }
}

/// Content drawn at scale `factor` from its top left corner: a menu that opens. Only the
/// drawing changes; the mouse sees the content at its final size.
pub struct Scaled<'a, Message> {
    content: Element<'a, Message>,
    factor: f32,
}

/// Draws `content` at scale `factor`, anchored at the top left.
pub fn scaled<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    factor: f32,
) -> Scaled<'a, Message> {
    Scaled {
        content: content.into(),
        factor,
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Scaled<'_, Message> {
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
        let origin: Point = layout.bounds().position();
        let transformation = Transformation::translate(origin.x, origin.y)
            * Transformation::scale(self.factor)
            * Transformation::translate(-origin.x, -origin.y);
        iced::advanced::Renderer::with_transformation(renderer, transformation, |renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
        });
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

impl<'a, Message: 'a> From<Scaled<'a, Message>> for Element<'a, Message> {
    fn from(widget: Scaled<'a, Message>) -> Self {
        Element::new(widget)
    }
}
