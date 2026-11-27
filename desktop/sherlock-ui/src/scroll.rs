//! Overlay scrollbars in the manner of macOS (lot UI-11), with "Show scroll bars:
//! automatically": the one scroll area of the desktop, `scroll(content)`, in place of iced's
//! `scrollable`.
//!
//! - The scroller floats over the content: no rail at rest and no room taken, the content
//!   keeps its whole width.
//! - Hidden at rest, it fades in as soon as the area scrolls (wheel, touchpad, keyboard,
//!   a program), and fades out `LINGER` after the last move.
//! - A thin rounded pill (`THIN`), translucent, in the text color of the theme (dark on a light
//!   background, light on a dark one), with an edge in the window color that keeps it visible
//!   on any background.
//! - Over its zone (`ZONE`, at the right edge) it widens to `WIDE` and a faint rail appears:
//!   the knob can be dragged, and a click in the rail jumps by a page.
//! - At a limit, the wheel and the touchpad stretch the content like a rubber band, and it
//!   springs back once they stop; the knob shrinks against its end meanwhile.
//! - Its length follows the share of the content shown, never below `MIN_LENGTH`; nothing
//!   shows when the content fits.
//! - "Reduce motion" (`motion::reduced`) makes it appear and vanish at once, and turns off the
//!   stretch.
//!
//! The widget wraps an iced `Scrollable` whose own scrollbar is hidden (`Scrollbar::hidden`,
//! zero wide and floating): iced keeps the offset, the clipping, `on_scroll` and the `id`
//! that `operation::scroll_to` reaches. This widget reads the offset and moves it through
//! iced's scrollable operation, and draws the scroller itself. It scrolls vertically only.
//! See "Scroll areas (UI-11)" in `docs/sherlock-ui.md`.

use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{Id, Operation, Tree, operation, tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse, overlay};
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollable, Scrollbar, Viewport};
use iced::{
    Background, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, border,
};

use crate::animated::{self, Level};
use crate::color::{Color, Palette};
use crate::motion::{self, Animation, Spring};
use crate::theme;

// Tokens.

/// Width of the knob at rest, a pill.
pub const THIN: f32 = 6.0;
/// Width of the knob with the pointer over its zone, or dragged.
pub const WIDE: f32 = 10.0;
/// Gap between the knob and the right edge, and between the knob and the ends of the area.
pub const INSET: f32 = 3.0;
/// Width of the zone at the right edge where the pointer widens the knob, and of the rail.
pub const ZONE: f32 = WIDE + 2.0 * INSET;
/// Shortest knob, whatever the length of the content.
pub const MIN_LENGTH: f32 = 24.0;
/// Width of the edge drawn around the knob.
pub const EDGE: f32 = 1.0;
/// Opacity of the knob at rest, widened, and dragged.
pub const KNOB_ALPHA: (f32, f32, f32) = (0.42, 0.52, 0.64);
/// Opacity of the edge of the knob.
pub const EDGE_ALPHA: f32 = 0.55;
/// Opacity of the rail, and of the line along its left side.
pub const RAIL_ALPHA: (f32, f32) = (0.05, 0.10);
/// What a click in the rail keeps of the page it leaves: the jump is the height shown less
/// this.
pub const PAGE_OVERLAP: f32 = 24.0;
/// Pixels of one notch of a mouse wheel: iced's.
pub const WHEEL_LINE: f32 = 60.0;
/// Less content than this beyond the area: it fits.
const FIT_TOLERANCE: f32 = 0.5;

/// Fade in once the area scrolls.
pub const SHOW: Duration = Duration::from_millis(100);
/// Time the scroller stays after the last move, or after the pointer left its zone.
pub const LINGER: Duration = Duration::from_millis(1000);
/// Fade out after `LINGER`.
pub const HIDE: Duration = Duration::from_millis(300);
/// Widening and narrowing of the knob, and fade of the rail.
pub const EXPAND: Duration = motion::FADE;
/// Rest of the wheel after which a stretched area springs back.
pub const RELEASE: Duration = Duration::from_millis(80);
/// The spring back of a stretched area: critically damped, no overshoot past its limit.
pub const BOUNCE_SPRING: Spring = Spring::new(400.0, 40.0);
/// Resistance of the stretch: the `c` of the rubber band of macOS, `(1 - 1 / (x c / d + 1)) d`.
pub const ELASTICITY: f32 = 0.55;

