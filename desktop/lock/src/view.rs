//! Pieces of the screen shared with the login screen (`desktop/greeter`): Sherlock in his
//! bubble and the name under it (the same avatar on both screens, lot D-24d), the pictures
//! behind, an element that rises into place, the spinner of the pill, and the fade of a color.

use std::time::Duration;

use iced::widget::{Space, container, image, stack, svg, text};
use iced::{
    Background, Border, Color as IcedColor, ContentFit, Element, Font, Length, Padding, Point,
};
use sherlock_3d::{Mood, Sherlock3d};
use sherlock_ui::color::{self, Color};
use sherlock_ui::icon::Icon;
use sherlock_ui::lock::screen::{self as ui, Colors};
use sherlock_ui::{metrics, motion, typography};

use crate::app::{Mascot, SHERLOCK};

/// What Sherlock does in his bubble.
#[derive(Debug, Clone, Copy)]
pub struct Avatar<'a> {
    pub mascot: Option<&'a Mascot>,
    pub mood: Mood,
    /// Where he looks, from the top left corner of the bubble's box; `None`: ahead.
    pub gaze: Option<Point>,
    /// Stops the 3D animation (the form away).
    pub paused: bool,
}

/// Sherlock in his bubble, in a box of `diameter`: in 3D on the GPU, the still picture in
/// software or without the model. The bubble grows from `BUBBLE_FROM` of its size as
/// `progress` goes from 0 to 1 (unclamped: the spring overshoots), and shrinks back as it
/// leaves. The round bubble of the theme follows the fade of the theme with the palette.
pub fn bubble<'a, Message: 'a>(
    avatar: Avatar<'_>,
    diameter: f32,
    progress: f32,
    colors: &Colors,
) -> Element<'a, Message> {
    let presence = progress.clamp(0.0, 1.0);
    let grow = ui::BUBBLE_FROM + (1.0 - ui::BUBBLE_FROM) * progress.max(0.0);
    let side = (diameter * grow).max(1.0);
    let fill = fade(colors.bubble, presence);
    let edge = fade(colors.bubble_border, presence);
    let mascot: Element<'a, Message> = match avatar.mascot {
        Some(Mascot(model)) => Sherlock3d::new(Some(model.clone()))
            .mood(avatar.mood)
            .look_at(avatar.gaze)
            .background(fill)
            .paused(avatar.paused)
            .width(side)
            .height(side)
            .into(),
        // The still picture of the boards, its top in the bubble.
        None => container(
            image(image::Handle::from_bytes(SHERLOCK))
                .height(side * 1.15)
                .opacity(presence),
        )
        .padding(Padding::default().top(side * 0.1))
        .width(side)
        .align_x(iced::Alignment::Center)
        .into(),
    };
    let bubble = container(mascot)
        .width(side)
        .height(side)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(Background::Color(fill)),
            border: Border::default().rounded(side / 2.0).width(1.0).color(edge),
            ..container::Style::default()
        });
    container(bubble).center(diameter).into()
}

/// The name under the bubble.
pub fn name<'a, Message: 'a>(
    shown: impl text::IntoFragment<'a>,
    tint: IcedColor,
) -> Element<'a, Message> {
    text(shown)
        .size(ui::NAME_SIZE)
        .font(Font {
            weight: iced::font::Weight::Semibold,
            ..typography::UI
        })
        .color(tint)
        .into()
}

/// Where the password field lies from the top left corner of the bubble's box of `diameter`:
/// under the bubble, the name, `between` (what the login screen puts under the name) and their
/// gaps.
pub fn field_point(diameter: f32, between: f32) -> Point {
    Point::new(
        diameter / 2.0,
        diameter
            + ui::BUBBLE_GAP
            + ui::NAME_SIZE
            + between
            + ui::NAME_GAP
            + ui::FIELD_HEIGHT / 2.0
            + 2.0 * ui::RISE,
    )
}

/// The pictures behind the screen, dark then light over it as opaque as the light palette is
/// mixed in, all of them `opacity` opaque; nothing until they are read. No plain background
/// under them: the compositor shows the same picture behind (lot D-24d), so the screen never
/// jumps from a plain color to the picture.
pub fn backdrop<'a, Message: 'a>(
    pictures: Option<&(image::Handle, image::Handle)>,
    opacity: f32,
) -> Element<'a, Message> {
    let mut layers = stack![Space::new().width(Length::Fill).height(Length::Fill)];
    if let Some((dark, light)) = pictures {
        let light_share = f32::from(color::mix_step()) / f32::from(color::MIX_STEPS);
        let cover = |handle: &image::Handle, share: f32| {
            image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Cover)
                .opacity(share * opacity)
        };
        if light_share < 1.0 {
            layers = layers.push(cover(dark, 1.0));
        }
        if light_share > 0.0 {
            layers = layers.push(cover(light, light_share));
        }
    }
    layers.width(Length::Fill).height(Length::Fill).into()
}

/// How opaque the pictures are `elapsed` after they were read: they fade in over
/// `PICTURE_FADE`.
pub fn picture_opacity(elapsed: Duration) -> f32 {
    motion::Ease::OutCubic.apply(elapsed.as_secs_f32() / ui::PICTURE_FADE.as_secs_f32())
}

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
