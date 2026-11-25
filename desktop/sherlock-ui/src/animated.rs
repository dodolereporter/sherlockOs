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
    key: Option<u64>,
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
        key: None,
    }
}

impl<Message> Hoverable<'_, Message> {
    /// Ties the hover state to what the widget shows, not to its place in the tree: in a
    /// virtualised list the same place shows another row after a scroll, and its veil must
    /// not follow.
    pub fn key(mut self, key: u64) -> Self {
        self.key = Some(key);
        self
    }
}

/// Hover state of one widget: whether the pointer is over it, and the running fade.
#[derive(Debug, Default)]
struct HoverState {
    key: Option<u64>,
    hovered: bool,
    fade: Option<Fade>,
}

impl HoverState {
    /// Opacity of the veil, from 0 to 1.
    fn value(&self, now: Duration) -> f32 {
        match &self.fade {
            Some(fade) => fade.value(now),
            None => f32::from(u8::from(self.hovered)),
        }
    }

    /// The pointer is (or is no longer) over the widget. Returns whether a fade started.
    fn set(&mut self, over: bool, now: Duration) -> bool {
        if over == self.hovered {
            return false;
        }
        self.hovered = over;
        let to = f32::from(u8::from(over));
        match &mut self.fade {
            Some(fade) => fade.retarget(now, to, motion::FADE),
            None => self.fade = Some(Fade::new(1.0 - to, to, now, motion::FADE)),
        }
        true
    }

    /// Forgets a finished fade. Returns whether one still runs.
    fn tick(&mut self, now: Duration) -> bool {
        match &self.fade {
            Some(fade) if fade.is_done(now) => {
                self.fade = None;
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    /// A widget that shows something else starts from rest.
    fn rekey(&mut self, key: Option<u64>) {
        if self.key != key {
            *self = Self {
                key,
                ..Self::default()
            };
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
        tree.state.downcast_mut::<HoverState>().rekey(self.key);
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
        // Any pointer event, a wheel scroll included, can change what is under the pointer:
        // a list that scrolls brings another row under a still pointer.
        if let Event::Mouse(_) = event {
            let over = cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
            if state.set(over, now) {
                shell.request_redraw();
            }
        }
        if let Event::Window(window::Event::RedrawRequested(_)) = event
            && state.tick(now)
        {
            shell.request_redraw();
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

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn the_veil_fades_in_and_out_within_a_hover() {
        let mut state = HoverState::default();
        assert_eq!(state.value(Duration::ZERO), 0.0);
        assert!(state.set(true, Duration::ZERO));
        // Entering again changes nothing.
        assert!(!state.set(true, 10 * MS));
        let half = state.value(75 * MS);
        assert!(half > 0.0 && half < 1.0, "{half}");
        assert!(state.tick(75 * MS));
        assert!(!state.tick(motion::FADE + MS));
        assert_eq!(state.value(motion::FADE + MS), 1.0);
        // Leaving fades back out from where it is.
        assert!(state.set(false, 200 * MS));
        assert!(state.value(210 * MS) > 0.0);
        assert!(!state.tick(200 * MS + motion::FADE + MS));
        assert_eq!(state.value(200 * MS + motion::FADE + MS), 0.0);
    }

    #[test]
    fn another_row_in_the_same_place_starts_from_rest() {
        let mut state = HoverState::default();
        state.rekey(Some(7));
        state.set(true, Duration::ZERO);
        state.rekey(Some(7));
        assert!(state.hovered);
        state.rekey(Some(8));
        assert!(!state.hovered);
        assert_eq!(state.value(Duration::ZERO), 0.0);
    }
}
