//! `Elastic`: content taller than its room, scrolled with the wheel or the touchpad, that
//! stretches past its ends and springs back, as on macOS (lot UI-08). The stretch follows the
//! fingers less and less: the rubber band of iOS (`apps::RUBBER`), the content never leaving
//! more than its room empty.
//!
//! The content is laid out at its own height, width of the room; its offset lives in the
//! widget's state, so it survives the redraws of the program. While it springs back, the widget
//! asks for the next frame itself, and nothing at rest.

use std::time::Duration;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, window};

use crate::animated::clock;
use crate::motion::{self, Animation};

/// Pixels a line of the wheel scrolls.
pub const LINE: f32 = 40.0;
/// The rubber band: `(1 - 1 / (excess × RUBBER / room + 1)) × room`.
pub const RUBBER: f32 = crate::apps::RUBBER;
/// The spring that brings a stretched list back to its end.
pub const BACK_SPRING: motion::Spring = motion::SNAP_SPRING;
/// A stretch is over within this many pixels.
const PRECISION: f64 = 0.5;

/// The stretch shown for an `excess` past an end, in a room `room` high.
pub fn rubber(excess: f32, room: f32) -> f32 {
    if room <= 0.0 {
        return 0.0;
    }
    let sign = excess.signum();
    sign * (1.0 - 1.0 / (excess.abs() * RUBBER / room + 1.0)) * room
}

/// The excess that shows `stretch`: the inverse of `rubber`.
pub fn excess(stretch: f32, room: f32) -> f32 {
    if room <= 0.0 {
        return 0.0;
    }
    let share = (stretch.abs() / room).min(0.99);
    stretch.signum() * room / RUBBER * (1.0 / (1.0 - share) - 1.0)
}

/// Content that scrolls in its room, elastic at its ends.
pub struct Elastic<'a, Message> {
    content: Element<'a, Message>,
    width: Length,
    height: Length,
}

/// Content scrolled in a room as wide and as high as it is given (`Length::Fill`).
pub fn elastic<'a, Message>(content: impl Into<Element<'a, Message>>) -> Elastic<'a, Message> {
    Elastic {
        content: content.into(),
        width: Length::Fill,
        height: Length::Fill,
    }
}

impl<Message> Elastic<'_, Message> {
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
}

/// Where the content is: its offset within its ends, and the stretch past them coming back.
#[derive(Debug, Default)]
struct State {
    offset: f32,
    /// Furthest offset: the content's height less the room's.
    end: f32,
    back: Option<Animation>,
}

impl State {
    fn stretch(&self, now: Duration) -> f32 {
        self.back.map_or(0.0, |back| back.value(now) as f32)
    }

    /// Where the content is drawn from: its offset and its stretch.
    fn shown(&self, now: Duration) -> f32 {
        self.offset + self.stretch(now)
    }

    /// Scrolls by `delta` pixels (positive: further down the content). Past an end, the
    /// stretch grows on the rubber band and springs back. Returns whether anything moved.
    fn scroll(&mut self, delta: f32, room: f32, now: Duration) -> bool {
        let stretch = self.stretch(now);
        // Back the other way while stretched: the stretch gives way first.
        let mut delta = delta;
        if stretch != 0.0 && stretch.signum() != delta.signum() {
            let raw = excess(stretch, room) + delta;
            if raw.signum() == stretch.signum() {
                self.back = Some(Animation::new(
                    BACK_SPRING,
                    f64::from(rubber(raw, room)),
                    0.0,
                    now,
                ));
                return true;
            }
            self.back = None;
            delta = raw;
        }
        let wanted = self.offset + delta;
        let within = wanted.clamp(0.0, self.end);
        let past = wanted - within;
        let moved = within != self.offset || past != 0.0;
        self.offset = within;
        if past != 0.0 {
            let raw = excess(self.stretch(now), room) + past;
            self.back = Some(Animation::new(
                BACK_SPRING,
                f64::from(rubber(raw, room)),
                0.0,
                now,
            ));
        }
        moved
    }

