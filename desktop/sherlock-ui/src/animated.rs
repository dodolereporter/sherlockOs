//! Animated widgets, on the physics of `motion`: the hover veil that fades in, and `Reveal`,
//! which draws its content faded, moved and scaled.
//!
//! An animated widget requests the next frame itself (`Shell::request_redraw`) while its
//! animation runs, and nothing afterwards: nothing redraws at rest. `Reveal` holds no
//! animation: the program gives it the values of its own springs at each frame.

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

use crate::color::{self, Color};
use crate::metrics;
use crate::motion::{self, Animation, Fade};

/// Time elapsed since the first call: the common origin of the widgets' animations.
pub fn clock() -> Duration {
    at(Instant::now())
}

/// `instant` on the clock of `clock`: the instant of a frame (`window::frames`).
pub fn at(instant: Instant) -> Duration {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    instant.saturating_duration_since(*ORIGIN.get_or_init(Instant::now))
}

/// A state that fades in and out instead of switching: hovered, pressed, open.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Level {
    on: bool,
    fade: Option<Fade>,
}

impl Level {
    /// At rest in `on`, without a fade.
    pub fn new(on: bool) -> Self {
        Self { on, fade: None }
    }

    pub fn on(&self) -> bool {
        self.on
    }

    /// From 0 (off) to 1 (on).
    pub fn value(&self, now: Duration) -> f32 {
        match &self.fade {
            Some(fade) => fade.value(now),
            None => f32::from(u8::from(self.on)),
        }
    }

    /// Heads for `on` over `duration` from where it is. Returns whether it changed.
    pub fn set(&mut self, on: bool, now: Duration, duration: Duration) -> bool {
        if on == self.on {
            return false;
        }
        self.on = on;
        let to = f32::from(u8::from(on));
        match &mut self.fade {
            Some(fade) => fade.retarget(now, to, duration),
            None => self.fade = Some(Fade::new(1.0 - to, to, now, duration)),
        }
        true
    }

    /// Forgets a finished fade. Returns whether one still runs.
    pub fn tick(&mut self, now: Duration) -> bool {
        match &self.fade {
            Some(fade) if fade.is_done(now) => {
                self.fade = None;
                false
            }
            Some(_) => true,
            None => false,
        }
    }
}

/// Content that lays a veil under itself on hover, fading over `motion::FADE` (150 ms), and
/// optionally a deeper veil while pressed, a veil and a mark while it is open, and a slight
/// shrink while pressed.
pub struct Hoverable<'a, Message> {
    content: Element<'a, Message>,
    fill: Color,
    radius: f32,
    key: Option<u64>,
    pressed: Option<Color>,
    open: Option<(bool, Color)>,
    mark: Option<Color>,
    press_scale: f32,
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
        pressed: None,
        open: None,
        mark: None,
        press_scale: 1.0,
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

    /// The veil while a mouse button is held on it, reached over `motion::PRESS_FADE`.
    pub fn pressed(mut self, fill: Color) -> Self {
        self.pressed = Some(fill);
        self
    }

    /// The veil while what it opens is open (a panel, a menu): it fades in and out when
    /// `open` changes from one view to the next.
    pub fn open(mut self, open: bool, fill: Color) -> Self {
        self.open = Some((open, fill));
        self
    }

    /// A short line under the content while it is open, in `tint`.
    pub fn mark(mut self, tint: Color) -> Self {
        self.mark = Some(tint);
        self
    }

    /// The content shrinks to `scale` while pressed, around its centre.
    pub fn press_scale(mut self, scale: f32) -> Self {
        self.press_scale = scale;
        self
    }
}

/// Hover state of one widget: whether the pointer is over it, pressed on it, and whether it is
/// open, each with its fade.
#[derive(Debug, Default)]
struct HoverState {
    key: Option<u64>,
    hover: Level,
    press: Level,
    /// `None` until the first view gives it: the first state shows at once.
    open: Option<Level>,
}

impl HoverState {
    /// Opacity of the hover veil, from 0 to 1.
    fn value(&self, now: Duration) -> f32 {
        self.hover.value(now)
    }

    /// The pointer is (or is no longer) over the widget. Returns whether a fade started.
    fn set(&mut self, over: bool, now: Duration) -> bool {
        let pressed = !over && self.press.set(false, now, motion::PRESS_FADE);
        self.hover.set(over, now, motion::FADE) || pressed
    }