// The pure part: what shows, where, and how far the area stretches, with time given.

/// What the scroller shows at an instant, each from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Opacity of the whole scroller.
    pub opacity: f32,
    /// From the thin knob without rail (0) to the wide knob over its rail (1).
    pub expansion: f32,
    /// Whether the knob is held.
    pub dragged: bool,
}

/// When the scroller shows itself and widens.
#[derive(Debug, Clone, Default)]
pub struct Presence {
    shown: Level,
    wide: Level,
    /// The last move, or the instant the pointer left the zone or let go of the knob: the
    /// linger counts from it.
    last: Option<Duration>,
    hovered: bool,
    dragged: bool,
}

impl Presence {
    /// The area scrolled at `now`.
    pub fn scrolled(&mut self, now: Duration) {
        self.last = Some(now);
        self.shown.set(true, now, motion::unless_reduced(SHOW));
    }

    /// The pointer is (or is no longer) over the zone of the scroller.
    pub fn hover(&mut self, over: bool, now: Duration) {
        if self.hovered == over {
            return;
        }
        self.hovered = over;
        self.widen(now);
    }

    /// The knob is (or is no longer) held.
    pub fn drag(&mut self, held: bool, now: Duration) {
        if self.dragged == held {
            return;
        }
        self.dragged = held;
        self.widen(now);
    }

    fn widen(&mut self, now: Duration) {
        let wide = self.hovered || self.dragged;
        if wide {
            self.shown.set(true, now, motion::unless_reduced(SHOW));
        } else {
            self.last = Some(now);
        }
        self.wide.set(wide, now, motion::unless_reduced(EXPAND));
    }

    /// Starts the fade out once the linger is over and forgets finished fades. Returns when to
    /// look again: `now` while a fade runs, the end of the linger while it waits, `None` at rest.
    pub fn tick(&mut self, now: Duration) -> Option<Duration> {
        let mut wake = None;
        if self.shown.on() && !self.hovered && !self.dragged {
            let end = self.last.unwrap_or(now) + LINGER;
            if now >= end {
                self.shown.set(false, now, motion::unless_reduced(HIDE));
            } else {
                wake = Some(end);
            }
        }
        // No short circuit: every finished fade is forgotten.
        let shown = self.shown.tick(now);
        let wide = self.wide.tick(now);
        if shown || wide { Some(now) } else { wake }
    }

    pub fn look(&self, now: Duration) -> Look {
        Look {
            opacity: self.shown.value(now),
            expansion: self.wide.value(now),
            dragged: self.dragged,
        }
    }
}

/// Whether content of height `content` overflows an area of height `area`.
pub fn overflows(area: f32, content: f32) -> bool {
    content > area + FIT_TOLERANCE
}

/// The knob of `width` in `bounds` for content of height `content` scrolled by `offset`,
/// with the area stretched by `stretch` (negative beyond the start): `None` when the content
/// fits.
pub fn knob(
    bounds: Rectangle,
    content: f32,
    offset: f32,
    stretch: f32,
    width: f32,
) -> Option<Rectangle> {
    if !overflows(bounds.height, content) {
        return None;
    }
    let (top, track, length) = track(bounds, content);
    let range = content - bounds.height;
    let share = (offset / range).clamp(0.0, 1.0);
    let squeezed = (length - stretch.abs()).max(width.min(length));
    let y = if stretch < 0.0 {
        top
    } else if stretch > 0.0 {
        top + track - squeezed
    } else {
        top + (track - length) * share
    };
    Some(Rectangle {
        x: bounds.x + bounds.width - INSET - width,
        y,
        width,
        height: squeezed,
    })
}

/// Top and length of the track, and length of the knob.
fn track(bounds: Rectangle, content: f32) -> (f32, f32, f32) {
    let track = (bounds.height - 2.0 * INSET).max(0.0);
    let length = (track * bounds.height / content.max(1.0))
        .max(MIN_LENGTH)
        .min(track);
    (bounds.y + INSET, track, length)
}

