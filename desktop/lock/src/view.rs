//! Pieces of the screen shared with the login screen (`desktop/greeter`): an element that rises
//! into place, the spinner of the pill, and the fade of a color.

use std::time::Duration;

use iced::widget::{container, svg};
use iced::{Color as IcedColor, Element, Padding};
use sherlock_ui::color::{self, Color};
use sherlock_ui::icon::Icon;
use sherlock_ui::lock::screen as ui;
use sherlock_ui::{metrics, motion};

/// An element that rises into place as `presence` goes from 0 to 1, in a box of constant
/// height so that nothing around it moves.
pub fn rise<'a, Message: 'a>(
    element: impl Into<Element<'a, Message>>,
    presence: f32,
) -> Element<'a, Message> {
    let offset = ui::RISE * (1.0 - presence.clamp(0.0, 1.0));
    container(element)
        .padding(Padding::default().top(offset).bottom(ui::RISE - offset))
        .into()
}

/// The spinner, `elapsed` after it started: the arc turned by one of `SPIN_STEPS` steps, drawn
/// into the SVG itself (one document per step, which iced keeps), since the rotation of the
/// SVG widget draws nothing with tiny-skia.
pub fn spinner<'a, Message: 'a>(elapsed: Duration, tint: Color) -> Element<'a, Message> {
    let steps = motion::SPIN_STEPS;
    let turn = elapsed.as_secs_f32() / ui::SPIN.as_secs_f32();
    let step = (turn.fract() * steps as f32) as u32 % steps;
    let angle = step as f32 * 360.0 / steps as f32;
    let center = metrics::ICON_GRID / 2.0;
    let document = Icon::Spinner.svg().replace(
        "<path ",
        &format!("<path transform=\"rotate({angle} {center} {center})\" "),
    );
    svg(svg::Handle::from_memory(document.into_bytes()))
        .width(ui::FIELD_ICON)
        .height(ui::FIELD_ICON)
        .style(move |_, _| svg::Style {
            color: Some(tint.into()),
        })
        // The renderers ignore the alpha of an SVG's color: the opacity carries it (UI-04).
        .opacity(color::icon_opacity(tint))
        .into()
}

/// A color of the palette, its opacity times `presence`.
pub fn fade(color: Color, presence: f32) -> IcedColor {
    let mut color: IcedColor = color.into();
    color.a *= presence;
    color
}

pub fn color_of(color: IcedColor) -> Color {
    Color {
        r: (color.r * 255.0).round() as u8,
        g: (color.g * 255.0).round() as u8,
        b: (color.b * 255.0).round() as u8,
        a: color.a,
    }
}
