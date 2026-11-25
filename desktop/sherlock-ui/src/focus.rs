//! Keyboard focus for the components iced 0.14 cannot focus (buttons, toggles, menu entries).
//!
//! `Focusable` wraps a widget: it registers with iced's focus operations, triggers its action
//! on Enter or Space, and draws the focus ring. The program moves the focus around:
//! `keyboard::listen().filter_map(focus::tab)` then `focus::advance` in its `update`.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Operation, Tree, operation, tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay};
use iced::keyboard::{self, Key, key::Named};
use iced::{Background, Element, Event, Length, Rectangle, Renderer, Size, Task, Theme, Vector};
use iced::{border, widget};

use crate::{color, metrics};

/// Direction of the focus move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Next,
    Previous,
}

/// Tab moves forward, Shift + Tab moves back. To pass to `keyboard::listen().filter_map`.
pub fn tab(event: keyboard::Event) -> Option<Direction> {
    match event {
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Tab),
            modifiers,
            ..
        } if modifiers.shift() => Some(Direction::Previous),
        keyboard::Event::KeyPressed {
            key: Key::Named(Named::Tab),
            ..
        } => Some(Direction::Next),
        _ => None,
    }
}

/// Moves the focus to the next or previous component.
pub fn advance<T>(direction: Direction) -> Task<T> {
    match direction {
        Direction::Next => widget::operation::focus_next(),
        Direction::Previous => widget::operation::focus_previous(),
    }
}

/// A widget that takes the keyboard focus.
pub struct Focusable<'a, Message> {
    content: Element<'a, Message>,
    on_activate: Option<Message>,
    radius: f32,
    ring: bool,
}

impl<'a, Message> Focusable<'a, Message> {
    /// `on_activate` fires on Enter or Space. Without an action, the widget is disabled and
    /// does not take the focus. `radius` is the widget's, which the ring follows.
    pub fn new(
        content: impl Into<Element<'a, Message>>,
        on_activate: Option<Message>,
        radius: f32,
    ) -> Self {
        Self {
            content: content.into(),
            on_activate,
            radius,
            ring: false,
        }
    }

    /// Draws the ring even without focus: used by the gallery.
    pub fn ring(mut self, ring: bool) -> Self {
        self.ring = ring;
        self
    }
}

#[derive(Default)]
struct State {
    focused: bool,
}

impl operation::Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

impl<Message: Clone> Widget<Message, Theme, Renderer> for Focusable<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        // A component that becomes disabled while it has the focus loses it.
        if self.on_activate.is_none() {
            tree.state.downcast_mut::<State>().focused = false;
        }
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
        let state = tree.state.downcast_mut::<State>();
        if self.on_activate.is_some() {
            operation.focusable(None, layout.bounds(), state);
        } else {
            state.focused = false;
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
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: Key::Named(Named::Enter | Named::Space),
                repeat,
                ..
            }) if state.focused => {
                if let Some(message) = &self.on_activate {
                    // A key held down activates only once.
                    if !repeat {
                        shell.publish(message.clone());
                    }
                    shell.capture_event();
                    return;
                }
            }
            // Like `:focus-visible` in a browser: the mouse clears the ring.
            Event::Mouse(mouse::Event::ButtonPressed(_)) => state.focused = false,
            _ => {}
        }
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
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        if self.ring || tree.state.downcast_ref::<State>().focused {
            let gap = metrics::FOCUS_RING_GAP + metrics::FOCUS_RING_WIDTH;
            let b = layout.bounds();
            let bounds = Rectangle::new(
                b.position() - Vector::new(gap, gap),
                b.size().expand([2.0 * gap, 2.0 * gap]),
            );
            let edge = border::width(metrics::FOCUS_RING_WIDTH)
                .color(color::FOCUS_RING)
                .rounded(self.radius + gap);
            let quad = Quad {
                bounds,
                border: edge,
                ..Quad::default()
            };
            iced::advanced::Renderer::fill_quad(
                renderer,
                quad,
                Background::Color(iced::Color::TRANSPARENT),
            );
        }
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

impl<'a, Message: Clone + 'a> From<Focusable<'a, Message>> for Element<'a, Message> {
    fn from(focusable: Focusable<'a, Message>) -> Self {
        Element::new(focusable)
    }
}