    /// Whether it is open, from the view.
    fn open(&mut self, open: bool, now: Duration) {
        match &mut self.open {
            Some(level) => {
                level.set(open, now, motion::FADE);
            }
            None => self.open = Some(Level::new(open)),
        }
    }

    /// Forgets finished fades. Returns whether one still runs.
    fn tick(&mut self, now: Duration) -> bool {
        let open = self.open.as_mut().is_some_and(|level| level.tick(now));
        // No short circuit: every finished fade is forgotten.
        let hover = self.hover.tick(now);
        let press = self.press.tick(now);
        hover || press || open
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

/// `fill` with its opacity scaled by `share`.
fn faded(fill: Color, share: f32) -> Color {
    Color {
        a: fill.a * share.clamp(0.0, 1.0),
        ..fill
    }
}

fn veil(renderer: &mut Renderer, bounds: Rectangle, radius: f32, fill: Color) {
    if fill.a <= 0.0 {
        return;
    }
    let quad = Quad {
        bounds,
        border: border::rounded(radius),
        ..Quad::default()
    };
    iced::advanced::Renderer::fill_quad(renderer, quad, Background::Color(fill.into()));
}

impl<Message> Widget<Message, Theme, Renderer> for Hoverable<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<HoverState>()
    }

    fn state(&self) -> tree::State {
        let mut state = HoverState::default();
        if let Some((open, _)) = self.open {
            state.open = Some(Level::new(open));
        }
        tree::State::new(state)
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<HoverState>();
        state.rekey(self.key);
        if let Some((open, _)) = self.open {
            state.open(open, clock());
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
        let over = cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
        let mut moved = false;
        // Any pointer event, a wheel scroll included, can change what is under the pointer:
        // a list that scrolls brings another row under a still pointer.
        if let Event::Mouse(event) = event {
            moved |= state.set(over, now);
            if self.pressed.is_some() {
                match event {
                    mouse::Event::ButtonPressed(mouse::Button::Left) if over => {
                        moved |= state.press.set(true, now, motion::PRESS_FADE);
                    }
                    mouse::Event::ButtonReleased(mouse::Button::Left) => {
                        moved |= state.press.set(false, now, motion::PRESS_FADE);
                    }
                    _ => {}
                }
            }
        }
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            moved |= state.tick(now);
        }
        if moved {
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
        let state = tree.state.downcast_ref::<HoverState>();
        let now = clock();
        let bounds = layout.bounds();
        let open = state.open.map_or(0.0, |level| level.value(now));
        if let Some((_, fill)) = self.open {
            veil(renderer, bounds, self.radius, faded(fill, open));
        }
        let hover = state.value(now);
        let press = state.press.value(now);
        let fill = match self.pressed {
            Some(pressed) => faded(self.fill, hover).lerp(pressed, press),
            None => faded(self.fill, hover),
        };
        veil(renderer, bounds, self.radius, fill);
        if let (Some(tint), true) = (self.mark, open > 0.0) {
            let (width, height) = metrics::OPEN_MARK;
            let mark = Rectangle {
                x: bounds.center_x() - width * open / 2.0,
                y: bounds.y + bounds.height - height - metrics::OPEN_MARK_INSET,
                width: width * open,
                height,
            };
            veil(renderer, mark, height / 2.0, faded(tint, open));
        }
        let draw = |renderer: &mut Renderer| {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                layout,
                cursor,
                viewport,
            );
        };
        let scale = 1.0 - (1.0 - self.press_scale) * press;
        if scale < 1.0 {
            let centre = bounds.center();
            let shrunk = Transformation::translate(centre.x, centre.y)
                * Transformation::scale(scale)
                * Transformation::translate(-centre.x, -centre.y);
            iced::advanced::Renderer::with_transformation(renderer, shrunk, draw);
        } else {
            draw(renderer);
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

impl<'a, Message: 'a> From<Hoverable<'a, Message>> for Element<'a, Message> {
    fn from(widget: Hoverable<'a, Message>) -> Self {
        Element::new(widget)
    }
}

/// Content drawn with an opacity, an offset and a scale around a point of its bounds: a panel
/// that unfolds from the bar, a page that slides in. Its layout and its input do not move: at
/// rest (opacity 1, no offset, scale 1) it is its content.
pub struct Reveal<'a, Message> {
    content: Element<'a, Message>,
    opacity: f32,
    offset: Vector,
    scale: f32,
    /// The point the scale keeps in place, as a share of the width and the height.
    origin: (f32, f32),
    clip: Clip,
    /// Drawn only: it takes no input and no focus (a page on its way out).
    inert: bool,
}

/// How far a mask cut below only reaches above and on the sides: beyond any surface.
const UNCUT: f32 = 1.0e4;

/// Where a moving `Reveal` is cut, so that nothing it draws leaves its panel.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Clip {
    None,
    /// Its bounds grown by this much on every side, while it moves.
    Margin(f32),
    /// Down to this height from the top of its bounds, nothing cut above nor on the sides.
    Height(f32),
}

/// `content` at rest; see the setters.
pub fn reveal<'a, Message>(content: impl Into<Element<'a, Message>>) -> Reveal<'a, Message> {
    Reveal {
        content: content.into(),
        opacity: 1.0,
        offset: Vector::ZERO,
        scale: 1.0,
        origin: (0.5, 0.0),
        clip: Clip::None,
        inert: false,
    }
}

