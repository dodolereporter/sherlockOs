//! The moving parts of the controls UI-01 adds to the design system: the box of a check box
//! and the ring of a radio button, which fill over a fade; the thumb of a segmented control,
//! which slides to the choice on a spring; the fill of a progress bar, which follows its value
//! on a spring; and the spinning arc of a loading indicator. Each asks for frames only while it
//! moves, but the spinner, which moves as long as it is shown.

use std::time::Duration;

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::svg::{Handle, Renderer as _, Svg};
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Shell, Widget, mouse};
use iced::{
    Background, Element, Event, Length, Radians, Rectangle, Renderer, Size, Theme, border, window,
};

use crate::animated::{Level, clock, veil};
use crate::color::Color;
use crate::icon::Icon;
use crate::motion::{self, Animation};
use crate::{metrics, theme};

/// What moves.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// A check box: its box fills green and its check appears.
    Check { on: bool, enabled: bool },
    /// A radio button: its ring fills green and its dot grows.
    Radio { on: bool, enabled: bool },
    /// The thumb of a segmented control under its choice, among `count`.
    Thumb { index: Option<usize>, count: usize },
    /// The fill of a progress bar, from 0 to 1.
    Progress { value: f32 },
    /// A loading indicator.
    Spinner { size: f32 },
}

impl Kind {
    /// The value a spring drives towards, for the kinds that have one.
    fn target(self) -> f64 {
        match self {
            Self::Thumb {
                index: Some(index), ..
            } => index as f64,
            Self::Progress { value } => f64::from(value.clamp(0.0, 1.0)),
            _ => 0.0,
        }
    }

    /// Whether it is on: checked, chosen, a thumb shown.
    fn on(self) -> bool {
        match self {
            Self::Check { on, .. } | Self::Radio { on, .. } => on,
            Self::Thumb { index, .. } => index.is_some(),
            Self::Progress { .. } | Self::Spinner { .. } => true,
        }
    }
}

/// A moving part of a control.
pub struct Motion {
    kind: Kind,
}

/// The box of a check box.
pub fn check(on: bool, enabled: bool) -> Motion {
    Motion {
        kind: Kind::Check { on, enabled },
    }
}

/// The ring of a radio button.
pub fn radio(on: bool, enabled: bool) -> Motion {
    Motion {
        kind: Kind::Radio { on, enabled },
    }
}

/// The thumb of a segmented control of `count` choices, under `index`. It fills its parent.
pub fn thumb(index: Option<usize>, count: usize) -> Motion {
    Motion {
        kind: Kind::Thumb { index, count },
    }
}

/// The fill of a progress bar, `value` from 0 to 1. It is as wide as its parent.
pub fn progress(value: f32) -> Motion {
    Motion {
        kind: Kind::Progress { value },
    }
}

/// A loading indicator of `size`.
pub fn spinner(size: f32) -> Motion {
    Motion {
        kind: Kind::Spinner { size },
    }
}

/// How on it is, over a fade, and where its spring is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionState {
    lit: Level,
    spring: Animation,
}

impl MotionState {
    /// At rest where `kind` is: the first state shows at once.
    fn new(kind: Kind) -> Self {
        let at = kind.target();
        Self {
            lit: Level::new(kind.on()),
            spring: Animation::new(motion::TOGGLE_SPRING, at, at, Duration::ZERO),
        }
    }

    /// `kind` from `now`: what changed leaves from where it is.
    fn sync(&mut self, kind: Kind, now: Duration) {
        self.lit.set(kind.on(), now, motion::FADE);
        let target = kind.target();
        if kind.on() && (self.spring.target() - target).abs() > f64::EPSILON {
            // A thumb that appears starts under its choice, without sliding from elsewhere.
            if matches!(kind, Kind::Thumb { .. }) && self.lit.value(now) == 0.0 {
                self.spring = Animation::new(motion::TOGGLE_SPRING, target, target, now);
            } else {
                self.spring.retarget(now, target);
            }
        }
    }

    /// Something still moves.
    fn running(&mut self, now: Duration) -> bool {
        let fading = self.lit.tick(now);
        fading || !self.spring.is_done(now, f64::from(motion::KNOB_PRECISION))
    }
}

/// The spinner's arc: three quarters of a ring.
const ARC: &str = "M12 4a8 8 0 1 1-8 8";