/// The offset that puts the top of the knob at `knob_top`: dragging it.
pub fn offset_at(bounds: Rectangle, content: f32, knob_top: f32) -> f32 {
    let (top, track, length) = track(bounds, content);
    let free = track - length;
    if free <= 0.0 {
        return 0.0;
    }
    ((knob_top - top) / free).clamp(0.0, 1.0) * (content - bounds.height).max(0.0)
}

/// The jump of a click in the rail, for an area of height `area`.
pub fn page(area: f32) -> f32 {
    (area - PAGE_OVERLAP).max(area / 2.0)
}

/// How far an area of height `area` shows stretched when pulled by `pull` past its limit:
/// less and less, never `area`.
pub fn rubber(pull: f32, area: f32) -> f32 {
    let d = area.max(1.0);
    pull.signum() * (1.0 - 1.0 / (pull.abs() * ELASTICITY / d + 1.0)) * d
}

/// The pull that shows a stretch of `shown`: the inverse of `rubber`.
pub fn unrubber(shown: f32, area: f32) -> f32 {
    let d = area.max(1.0);
    let r = (shown.abs() / d).min(0.99);
    shown.signum() * d / ELASTICITY * (1.0 / (1.0 - r) - 1.0)
}

/// The stretch of an area pulled past a limit by the wheel or the touchpad, then its spring
/// back. Negative beyond the start, positive beyond the end.
#[derive(Debug, Clone, Default)]
pub struct Stretch {
    pull: f32,
    last_push: Option<Duration>,
    back: Option<Animation>,
}

impl Stretch {
    /// The stretch shown at `now` for an area of height `area`.
    pub fn shown(&self, now: Duration, area: f32) -> f32 {
        match &self.back {
            Some(back) => back.value(now) as f32,
            None => rubber(self.pull, area),
        }
    }

    /// Whether the area shows stretched: the wheel then acts on the stretch first.
    pub fn active(&self) -> bool {
        self.pull != 0.0 || self.back.is_some()
    }

    /// The wheel pushes by `delta` past the limit, or back towards it.
    pub fn push(&mut self, delta: f32, now: Duration, area: f32) {
        if let Some(back) = self.back.take() {
            self.pull = unrubber(back.value(now) as f32, area);
        }
        let pulled = self.pull + delta;
        self.pull = if self.pull != 0.0 && pulled.signum() != self.pull.signum() {
            0.0
        } else {
            pulled
        };
        self.last_push = Some(now);
    }

    /// Springs back once the wheel rests, and forgets a finished spring. Returns when to look
    /// again, as `Presence::tick`.
    pub fn tick(&mut self, now: Duration, area: f32) -> Option<Duration> {
        if let Some(back) = &self.back {
            if back.is_done(now, motion::GEOMETRY_PRECISION) {
                self.back = None;
                return None;
            }
            return Some(now);
        }
        if self.pull == 0.0 {
            return None;
        }
        let release = self.last_push.unwrap_or(now) + RELEASE;
        if now < release {
            return Some(release);
        }
        let from = f64::from(rubber(self.pull, area));
        self.pull = 0.0;
        self.back = Some(Animation::new(BOUNCE_SPRING, from, 0.0, now));
        Some(now)
    }
}

/// Colors of the scroller in `palette`: knob, edge, rail, line of the rail, before the opacity
/// of the look.
fn colors(palette: &Palette, look: Look) -> (Color, Color, Color, Color) {
    let (rest, wide, held) = KNOB_ALPHA;
    let knob = if look.dragged {
        held
    } else {
        rest + (wide - rest) * look.expansion
    };
    let ink = palette.text_primary;
    let tint = |a: f32| Color {
        a: a * look.opacity,
        ..ink
    };
    let edge = Color {
        a: EDGE_ALPHA * look.opacity,
        ..palette.window
    };
    let (rail, line) = RAIL_ALPHA;
    (
        tint(knob),
        edge,
        tint(rail * look.expansion),
        tint(line * look.expansion),
    )
}

// The widget.

/// A vertical scroll area with an overlay scroller.
pub struct Scroll<'a, Message> {
    inner: Scrollable<'a, Message, Theme, Renderer>,
    elastic: bool,
}

