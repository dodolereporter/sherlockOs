//! The shape of the pointer each component asks for (lot UI-01), through iced's
//! `mouse::Interaction`, which the compositor gets as a `wp_cursor_shape_v1` shape:
//!
//! | Component | Shape |
//! | --- | --- |
//! | Button, tile, list entry, toggle, chevron | `Pointer`, the pointing hand |
//! | Slider, hovered | `Grab`, the open hand (iced's slider) |
//! | Slider, dragged | `Grabbing`, the closed hand (iced's slider) |
//! | Text field | `Text` (iced's text input) |
//! | Anything disabled | `NotAllowed` |

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, renderer};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme};

/// The shape over a clickable control: the pointing hand when it acts, the forbidden sign when
/// it is disabled. A shape its content asks for (a slider inside, a text field) wins while
/// enabled; away from the control, its content decides.
pub fn control(inner: mouse::Interaction, over: bool, enabled: bool) -> mouse::Interaction {
    match (over, enabled, inner) {
        (true, false, _) => mouse::Interaction::NotAllowed,
        (true, true, mouse::Interaction::None | mouse::Interaction::Idle) => {
            mouse::Interaction::Pointer
        }
        _ => inner,
    }
}

/// Content that does nothing: the forbidden sign over it. A disabled slider, a disabled text
/// field.
pub struct Disabled<'a, Message> {
    content: Element<'a, Message>,
}

/// Shows the forbidden sign over `content`.
pub fn disabled<'a, Message>(content: impl Into<Element<'a, Message>>) -> Disabled<'a, Message> {
    Disabled {
        content: content.into(),
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Disabled<'_, Message> {
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
        let inner = self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        );
        control(inner, cursor.is_over(layout.bounds()), false)
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
    }
}

impl<'a, Message: 'a> From<Disabled<'a, Message>> for Element<'a, Message> {
    fn from(widget: Disabled<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mouse::Interaction::{Grab, Grabbing, Idle, None, NotAllowed, Pointer, Text};

    #[test]
    fn a_control_points_when_it_acts_and_forbids_when_it_cannot() {
        assert_eq!(control(None, true, true), Pointer);
        assert_eq!(control(Idle, true, true), Pointer);
        assert_eq!(control(Pointer, true, false), NotAllowed);
        assert_eq!(control(Text, true, false), NotAllowed);
        // What the content asks for wins while it acts: a slider, a field.
        assert_eq!(control(Grab, true, true), Grab);
        assert_eq!(control(Text, true, true), Text);
        // Away from it, the content decides: a slider dragged out keeps its closed hand.
        assert_eq!(control(Grabbing, false, true), Grabbing);
        assert_eq!(control(None, false, false), None);
    }
}
