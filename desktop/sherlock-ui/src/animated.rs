//! Widgets animés, sur la physique de `motion` : le voile de survol qui apparaît en fondu, et
//! l'échelle d'un panneau qui s'ouvre.
//!
//! Un widget animé demande lui-même l'image suivante (`Shell::request_redraw`) tant que son
//! animation court, et plus rien ensuite : rien ne redessine au repos.

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

/// Temps écoulé depuis le premier appel : l'origine commune des animations des widgets.
pub fn clock() -> Duration {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed()
}

/// Un contenu qui pose un voile sous lui au survol, en fondu de `motion::FADE` (150 ms).
pub struct Hoverable<'a, Message> {
    content: Element<'a, Message>,
    fill: Color,
    radius: f32,
}

/// Enveloppe `content` d'un voile de survol `fill`, aux coins de rayon `radius`.
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

/// Un contenu dessiné à l'échelle `factor` depuis son coin haut gauche : un menu qui s'ouvre.
/// Seul le dessin change ; la souris voit le contenu à sa taille finale.
pub struct Scaled<'a, Message> {
    content: Element<'a, Message>,
    factor: f32,
}

/// Dessine `content` à l'échelle `factor`, ancré en haut à gauche.
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