/// Scrolls `content` vertically, the scroller floating over it.
pub fn scroll<'a, Message: 'a>(content: impl Into<Element<'a, Message>>) -> Scroll<'a, Message> {
    Scroll {
        inner: Scrollable::with_direction(content, Direction::Vertical(Scrollbar::hidden())),
        elastic: true,
    }
}

impl<'a, Message: 'a> Scroll<'a, Message> {
    /// The id `operation::scroll_to` and `operation::snap_to` reach.
    pub fn id(mut self, id: impl Into<Id>) -> Self {
        self.inner = self.inner.id(id);
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.inner = self.inner.width(width);
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.inner = self.inner.height(height);
        self
    }

    /// The message sent when the area scrolls, or its size or that of its content changes.
    pub fn on_scroll(mut self, f: impl Fn(Viewport) -> Message + 'a) -> Self {
        self.inner = self.inner.on_scroll(f);
        self
    }

    /// Whether the area stretches at its limits (on by default).
    pub fn elastic(mut self, elastic: bool) -> Self {
        self.elastic = elastic;
        self
    }
}

/// State of one scroll area.
#[derive(Debug, Default)]
struct State {
    presence: Presence,
    stretch: Stretch,
    /// Where the knob was grabbed, from its top, while it is held.
    grab: Option<f32>,
    /// Offset and content height last read: an offset that changes is a scroll, whoever made it.
    offset: Option<f32>,
    content: f32,
}

/// What the scroll area reads of iced's scrollable it wraps: its own state comes first; with
/// `cursor`, it also looks for a scroll area inside it, under that point, that can scroll.
struct Read {
    ours: Option<(Rectangle, f32, f32)>,
    cursor: Option<Point>,
    nested: bool,
}

impl Operation for Read {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        if self.cursor.is_some() {
            operate(self);
        }
    }

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn operation::Scrollable,
    ) {
        if self.ours.is_none() {
            self.ours = Some((bounds, content_bounds.height, translation.y));
        } else if self
            .cursor
            .is_some_and(|p| bounds.contains(p) && overflows(bounds.height, content_bounds.height))
        {
            self.nested = true;
        }
    }
}

/// Moves the wrapped scrollable: by a distance, or to an offset.
enum Move {
    By(f32),
    To(f32),
}

impl Operation for Move {
    fn traverse(&mut self, _operate: &mut dyn FnMut(&mut dyn Operation)) {}

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        match *self {
            Move::By(y) => state.scroll_by(AbsoluteOffset { x: 0.0, y }, bounds, content_bounds),
            Move::To(y) => {
                let range = (content_bounds.height - bounds.height).max(0.0);
                state.scroll_to(AbsoluteOffset {
                    x: None,
                    y: Some(y.clamp(0.0, range)),
                });
            }
        }
    }
}

impl<Message> Scroll<'_, Message> {
    fn as_widget(&self) -> &dyn Widget<Message, Theme, Renderer> {
        &self.inner
    }

    /// Bounds, content height and offset of the wrapped scrollable, and whether a scroll area
    /// inside it, under `cursor`, can scroll.
    fn read(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        cursor: Option<Point>,
    ) -> Option<(Rectangle, f32, f32, bool)> {
        let mut read = Read {
            ours: None,
            cursor,
            nested: false,
        };
        if let Some(inner) = tree.children.first_mut() {
            self.inner.operate(inner, layout, renderer, &mut read);
        }
        read.ours
            .map(|(bounds, content, offset)| (bounds, content, offset, read.nested))
    }

    fn move_to(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &Renderer, to: Move) {
        let mut to = to;
        if let Some(inner) = tree.children.first_mut() {
            self.inner.operate(inner, layout, renderer, &mut to);
        }
    }
}

/// The zone of the scroller in `bounds`.
fn zone(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: bounds.x + bounds.width - ZONE,
        width: ZONE.min(bounds.width),
        ..bounds
    }
}

/// The cursor the content sees: none over the scroller, and moved with the stretch.
fn inner_cursor(cursor: mouse::Cursor, over_zone: bool, stretch: f32) -> mouse::Cursor {
    let cursor = if over_zone { cursor.levitate() } else { cursor };
    cursor + Vector::new(0.0, stretch)
}