fn icon_svg(path: &str, stroke: f32) -> Handle {
    Handle::from_memory(
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" \
             stroke=\"#000\" stroke-width=\"{stroke}\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"><path d=\"{path}\"/></svg>"
        )
        .into_bytes(),
    )
}

/// Draws an SVG `handle` in `tint`, at `opacity`, turned by `angle`.
fn paint(
    renderer: &mut Renderer,
    handle: Handle,
    tint: Color,
    opacity: f32,
    area: Rectangle,
    angle: f32,
) {
    if opacity > 0.0 {
        let svg = Svg {
            handle,
            color: Some(tint.into()),
            rotation: Radians(angle),
            opacity,
        };
        renderer.draw_svg(svg, area, area);
    }
}

/// The angle of the spinner at `now`: one turn every `motion::SPIN`.
pub fn spin_angle(now: Duration) -> f32 {
    let turn = motion::SPIN.as_secs_f32();
    std::f32::consts::TAU * (now.as_secs_f32() % turn) / turn
}

impl<Message> Widget<Message, Theme, Renderer> for Motion {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<MotionState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(MotionState::new(self.kind))
    }

    fn diff(&self, tree: &mut Tree) {
        tree.state
            .downcast_mut::<MotionState>()
            .sync(self.kind, clock());
    }

    fn size(&self) -> Size<Length> {
        match self.kind {
            Kind::Check { .. } | Kind::Radio { .. } => Size::new(
                Length::Fixed(metrics::CHECKBOX),
                Length::Fixed(metrics::CHECKBOX),
            ),
            Kind::Thumb { .. } => {
                Size::new(Length::Fill, Length::Fixed(crate::panels::SEGMENT_HEIGHT))
            }
            Kind::Progress { .. } => {
                Size::new(Length::Fill, Length::Fixed(metrics::PROGRESS_HEIGHT))
            }
            Kind::Spinner { size } => Size::new(Length::Fixed(size), Length::Fixed(size)),
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let size = Widget::<Message, Theme, Renderer>::size(self);
        layout::Node::new(limits.resolve(size.width, size.height, Size::ZERO))
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
        if let Event::Window(window::Event::RedrawRequested(_)) = event {
            let state = tree.state.downcast_mut::<MotionState>();
            if state.running(clock()) || matches!(self.kind, Kind::Spinner { .. }) {
                shell.request_redraw();
            }
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
        let state = tree.state.downcast_ref::<MotionState>();
        let now = clock();
        let p = theme::palette(theme);
        let bounds = layout.bounds();
        let lit = state.lit.value(now);
        let at = state.spring.value(now) as f32;
        match self.kind {
            Kind::Check { enabled, .. } | Kind::Radio { enabled, .. } => {
                let round = matches!(self.kind, Kind::Radio { .. });
                let radius = if round {
                    bounds.width / 2.0
                } else {
                    metrics::CHECKBOX_RADIUS
                };
                let (edge, fill, ink) = if enabled {
                    (p.secondary_border, p.accent, p.text_on_accent)
                } else {
                    (p.divider, p.disabled, p.text_tertiary)
                };
                // The empty box, its edge only, then the fill over it.
                let ring = Quad {
                    bounds,
                    border: border::rounded(radius)
                        .width(metrics::CHECKBOX_EDGE)
                        .color(iced::Color::from(edge)),
                    ..Quad::default()
                };
                iced::advanced::Renderer::fill_quad(
                    renderer,
                    ring,
                    Background::Color(iced::Color::TRANSPARENT),
                );
                veil(
                    renderer,
                    bounds,
                    radius,
                    Color {
                        a: fill.a * lit,
                        ..fill
                    },
                );
                if round {
                    let dot = metrics::RADIO_DOT * lit;
                    let centre = bounds.center();
                    let area = Rectangle {
                        x: centre.x - dot / 2.0,
                        y: centre.y - dot / 2.0,
                        width: dot,
                        height: dot,
                    };
                    veil(renderer, area, dot / 2.0, ink);
                } else {
                    let handle = icon_svg(Icon::Check.path(), metrics::ICON_STROKE);
                    paint(renderer, handle, ink, lit, bounds.shrink(2.0), 0.0);
                }
            }
            Kind::Thumb { count, .. } => {
                let count = count.max(1) as f32;
                let gap = crate::panels::SEGMENT_GAP;
                let width = (bounds.width - gap * (count - 1.0)) / count;
                let area = Rectangle {
                    x: bounds.x + at * (width + gap),
                    width,
                    ..bounds
                };
                let cream = Color {
                    a: p.cream.a * lit,
                    ..p.cream
                };
                veil(renderer, area, crate::panels::SEGMENT_RADIUS, cream);
            }
            Kind::Progress { .. } => {
                let radius = bounds.height / 2.0;
                veil(renderer, bounds, radius, p.pressed);
                let filled = Rectangle {
                    width: (bounds.width * at.clamp(0.0, 1.0)).max(bounds.height),
                    ..bounds
                };
                if at > 0.0 {
                    veil(renderer, filled, radius, p.accent);
                }
            }
            Kind::Spinner { .. } => {
                // Turned inside the document rather than by the renderer, whose software
                // version turns around the wrong point; in steps, so that each is drawn once.
                let steps = motion::SPIN_STEPS as f32;
                let step = (spin_angle(now) / std::f32::consts::TAU * steps).floor();
                let degrees = step * 360.0 / steps;
                let path = format!("{ARC}\" transform=\"rotate({degrees} 12 12)");
                let handle = icon_svg(&path, metrics::SPINNER_STROKE);
                paint(renderer, handle, p.accent_text, 1.0, bounds, 0.0);
            }
        }
    }
}

impl<'a, Message: 'a> From<Motion> for Element<'a, Message> {
    fn from(widget: Motion) -> Self {
        Element::new(widget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn a_check_box_fills_then_rests() {
        let off = Kind::Check {
            on: false,
            enabled: true,
        };
        let mut state = MotionState::new(off);
        assert_eq!(state.lit.value(Duration::ZERO), 0.0);
        assert!(!state.running(Duration::ZERO), "no frame at rest");
        state.sync(
            Kind::Check {
                on: true,
                enabled: true,
            },
            100 * MS,
        );
        let half = state.lit.value(150 * MS);
        assert!(half > 0.0 && half < 1.0, "{half}");
        assert!(state.running(150 * MS));
        assert!(!state.running(Duration::from_secs(2)));
        assert_eq!(state.lit.value(Duration::from_secs(2)), 1.0);
    }

    #[test]
    fn a_thumb_slides_to_its_choice_and_appears_in_place() {
        let mut state = MotionState::new(Kind::Thumb {
            index: Some(0),
            count: 3,
        });
        state.sync(
            Kind::Thumb {
                index: Some(2),
                count: 3,
            },
            Duration::ZERO,
        );
        let between = state.spring.value(60 * MS);
        assert!(between > 0.2 && between < 2.0, "{between}");
        assert!(!state.running(Duration::from_secs(2)));
        assert!((state.spring.value(Duration::from_secs(2)) - 2.0).abs() < 0.01);
        // From no choice, the thumb fades in under its choice without sliding.
        let mut state = MotionState::new(Kind::Thumb {
            index: None,
            count: 3,
        });
        state.sync(
            Kind::Thumb {
                index: Some(1),
                count: 3,
            },
            Duration::ZERO,
        );
        assert_eq!(state.spring.value(10 * MS), 1.0);
    }

    #[test]
    fn a_progress_bar_follows_its_value() {
        let mut state = MotionState::new(Kind::Progress { value: 0.2 });
        state.sync(Kind::Progress { value: 0.8 }, Duration::ZERO);
        let between = state.spring.value(50 * MS);
        assert!(between > 0.2 && between < 0.9, "{between}");
        assert!(!state.running(Duration::from_secs(2)));
        // Beyond its range, it is kept in it.
        assert_eq!(Kind::Progress { value: 3.0 }.target(), 1.0);
    }

    #[test]
    fn the_spinner_turns_once_per_period() {
        assert_eq!(spin_angle(Duration::ZERO), 0.0);
        let quarter = spin_angle(motion::SPIN / 4);
        assert!(
            (quarter - std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "{quarter}"
        );
        let turn = spin_angle(motion::SPIN);
        assert!(turn < 1e-3 || std::f32::consts::TAU - turn < 1e-3, "{turn}");
    }
}