impl<Message> Reveal<'_, Message> {
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    pub fn offset(mut self, x: f32, y: f32) -> Self {
        self.offset = Vector::new(x, y);
        self
    }

    /// Scaled by `scale` around `origin` (shares of the width and the height).
    pub fn scale(mut self, scale: f32, origin: (f32, f32)) -> Self {
        self.scale = scale;
        self.origin = origin;
        self
    }

    /// While it moves, nothing is drawn further than `margin` from its bounds: a page that
    /// slides stays inside the glass of its panel, its padding being `margin`.
    pub fn clip_margin(mut self, margin: f32) -> Self {
        self.clip = Clip::Margin(margin);
        self
    }

    /// Nothing is drawn below `height` from its top, everything above and on its sides: a panel
    /// unfolding by a mask, its shadow and its sides left whole.
    pub fn clip_height(mut self, height: f32) -> Self {
        self.clip = Clip::Height(height);
        self
    }

    /// Drawn only, without input nor focus: a page or a panel on its way out.
    pub fn inert(mut self) -> Self {
        self.inert = true;
        self
    }

    /// The area it is cut to, if any, for its bounds.
    fn clip_area(&self, bounds: Rectangle) -> Option<Rectangle> {
        let moving = self.offset != Vector::ZERO || self.scale != 1.0;
        match self.clip {
            Clip::Margin(margin) if moving => Some(bounds.expand(margin)),
            // Cut below only: above and on the sides, everything it draws stays, a panel moving
            // sideways and its shadow included.
            Clip::Height(height) => Some(Rectangle {
                x: bounds.x - UNCUT,
                y: bounds.y - UNCUT,
                width: bounds.width + 2.0 * UNCUT,
                height: UNCUT + height.max(0.0),
            }),
            Clip::Margin(_) | Clip::None => None,
        }
    }

    /// Where the content is drawn: from its bounds to the screen.
    fn transformation(&self, bounds: Rectangle) -> Transformation {
        let pivot = Point::new(
            bounds.x + bounds.width * self.origin.0,
            bounds.y + bounds.height * self.origin.1,
        );
        Transformation::translate(pivot.x + self.offset.x, pivot.y + self.offset.y)
            * Transformation::scale(self.scale)
            * Transformation::translate(-pivot.x, -pivot.y)
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Reveal<'_, Message> {
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
        if self.inert {
            return;
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
        if self.inert {
            return;
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
        if self.inert {
            return mouse::Interaction::None;
        }
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
        if self.opacity <= 0.0 {
            return;
        }
        let draw = |renderer: &mut Renderer| {
            color::with_opacity(self.opacity, || {
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
        };
        let bounds = layout.bounds();
        let placed = |renderer: &mut Renderer| {
            if self.offset == Vector::ZERO && self.scale == 1.0 {
                draw(renderer);
            } else {
                let moved = self.transformation(bounds);
                iced::advanced::Renderer::with_transformation(renderer, moved, draw);
            }
        };
        match self.clip_area(bounds) {
            Some(area) => iced::advanced::Renderer::with_layer(renderer, area, placed),
            None => placed(renderer),
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

impl<'a, Message: 'a> From<Reveal<'a, Message>> for Element<'a, Message> {
    fn from(widget: Reveal<'a, Message>) -> Self {
        Element::new(widget)
    }
}

/// The track and knob of a toggle (lot UI-01): switched, the knob slides on a spring and the
/// track fades from one color to the other; hovered, the track lightens; pressed, the knob
/// stretches. The first state shows at once. Clicks and the keyboard belong to the button
/// around it (`component::toggle`).
pub struct Switch {
    on: bool,
    enabled: bool,
    /// A state forced by the gallery, instead of the pointer's.
    forced: Option<iced::widget::button::Status>,
}

/// The switch of a toggle that is `on`.
pub fn switch(on: bool, enabled: bool) -> Switch {
    Switch {
        on,
        enabled,
        forced: None,
    }
}

impl Switch {
    /// Shows `status` whatever the pointer does. For the gallery.
    pub fn forced(mut self, status: Option<iced::widget::button::Status>) -> Self {
        self.forced = status;
        self
    }
}

/// Where the knob is, how lit the track is, and the pointer on the switch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchState {
    on: bool,
    /// From 0 (left, off) to 1 (right, on), on `motion::TOGGLE_SPRING`.
    knob: Animation,
    /// From 0 (off colors) to 1 (on colors), over `motion::FADE`.
    lit: Level,
    hover: Level,
    press: Level,
}

impl SwitchState {
    /// At rest in `on`.
    pub fn new(on: bool) -> Self {
        let at = f64::from(u8::from(on));
        Self {
            on,
            knob: Animation::new(motion::TOGGLE_SPRING, at, at, Duration::ZERO),
            lit: Level::new(on),
            hover: Level::default(),
            press: Level::default(),
        }
    }

    /// Switched to `on` at `now`: the knob leaves from where it is, with its velocity.
    pub fn switch(&mut self, on: bool, now: Duration) {
        if on == self.on {
            return;
        }
        self.on = on;
        self.knob.retarget(now, f64::from(u8::from(on)));
        self.lit.set(on, now, motion::FADE);
    }

    /// The knob's place, from 0 to 1 (a little beyond on the overshoot).
    pub fn knob(&self, now: Duration) -> f32 {
        self.knob.value(now) as f32
    }

    /// How much of the on colors the track takes, from 0 to 1.
    pub fn lit(&self, now: Duration) -> f32 {
        self.lit.value(now)
    }

    /// Something still moves: frames are needed.
    pub fn running(&mut self, now: Duration) -> bool {
        let moving = !self.knob.is_done(now, f64::from(motion::KNOB_PRECISION));
        let lit = self.lit.tick(now);
        let hover = self.hover.tick(now);
        let press = self.press.tick(now);
        moving || lit || hover || press
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Switch {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<SwitchState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(SwitchState::new(self.on))
    }

    fn diff(&self, tree: &mut Tree) {
        tree.state
            .downcast_mut::<SwitchState>()
            .switch(self.on, clock());
    }

    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed(metrics::TOGGLE_WIDTH),
            Length::Fixed(metrics::TOGGLE_HEIGHT),
        )
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = Size::new(metrics::TOGGLE_WIDTH, metrics::TOGGLE_HEIGHT);
        layout::Node::new(limits.resolve(
            Length::Fixed(size.width),
            Length::Fixed(size.height),
            size,
        ))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<SwitchState>();
        let now = clock();
        let over = self.enabled && cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
        let mut moved = false;
        if let Event::Mouse(event) = event {
            moved |= state.hover.set(over, now, motion::FADE);
            match event {
                mouse::Event::ButtonPressed(mouse::Button::Left) if over => {
                    moved |= state.press.set(true, now, motion::PRESS_FADE);
                }
                mouse::Event::ButtonReleased(mouse::Button::Left) => {
                    moved |= state.press.set(false, now, motion::PRESS_FADE);
                }
                mouse::Event::CursorLeft => {
                    moved |= state.press.set(false, now, motion::PRESS_FADE);
                }
                _ => {}
            }
        }
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            moved |= state.running(now);
        }
        if moved {
            shell.request_redraw();
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use iced::widget::button::Status;
        let state = tree.state.downcast_ref::<SwitchState>();
        let now = clock();
        let p = crate::theme::palette(theme);
        let (hover, press) = match self.forced {
            Some(Status::Hovered) => (1.0, 0.0),
            Some(Status::Pressed) => (1.0, 1.0),
            Some(_) => (0.0, 0.0),
            None => (state.hover.value(now), state.press.value(now)),
        };
        let enabled = self.enabled && self.forced != Some(Status::Disabled);
        let lit = state.lit(now);
        let (track, knob) = if enabled {
            let off = p.toggle_off.lerp(p.toggle_off_hover, hover);
            let on = p
                .accent
                .lerp(p.accent_hover, hover)
                .lerp(p.accent_pressed, press);
            (off.lerp(on, lit), p.text_secondary.lerp(p.deepest, lit))
        } else {
            (p.disabled, p.text_tertiary)
        };
        let bounds = layout.bounds();
        let radius = bounds.height / 2.0;
        veil(renderer, bounds, radius, track);
        let inset = metrics::TOGGLE_INSET;
        let side = bounds.height - 2.0 * inset;
        let stretch = metrics::TOGGLE_KNOB_STRETCH * press;
        let travel = bounds.width - 2.0 * inset - side - stretch;
        let knob_box = Rectangle {
            x: bounds.x + inset + travel * state.knob(now),
            y: bounds.y + inset,
            width: side + stretch,
            height: side,
        };
        veil(renderer, knob_box, side / 2.0, knob);
    }
}

impl<'a, Message: 'a> From<Switch> for Element<'a, Message> {
    fn from(widget: Switch) -> Self {
        Element::new(widget)
    }
}

/// A line icon that cross-fades when it changes (UI-01): a Wi-Fi gaining an arc, a sound
/// muted. Given several documents, it shows them in turn every `motion::PULSE`, each change
/// faded: the arcs of a Wi-Fi connecting. It asks for a frame only while it fades, and for the
/// next step of a cycle at its time.
pub struct GlyphFade {
    frames: Vec<String>,
    size: f32,
    tint: Color,
}

/// `frames` (one, or a cycle) at `size`, in `tint`.
pub fn glyph_fade(frames: Vec<String>, size: f32, tint: Color) -> GlyphFade {
    GlyphFade { frames, size, tint }
}

impl GlyphFade {
    /// The document shown at `now`.
    fn frame(&self, now: Duration) -> &str {
        let count = self.frames.len().max(1) as u128;
        let step = now.as_millis() / motion::PULSE.as_millis().max(1) % count;
        self.frames
            .get(usize::try_from(step).unwrap_or_default())
            .map_or("", String::as_str)
    }
}

/// The document shown, the one it replaced, and the fade between them.
#[derive(Debug, Default)]
struct GlyphState {
    shown: String,
    previous: Option<String>,
    fade: Level,
}

impl GlyphState {
    /// Shows `document` from `now`, fading from the one shown. Returns whether it changed.
    fn show(&mut self, document: &str, now: Duration) -> bool {
        if self.shown == document {
            return false;
        }
        if self.shown.is_empty() {
            self.shown = document.to_owned();
            return false;
        }
        self.previous = Some(std::mem::replace(&mut self.shown, document.to_owned()));
        self.fade = Level::new(false);
        self.fade.set(true, now, motion::GLYPH_FADE);
        true
    }

    /// Share of the new document, from 0 to 1.
    fn share(&self, now: Duration) -> f32 {
        if self.previous.is_some() {
            self.fade.value(now)
        } else {
            1.0
        }
    }

    /// Forgets a finished fade. Returns whether one still runs.
    fn tick(&mut self, now: Duration) -> bool {
        if self.fade.tick(now) {
            return true;
        }
        self.previous = None;
        false
    }
}

fn glyph_handle(document: &str) -> iced::advanced::svg::Handle {
    iced::advanced::svg::Handle::from_memory(document.as_bytes().to_vec())
}

impl<Message> Widget<Message, Theme, Renderer> for GlyphFade {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<GlyphState>()
    }

    fn state(&self) -> tree::State {
        let mut state = GlyphState::default();
        state.show(self.frame(clock()), Duration::ZERO);
        tree::State::new(state)
    }

    fn diff(&self, tree: &mut Tree) {
        tree.state
            .downcast_mut::<GlyphState>()
            .show(self.frame(clock()), clock());
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(self.size), Length::Fixed(self.size))
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let side = Length::Fixed(self.size);
        layout::Node::new(limits.resolve(side, side, Size::new(self.size, self.size)))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _viewport: &Rectangle,
    ) {
        let Event::Window(window::Event::RedrawRequested(_)) = event else {
            return;
        };
        let state = tree.state.downcast_mut::<GlyphState>();
        let now = clock();
        let changed = state.show(self.frame(now), now);
        if state.tick(now) || changed {
            shell.request_redraw();
        } else if self.frames.len() > 1 {
            // The next step of the cycle, at its time: no frame in between.
            let pulse = motion::PULSE.as_millis().max(1);
            let wait = pulse - now.as_millis() % pulse;
            let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(1));
            shell.request_redraw_at(window::RedrawRequest::At(Instant::now() + wait));
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        use iced::advanced::svg::{Renderer as _, Svg};
        let state = tree.state.downcast_ref::<GlyphState>();
        let share = state.share(clock());
        let bounds = layout.bounds();
        let tint: iced::Color = self.tint.into();
        let mut paint = |document: &str, opacity: f32| {
            if opacity > 0.0 && !document.is_empty() {
                let svg = Svg {
                    handle: glyph_handle(document),
                    color: Some(tint),
                    rotation: iced::Radians(0.0),
                    opacity,
                };
                renderer.draw_svg(svg, bounds, bounds);
            }
        };
        if let Some(previous) = &state.previous {
            paint(previous, 1.0 - share);
        }
        paint(&state.shown, share);
    }
}

impl<'a, Message: 'a> From<GlyphFade> for Element<'a, Message> {
    fn from(widget: GlyphFade) -> Self {
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
    fn a_reveal_scales_around_its_origin_and_moves_by_its_offset() {
        let bounds = Rectangle::new(Point::new(100.0, 10.0), Size::new(200.0, 50.0));
        let panel = reveal::<()>(iced::widget::space())
            .offset(0.0, -8.0)
            .scale(0.5, (1.0, 0.0));
        let moved = panel.transformation(bounds);
        // The top right corner stays where it is, but for the offset.
        let corner = Point::new(300.0, 10.0) * moved;
        assert!((corner.x - 300.0).abs() < 1e-4 && (corner.y - 2.0).abs() < 1e-4);
        let left = Point::new(100.0, 10.0) * moved;
        assert!((left.x - 200.0).abs() < 1e-4, "{left:?}");
        let rest = reveal::<()>(iced::widget::space()).transformation(bounds);
        assert_eq!(Point::new(120.0, 30.0) * rest, Point::new(120.0, 30.0));
    }

    #[test]
    fn a_switch_slides_its_knob_and_fades_its_track_then_rests() {
        let mut switch = SwitchState::new(false);
        assert_eq!(
            (switch.knob(Duration::ZERO), switch.lit(Duration::ZERO)),
            (0.0, 0.0)
        );
        assert!(!switch.running(Duration::ZERO), "no frame at rest");
        switch.switch(true, 100 * MS);
        assert!(switch.running(100 * MS));
        let (knob, lit) = (switch.knob(140 * MS), switch.lit(140 * MS));
        assert!(knob > 0.2 && knob < 1.0, "{knob}");
        assert!(lit > 0.2 && lit < 1.0, "{lit}");
        // Switched back halfway: the knob turns back from where it is, without a jump.
        switch.switch(false, 140 * MS);
        assert!((switch.knob(140 * MS) - knob).abs() < 1e-4);
        assert!((switch.lit(140 * MS) - lit).abs() < 1e-4);
        assert!(switch.running(200 * MS));
        assert!(!switch.running(Duration::from_secs(2)));
        assert!(switch.knob(Duration::from_secs(2)).abs() < 0.01);
        assert_eq!(switch.lit(Duration::from_secs(2)), 0.0);
    }

    #[test]
    fn the_knob_overshoots_slightly_and_settles_quickly() {
        let mut switch = SwitchState::new(false);
        switch.switch(true, Duration::ZERO);
        let path: Vec<f32> = (0..600).map(|t| switch.knob(t * MS)).collect();
        let peak = path.iter().copied().fold(0.0, f32::max);
        assert!(peak > 1.0 && peak < 1.04, "{peak}");
        let settled = path
            .iter()
            .rposition(|v| (v - 1.0).abs() > 0.01)
            .unwrap_or(0);
        assert!(settled < 300, "settled at {settled} ms");
    }

    #[test]
    fn a_press_deepens_the_veil_and_a_release_lightens_it() {
        let mut state = HoverState::default();
        state.set(true, Duration::ZERO);
        assert!(state.press.set(true, 200 * MS, motion::PRESS_FADE));
        assert!(state.tick(210 * MS));
        assert_eq!(state.press.value(200 * MS + motion::PRESS_FADE), 1.0);
        // Leaving with the button held releases the veil too.
        assert!(state.set(false, 400 * MS));
        assert!(!state.press.on());
        assert!(!state.tick(Duration::from_secs(1)));
    }

    #[test]
    fn an_open_veil_fades_but_its_first_state_shows_at_once() {
        let mut state = HoverState::default();
        state.open(true, Duration::ZERO);
        assert_eq!(state.open.map(|l| l.value(Duration::ZERO)), Some(1.0));
        assert!(!state.tick(Duration::ZERO));
        state.open(false, 100 * MS);
        assert!(state.tick(120 * MS));
        let half = state.open.map_or(0.0, |l| l.value(120 * MS));
        assert!(half > 0.0 && half < 1.0, "{half}");
        assert!(!state.tick(100 * MS + motion::FADE));
    }

    #[test]
    fn a_moving_page_is_cut_to_its_panel_and_a_mask_to_its_height() {
        let bounds = Rectangle::new(Point::new(15.0, 15.0), Size::new(306.0, 400.0));
        let page = || reveal::<()>(iced::widget::space()).clip_margin(14.0);
        // At rest nothing is cut: no layer to draw.
        assert_eq!(page().clip_area(bounds), None);
        let moving = page().offset(24.0, 0.0).clip_area(bounds);
        assert_eq!(
            moving,
            Some(Rectangle::new(
                Point::new(1.0, 1.0),
                Size::new(334.0, 428.0)
            ))
        );
        // A mask cuts below only: a panel that slides sideways keeps its edges, its corners and
        // its shadow, wherever its surface holds them.
        let mask = reveal::<()>(iced::widget::space())
            .offset(12.0, 0.0)
            .clip_height(120.0);
        let Some(area) = mask.clip_area(bounds) else {
            panic!("a mask cuts");
        };
        assert_eq!(area.y + area.height, bounds.y + 120.0);
        let surface = Rectangle::new(Point::ORIGIN, Size::new(418.0, 773.0));
        assert!(area.x <= surface.x && area.y <= surface.y);
        assert!(area.x + area.width >= surface.x + surface.width);
        let beyond = reveal::<()>(iced::widget::space()).clip_height(900.0);
        assert_eq!(
            beyond.clip_area(bounds).map(|a| a.y + a.height),
            Some(915.0)
        );
    }

    #[test]
    fn a_glyph_cross_fades_to_its_new_level_then_rests() {
        let mut state = GlyphState::default();
        assert!(
            !state.show("a", Duration::ZERO),
            "the first one shows at once"
        );
        assert_eq!(state.share(Duration::ZERO), 1.0);
        assert!(!state.tick(Duration::ZERO));
        assert!(state.show("b", 100 * MS));
        let half = state.share(140 * MS);
        assert!(half > 0.0 && half < 1.0, "{half}");
        assert!(state.tick(140 * MS));
        assert!(!state.tick(100 * MS + motion::GLYPH_FADE));
        assert!(state.previous.is_none());
        assert_eq!(state.share(Duration::from_secs(1)), 1.0);
    }

    #[test]
    fn a_cycle_shows_its_frames_in_turn() {
        let cycle = glyph_fade(
            vec!["a".into(), "b".into(), "c".into()],
            16.0,
            Color::rgb(0, 0, 0),
        );
        assert_eq!(cycle.frame(Duration::ZERO), "a");
        assert_eq!(cycle.frame(motion::PULSE), "b");
        assert_eq!(cycle.frame(motion::PULSE * 3), "a");
        let still = glyph_fade(vec!["a".into()], 16.0, Color::rgb(0, 0, 0));
        assert_eq!(still.frame(motion::PULSE * 7), "a");
    }

    #[test]
    fn another_row_in_the_same_place_starts_from_rest() {
        let mut state = HoverState::default();
        state.rekey(Some(7));
        state.set(true, Duration::ZERO);
        state.rekey(Some(7));
        assert!(state.hover.on());
        state.rekey(Some(8));
        assert!(!state.hover.on());
        assert_eq!(state.value(Duration::ZERO), 0.0);
    }
}
