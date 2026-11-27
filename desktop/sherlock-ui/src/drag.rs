//! Tokens of drag and drop between applications (lot E-03): the picture under the pointer when
//! files are dragged out of Evidence, the folder that lights up under a drop, and the folder
//! that opens when the pointer rests on it (the spring-loaded folders of the Finder).

use std::time::Duration;

use crate::color::{self, Color};

/// How far the pointer goes from a press on the selection, in logical pixels, before it becomes
/// a drag: a click with a slight slip stays a click.
pub const THRESHOLD: f32 = 6.0;

/// Side of the card of each file in the picture, its corner radius, and the side of the glyph
/// on it, in logical pixels.
pub const CARD: f32 = 56.0;
pub const CARD_RADIUS: f32 = 12.0;
pub const GLYPH: f32 = 30.0;
/// Width of the card's border.
pub const CARD_BORDER: f32 = 1.0;
/// At most this many cards are stacked, each shifted by `FAN` up and right and turned by `TURN`
/// degrees from the one in front.
pub const STACK: usize = 3;
pub const FAN: f32 = 5.0;
pub const TURN: f32 = 4.0;
/// The count of files, past one, in a pill at the top right: its height, and the height of its
/// digits.
pub const BADGE: f32 = 20.0;
pub const DIGIT: f32 = 10.0;
/// Room around the stack, so that the turned cards and the badge are not cut.
pub const MARGIN: f32 = 12.0;
/// Where the pointer is on the picture, from its top left corner: on the front card, a little
/// in, as if it held it.
pub const HOTSPOT: (f32, f32) = (MARGIN + 20.0, MARGIN + 20.0 + FAN * (STACK as f32 - 1.0));
/// Pixels of the picture per logical pixel: sharp at scale 2, downscaled below.
pub const SCALE: i32 = 2;
/// The largest picture file decoded for the thumbnail of a photo, in bytes.
pub const THUMBNAIL_MAX_FILE: u64 = 24 << 20;

/// A refused drag: its picture glides back for this long (the compositor's `DRAG_RETURN`), and
/// lives a little longer.
pub const RETURN: Duration = Duration::from_millis(350);

/// A folder under a drag opens after the pointer has rested on it this long.
pub const SPRING_DELAY: Duration = Duration::from_millis(800);
/// It blinks this many times before it opens, as the Finder's.
pub const SPRING_BLINKS: u32 = 2;

/// The card of a file in the picture.
pub fn card() -> Color {
    color::palette().card
}

/// Its border.
pub fn card_border() -> Color {
    color::palette().card_border
}

/// The count of files: white on the accent, as a notification badge.
pub fn badge() -> Color {
    color::palette().accent
}

pub fn badge_text() -> Color {
    color::palette().text_on_accent
}

/// The folder, or the listing, that a drop would land in.
pub fn target_fill() -> Color {
    color::palette().accent_soft
}

pub fn target_border() -> Color {
    color::palette().accent_border
}

const _: () = {
    assert!(GLYPH < CARD);
    assert!(DIGIT < BADGE);
    assert!(SCALE >= 1);
};

/// Radius of the drop target's outline: a row's, or the listing's.
pub const TARGET_RADIUS: f32 = 8.0;
/// Width of that outline.
pub const TARGET_BORDER: f32 = 2.0;

#[cfg(feature = "iced")]
pub use style::target;

#[cfg(feature = "iced")]
mod style {
    use iced::widget::container;
    use iced::{Background, Theme, border};

    /// The drop target, lit at `light` (0 to 1, it blinks before a folder opens).
    pub fn target(light: f32) -> impl Fn(&Theme) -> container::Style {
        move |_| {
            let fill = super::target_fill();
            let line = super::target_border();
            container::Style {
                background: Some(Background::Color(
                    iced::Color::from(fill).scale_alpha(light),
                )),
                border: border::width(super::TARGET_BORDER)
                    .color(iced::Color::from(line).scale_alpha(light))
                    .rounded(super::TARGET_RADIUS),
                ..container::Style::default()
            }
        }
    }
}