impl<Message> Widget<Message, Theme, Renderer> for Scroll<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(self.as_widget())]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[self.as_widget()]);
    }

    fn size(&self) -> Size<Length> {
        self.inner.size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.inner.layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.inner
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
        let now = animated::clock();
        let Some((bounds, content, offset, _)) = self.read(tree, layout, renderer, None) else {
            return;
        };
        let area = bounds.height;
        let overflow = overflows(area, content);
        let elastic = self.elastic && overflow && !motion::reduced();
        let state = tree.state.downcast_mut::<State>();
        let over_zone = overflow && cursor.is_over(zone(bounds)) && cursor.is_over(*viewport)
            || state.grab.is_some();
        if let Event::Mouse(_) = event {
            state.presence.hover(over_zone, now);
        }
        let stretch = state.stretch.shown(now, area);

        let mut to = None;
        let handled = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if over_zone && state.grab.is_none() =>
            {
                if let (Some(at), Some(knob)) =
                    (cursor.position(), knob(bounds, content, offset, 0.0, WIDE))
                {
                    if (knob.y..knob.y + knob.height).contains(&at.y) {
                        state.grab = Some(at.y - knob.y);
                        state.presence.drag(true, now);
                    } else {
                        let jump = page(area) * if at.y < knob.y { -1.0 } else { 1.0 };
                        to = Some(Move::By(jump));
                    }
                }
                true
            }
            Event::Mouse(mouse::Event::CursorMoved { position }) if state.grab.is_some() => {
                let grab = state.grab.unwrap_or_default();
                to = Some(Move::To(offset_at(bounds, content, position.y - grab)));
                true
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if state.grab.is_some() =>
            {
                state.grab = None;
                state.presence.drag(false, now);
                true
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if elastic && state.stretch.active() && cursor.is_over(bounds) =>
            {
                // A stretched area takes the wheel until it is back at its limit.
                state.stretch.push(wheel(*delta), now, area);
                state.presence.scrolled(now);
                true
            }
            _ => false,
        };

        if let Some(to) = to {
            self.move_to(tree, layout, renderer, to);
        }
        if handled {
            shell.capture_event();
            shell.request_redraw();
        } else {
            self.inner.update(
                &mut tree.children[0],
                event,
                layout,
                inner_cursor(cursor, over_zone, stretch),
                renderer,
                clipboard,
                shell,
                viewport,
            );
            if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event
                && elastic
            {
                self.overscroll(tree, layout, renderer, cursor, wheel(*delta), offset, shell);
            }
        }

        // An offset that changed is a scroll, whoever made it: the wheel, the knob, a key, a
        // program.
        let read = self.read(tree, layout, renderer, None);
        let state = tree.state.downcast_mut::<State>();
        if let Some((_, content, after, _)) = read {
            if state
                .offset
                .is_some_and(|before| (before - after).abs() > 0.01)
            {
                state.presence.scrolled(now);
            }
            state.offset = Some(after);
            state.content = content;
        }
        if !overflow {
            state.presence.hover(false, now);
        }
        let wake = [state.presence.tick(now), state.stretch.tick(now, area)]
            .into_iter()
            .flatten()
            .min();
        match wake {
            Some(at) if at <= now => shell.request_redraw(),
            Some(at) => shell.request_redraw_at(Instant::now() + (at - now)),
            None => {}
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
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        if state.grab.is_some()
            || overflows(bounds.height, state.content) && cursor.is_over(zone(bounds))
        {
            return mouse::Interaction::Idle;
        }
        let stretch = state.stretch.shown(animated::clock(), bounds.height);
        self.inner.mouse_interaction(
            &tree.children[0],
            layout,
            inner_cursor(cursor, false, stretch),
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
        let state = tree.state.downcast_ref::<State>();
        let now = animated::clock();
        let bounds = layout.bounds();
        let content = layout
            .children()
            .next()
            .map_or(0.0, |content| content.bounds().height);
        let overflow = overflows(bounds.height, content);
        let stretch = if overflow {
            state.stretch.shown(now, bounds.height)
        } else {
            0.0
        };
        let over_zone = overflow && cursor.is_over(zone(bounds)) || state.grab.is_some();
        let cursor = inner_cursor(cursor, over_zone, stretch);
        let Some(visible) = bounds.intersection(viewport) else {
            return;
        };
        if stretch.abs() < 0.1 {
            self.inner.draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
        } else {
            let shift = Vector::new(0.0, -stretch);
            let moved = Rectangle {
                y: viewport.y + stretch,
                ..*viewport
            };
            iced::advanced::Renderer::with_layer(renderer, visible, |renderer| {
                iced::advanced::Renderer::with_translation(renderer, shift, |renderer| {
                    self.inner.draw(
                        &tree.children[0],
                        renderer,
                        theme,
                        style,
                        layout,
                        cursor,
                        &moved,
                    );
                });
            });
        }

        let look = state.presence.look(now);
        if !overflow || look.opacity <= 0.0 {
            return;
        }
        let width = THIN + (WIDE - THIN) * look.expansion;
        let offset = state.offset.unwrap_or_default();
        let Some(knob) = knob(bounds, content, offset, stretch, width) else {
            return;
        };
        let (fill, edge, rail, line) = colors(theme::palette(theme), look);
        iced::advanced::Renderer::with_layer(renderer, visible, |renderer| {
            if look.expansion > 0.0 {
                let rail_bounds = zone(bounds);
                quad(renderer, rail_bounds, 0.0, rail, None);
                let line_bounds = Rectangle {
                    width: EDGE,
                    ..rail_bounds
                };
                quad(renderer, line_bounds, 0.0, line, None);
            }
            quad(renderer, knob, width / 2.0, fill, Some(edge));
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
        self.inner.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<Message> Scroll<'_, Message> {
    /// After the wrapped scrollable took a wheel move of `delta` from `before`: what it could
    /// not travel past a limit stretches the area, unless a scroll area inside, under the
    /// pointer, took it.
    #[allow(clippy::too_many_arguments)]
    fn overscroll(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        cursor: mouse::Cursor,
        delta: f32,
        before: f32,
        shell: &mut Shell<'_, Message>,
    ) {
        let Some(at) = cursor.position_over(layout.bounds()) else {
            return;
        };
        let Some((bounds, content, after, _)) = self.read(tree, layout, renderer, None) else {
            return;
        };
        let range = content - bounds.height;
        let at_limit =
            delta < 0.0 && after <= FIT_TOLERANCE || delta > 0.0 && after >= range - FIT_TOLERANCE;
        let rest = delta - (after - before);
        if !at_limit || rest.abs() <= FIT_TOLERANCE || rest.signum() != delta.signum() {
            return;
        }
        let point = at + Vector::new(0.0, after);
        if self
            .read(tree, layout, renderer, Some(point))
            .is_some_and(|(.., nested)| nested)
        {
            return;
        }
        let now = animated::clock();
        let state = tree.state.downcast_mut::<State>();
        state.stretch.push(rest, now, bounds.height);
        state.presence.scrolled(now);
        shell.capture_event();
        shell.request_redraw();
    }
}

/// Pixels a wheel move scrolls, positive downwards: iced's own reading.
fn wheel(delta: mouse::ScrollDelta) -> f32 {
    match delta {
        mouse::ScrollDelta::Lines { y, .. } => -y * WHEEL_LINE,
        mouse::ScrollDelta::Pixels { y, .. } => -y,
    }
}

fn quad(renderer: &mut Renderer, bounds: Rectangle, radius: f32, fill: Color, edge: Option<Color>) {
    if fill.a <= 0.0 {
        return;
    }
    let border = match edge {
        Some(edge) => border::rounded(radius).width(EDGE).color(edge),
        None => border::rounded(radius),
    };
    iced::advanced::Renderer::fill_quad(
        renderer,
        Quad {
            bounds,
            border,
            ..Quad::default()
        },
        Background::Color(fill.into()),
    );
}

impl<'a, Message: 'a> From<Scroll<'a, Message>> for Element<'a, Message> {
    fn from(widget: Scroll<'a, Message>) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Reduce motion" is one switch for the process: the tests that read it take turns.
    static MOTION: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn motion_lock() -> std::sync::MutexGuard<'static, ()> {
        MOTION
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    const AREA: Rectangle = Rectangle {
        x: 0.0,
        y: 0.0,
        width: 300.0,
        height: 400.0,
    };

    #[test]
    fn hidden_at_rest_shown_while_scrolling_then_gone_after_the_linger() {
        let _turn = motion_lock();
        let mut p = Presence::default();
        assert_eq!(p.tick(ms(0)), None, "nothing runs at rest");
        assert_eq!(p.look(ms(0)).opacity, 0.0);
        p.scrolled(ms(100));
        assert!(p.look(ms(150)).opacity > 0.0 && p.look(ms(150)).opacity < 1.0);
        assert_eq!(p.tick(ms(150)), Some(ms(150)), "fading in: next frame");
        assert_eq!(p.look(ms(100) + SHOW).opacity, 1.0);
        // Shown and waiting: it asks to be woken at the end of the linger, not every frame.
        assert_eq!(p.tick(ms(400)), Some(ms(100) + LINGER));
        // Another scroll pushes the linger back.
        p.scrolled(ms(800));
        assert_eq!(p.tick(ms(1200)), Some(ms(800) + LINGER));
        let end = ms(800) + LINGER;
        assert_eq!(p.tick(end), Some(end), "the fade out starts");
        assert!(p.look(end + HIDE / 2).opacity > 0.0);
        assert_eq!(p.look(end + HIDE).opacity, 0.0);
        assert_eq!(p.tick(end + HIDE), None, "back at rest");
        assert_eq!(p.look(ms(0)).expansion, 0.0, "scrolling never widens it");
    }

    #[test]
    fn hovering_the_zone_widens_it_and_keeps_it_until_the_pointer_leaves() {
        let _turn = motion_lock();
        let mut p = Presence::default();
        p.hover(true, ms(0));
        let shown = p.look(SHOW.max(EXPAND));
        assert_eq!((shown.opacity, shown.expansion), (1.0, 1.0));
        // The linger does not run while the pointer stays.
        assert_eq!(p.tick(ms(5000)), None);
        assert_eq!(p.look(ms(5000)).opacity, 1.0);
        p.hover(false, ms(6000));
        assert_eq!(p.look(ms(6000) + EXPAND).expansion, 0.0);
        assert_eq!(p.look(ms(6000) + EXPAND).opacity, 1.0, "it lingers");
        let _ = p.tick(ms(6000) + LINGER);
        assert_eq!(p.look(ms(6000) + LINGER + HIDE).opacity, 0.0);
    }

    #[test]
    fn a_held_knob_stays_wide_even_off_its_zone() {
        let _turn = motion_lock();
        let mut p = Presence::default();
        p.hover(true, ms(0));
        p.drag(true, ms(10));
        p.hover(false, ms(20));
        let look = p.look(ms(500));
        assert!(look.dragged && look.expansion == 1.0 && look.opacity == 1.0);
        assert_eq!(p.tick(ms(5000)), None);
        p.drag(false, ms(6000));
        assert_eq!(p.look(ms(6000) + EXPAND).expansion, 0.0);
    }

    #[test]
    fn reduce_motion_switches_without_fading() {
        let _turn = motion_lock();
        motion::set_reduced(true);
        let mut p = Presence::default();
        p.scrolled(ms(0));
        assert_eq!(p.look(ms(0)).opacity, 1.0);
        let _ = p.tick(LINGER);
        assert_eq!(p.look(LINGER).opacity, 0.0);
        motion::set_reduced(false);
    }

    #[test]
    fn the_knob_is_proportional_with_a_minimum_and_absent_when_the_content_fits() {
        assert_eq!(knob(AREA, 400.0, 0.0, 0.0, THIN), None);
        assert_eq!(knob(AREA, 300.0, 0.0, 0.0, THIN), None);
        let track = AREA.height - 2.0 * INSET;
        // Twice the area: half the track, at the top, then at the bottom.
        let top = knob(AREA, 800.0, 0.0, 0.0, THIN).unwrap();
        assert!((top.height - track / 2.0).abs() < 0.01);
        assert_eq!(top.y, INSET);
        assert_eq!(
            top.x + top.width,
            AREA.width - INSET,
            "against the right edge"
        );
        let bottom = knob(AREA, 800.0, 400.0, 0.0, THIN).unwrap();
        assert!((bottom.y + bottom.height - (AREA.height - INSET)).abs() < 0.01);
        // A very long content: never below the minimum.
        assert_eq!(
            knob(AREA, 1.0e6, 0.0, 0.0, THIN).unwrap().height,
            MIN_LENGTH
        );
        // Dragging the knob back gives the offset it shows.
        let middle = knob(AREA, 800.0, 200.0, 0.0, WIDE).unwrap();
        assert!((offset_at(AREA, 800.0, middle.y) - 200.0).abs() < 0.01);
        assert_eq!(offset_at(AREA, 800.0, -50.0), 0.0);
        assert_eq!(offset_at(AREA, 800.0, 1.0e4), 400.0);
    }

    #[test]
    fn a_stretched_area_squeezes_the_knob_against_its_end() {
        let rest = knob(AREA, 800.0, 0.0, 0.0, THIN).unwrap();
        let pulled = knob(AREA, 800.0, 0.0, -40.0, THIN).unwrap();
        assert_eq!(pulled.y, rest.y);
        assert!((pulled.height - (rest.height - 40.0)).abs() < 0.01);
        let end = knob(AREA, 800.0, 400.0, 1.0e4, THIN).unwrap();
        assert_eq!(end.height, THIN, "squeezed to a dot, no further");
        assert!((end.y + end.height - (AREA.height - INSET)).abs() < 0.01);
    }

    #[test]
    fn the_rubber_band_resists_more_and_more_and_springs_back() {
        let _turn = motion_lock();
        let area = 400.0;
        assert_eq!(rubber(0.0, area), 0.0);
        let (a, b) = (rubber(50.0, area), rubber(100.0, area));
        assert!(
            a > 0.0 && a < 50.0 && b > a && b - a < a,
            "resists more and more"
        );
        assert!(rubber(1.0e6, area) < area);
        assert_eq!(rubber(-50.0, area), -a);
        assert!((unrubber(a, area) - 50.0).abs() < 0.01);

        let mut s = Stretch::default();
        s.push(-60.0, ms(0), area);
        assert!(s.active() && s.shown(ms(0), area) < 0.0);
        assert_eq!(
            s.tick(ms(10), area),
            Some(RELEASE),
            "waits for the wheel to rest"
        );
        // Pushing back towards the limit undoes the stretch, never past it.
        s.push(200.0, ms(20), area);
        assert_eq!(s.shown(ms(20), area), 0.0);
        s.push(-60.0, ms(30), area);
        let pulled = s.shown(ms(30), area);
        assert_eq!(s.tick(ms(30) + RELEASE, area), Some(ms(30) + RELEASE));
        assert!(
            (s.shown(ms(30) + RELEASE, area) - pulled).abs() < 0.01,
            "no jump"
        );
        assert!(s.shown(ms(30) + RELEASE + ms(100), area) > pulled);
        let later = ms(30) + RELEASE + ms(1500);
        assert_eq!(s.tick(later, area), None);
        assert!(!s.active());
        assert_eq!(s.shown(later, area), 0.0);
    }

    #[test]
    fn the_scroller_follows_the_theme() {
        let look = Look {
            opacity: 1.0,
            expansion: 0.0,
            dragged: false,
        };
        for p in [&Palette::DARK, &Palette::LIGHT] {
            let (knob, edge, rail, _) = colors(p, look);
            // Dark on a light window, light on a dark one, and its edge stands apart from it.
            assert!(knob.contrast(p.window) > 3.0);
            assert!(edge.contrast(knob) > 3.0);
            assert_eq!(rail.a, 0.0, "no rail at rest");
        }
        let faded = colors(
            &Palette::DARK,
            Look {
                opacity: 0.5,
                ..look
            },
        );
        assert_eq!(faded.0.a, KNOB_ALPHA.0 * 0.5);
    }

    #[test]
    fn a_page_jump_keeps_a_little_of_the_page() {
        assert_eq!(page(400.0), 400.0 - PAGE_OVERLAP);
        assert_eq!(page(30.0), 15.0);
    }
}
