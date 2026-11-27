//! Glass that asks for the blur behind it (lot D-37, ADR 0027): its content, laid out and drawn
//! as is, and its rounded shape given to the compositor as the blur region of the surface,
//! through iced_exwlshell's surface regions (`ext_background_effect_v1`). Behind the
//! `layer-shell` feature: only the programs on layer-shell have such surfaces.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, renderer};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme};
use iced_exwlshell::BlurRegions;
use iced_exwlshell::reexport::BlurRegion;

use crate::blur::rounded_region;

/// `content`, blurred behind over its bounds rounded by `radius`: the style of the content
/// draws the glass itself (tint, border).
pub struct Glass<'a, Message> {
    content: Element<'a, Message>,
    radius: f32,
}

pub fn glass<'a, Message>(
    content: impl Into<Element<'a, Message>>,
    radius: f32,
) -> Glass<'a, Message> {
    Glass {
        content: content.into(),
        radius,
    }
}

/// The blur region of glass at `bounds`, in the coordinates of the surface.
pub fn region(bounds: Rectangle, radius: f32) -> Vec<BlurRegion> {
    let (x, y) = (bounds.x.round() as i32, bounds.y.round() as i32);
    let (width, height) = (bounds.width.round() as i32, bounds.height.round() as i32);
    rounded_region(x, y, width, height, radius)
        .into_iter()
        .map(|[x, y, width, height]| BlurRegion {
            x,
            y,
            width,
            height,
        })
        .collect()
}

impl<Message> Widget<Message, Theme, Renderer> for Glass<'_, Message> {
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

    // iced_exwlshell gathers the regions of every glass of the surface at each redraw.
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let mut regions = BlurRegions(region(layout.bounds(), self.radius));
        operation.custom(None, layout.bounds(), &mut regions);
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

impl<'a, Message: 'a> From<Glass<'a, Message>> for Element<'a, Message> {
    fn from(widget: Glass<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_region_is_the_rounded_bounds_on_the_surface() {
        let bounds = Rectangle::new((100.4, 20.0).into(), (300.0, 76.0).into());
        let rects = region(bounds, 22.0);
        let top = rects.iter().map(|r| r.y).min();
        let bottom = rects.iter().map(|r| r.y + r.height).max();
        assert_eq!((top, bottom), (Some(20), Some(96)));
        // The middle band spans the whole width; the first row is set in by the curve.
        assert!(rects.iter().any(|r| r.x == 100 && r.width == 300));
        assert!(rects.iter().filter(|r| r.y == 20).all(|r| r.x > 100));
    }
}
