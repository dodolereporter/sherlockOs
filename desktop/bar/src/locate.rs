//! Where a widget of the bar is drawn (lot D-45): the chevron of the notification area sits
//! after icons and before indicators whose widths only the renderer knows, and its popup opens
//! under it. `locate` wraps the chevron and sends its bounds in the bar surface each time they
//! change, at a redraw.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, window};

/// `content`, whose bounds go to `on_bounds` each time they change.
pub fn locate<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    on_bounds: impl Fn(Rectangle) -> Message + 'a,
) -> Element<'a, Message> {
    Element::new(Locate {
        content: content.into(),
        on_bounds: Box::new(on_bounds),
    })
}

struct Locate<'a, Message> {
    content: Element<'a, Message>,
    on_bounds: Box<dyn Fn(Rectangle) -> Message + 'a>,
}

/// The bounds last sent.
#[derive(Default)]
struct Sent(Option<Rectangle>);

impl<Message> Widget<Message, Theme, Renderer> for Locate<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Sent>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Sent::default())
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
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let bounds = layout.bounds();
            let sent = tree.state.downcast_mut::<Sent>();
            if sent.0 != Some(bounds) {
                sent.0 = Some(bounds);
                shell.publish((self.on_bounds)(bounds));
            }
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