    /// The stretch has come back: it is dropped. Returns whether frames are still needed.
    fn settle(&mut self, now: Duration) -> bool {
        match self.back {
            Some(back) if back.is_done(now, PRECISION) => {
                self.back = None;
                false
            }
            Some(_) => true,
            None => false,
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Elastic<'_, Message> {
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
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let room = limits.resolve(self.width, self.height, Size::ZERO);
        // The content at its own height, as wide as the room.
        let inner = layout::Limits::new(
            Size::new(room.width, 0.0),
            Size::new(room.width, f32::INFINITY),
        );
        let content = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, &inner);
        let state = tree.state.downcast_mut::<State>();
        state.end = (content.size().height - room.height).max(0.0);
        state.offset = state.offset.clamp(0.0, state.end);
        layout::Node::with_children(room, vec![content])
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        if let Some(content) = layout.children().next() {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                content,
                renderer,
                operation,
            );
        }
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
        let bounds = layout.bounds();
        let now = clock();
        let shown = tree.state.downcast_ref::<State>().shown(now);
        let Some(content) = layout.children().next() else {
            return;
        };
        let inner_cursor = moved(cursor, bounds, shown);
        let inner_viewport = Rectangle {
            y: bounds.y + shown,
            ..bounds
        };
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            content,
            inner_cursor,
            renderer,
            clipboard,
            shell,
            &inner_viewport
                .intersection(&(*viewport + Vector::new(0.0, shown)))
                .unwrap_or(inner_viewport),
        );
        let state = tree.state.downcast_mut::<State>();
        match event {
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if !shell.is_event_captured() && cursor.is_over(bounds) =>
            {
                let dy = match delta {
                    mouse::ScrollDelta::Lines { y, .. } => -y * LINE,
                    mouse::ScrollDelta::Pixels { y, .. } => -y,
                };
                // Nothing to scroll: the wheel goes on to what holds it.
                if state.end <= 0.0 || dy == 0.0 {
                    return;
                }
                if state.scroll(dy, bounds.height, now) {
                    shell.capture_event();
                    shell.request_redraw();
                }
            }
            Event::Window(window::Event::RedrawRequested(_)) if state.settle(now) => {
                shell.request_redraw();
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
        let bounds = layout.bounds();
        let shown = tree.state.downcast_ref::<State>().shown(clock());
        let Some(content) = layout.children().next() else {
            return mouse::Interaction::None;
        };
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            content,
            moved(cursor, bounds, shown),
            &(*viewport + Vector::new(0.0, shown)),
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
        let shown = tree.state.downcast_ref::<State>().shown(clock());
        let Some(content) = layout.children().next() else {
            return;
        };
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };
        let inner_cursor = moved(cursor, bounds, shown);
        let inner_viewport = Rectangle {
            y: visible.y + shown,
            ..visible
        };
        iced::advanced::Renderer::with_layer(renderer, visible, |renderer| {
            iced::advanced::Renderer::with_translation(
                renderer,
                Vector::new(0.0, -shown),
                |renderer| {
                    self.content.as_widget().draw(
                        &tree.children[0],
                        renderer,
                        theme,
                        style,
                        content,
                        inner_cursor,
                        &inner_viewport,
                    );
                },
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
        let shown = tree.state.downcast_ref::<State>().shown(clock());
        let content = layout.children().next()?;
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            content,
            renderer,
            viewport,
            translation - Vector::new(0.0, shown),
        )
    }
}

/// The pointer in the content's frame: moved by the offset over the room, out of reach
/// elsewhere (a row scrolled out of the room is never hovered).
fn moved(cursor: mouse::Cursor, bounds: Rectangle, shown: f32) -> mouse::Cursor {
    match cursor.position() {
        Some(point) if bounds.contains(point) => {
            mouse::Cursor::Available(point + Vector::new(0.0, shown))
        }
        _ => mouse::Cursor::Unavailable,
    }
}

impl<'a, Message: 'a> From<Elastic<'a, Message>> for Element<'a, Message> {
    fn from(widget: Elastic<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rubber_band_follows_less_and_less_and_never_reaches_the_room() {
        let room = 300.0;
        assert_eq!(rubber(0.0, room), 0.0);
        let small = rubber(10.0, room);
        assert!(small > 0.0 && small < 10.0);
        assert!(rubber(10_000.0, room) < room);
        assert!(rubber(-50.0, room) < 0.0);
        for stretch in [1.0, 40.0, 150.0, -80.0] {
            let back = rubber(excess(stretch, room), room);
            assert!((back - stretch).abs() < 0.01, "{stretch} {back}");
        }
    }

    #[test]
    fn a_list_scrolls_within_its_ends_then_stretches_and_comes_back() {
        let mut state = State {
            end: 200.0,
            ..State::default()
        };
        let now = Duration::from_secs(1);
        assert!(state.scroll(150.0, 300.0, now));
        assert_eq!((state.offset, state.stretch(now)), (150.0, 0.0));
        // 100 asked, 50 left: the rest stretches, less than asked.
        assert!(state.scroll(100.0, 300.0, now));
        assert_eq!(state.offset, 200.0);
        let stretch = state.stretch(now);
        assert!(stretch > 0.0 && stretch < 50.0, "{stretch}");
        // It springs back to the end.
        let later = now + Duration::from_secs(2);
        assert!(!state.settle(later));
        assert_eq!(state.shown(later), 200.0);
        // Back the other way while stretched: the stretch gives way before the list moves.
        let _ = state.scroll(30.0, 300.0, later);
        let _ = state.scroll(-500.0, 300.0, later);
        assert_eq!(state.offset, 0.0);
        assert!(state.stretch(later) < 0.0, "stretched past the top");
    }
}
