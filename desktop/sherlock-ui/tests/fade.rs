//! Icons fade and move with what is around them (UI-04): a panel unfolding showed its texts
//! and its backgrounds half faded and its icons at full strength, iced's renderers ignoring the
//! alpha of an SVG's color.

use iced::advanced::clipboard;
use iced::advanced::renderer::Headless;
use iced::widget::{container, row};
use iced::{Element, Size, mouse};
use iced_runtime::user_interface::{Cache, UserInterface};
use sherlock_ui::animated::reveal;
use sherlock_ui::color::{self, Color};
use sherlock_ui::component as ui;
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, Glyph};
use sherlock_ui::{theme, typography};

const SIZE: Size = Size::new(200.0, 40.0);
/// The screenshot's scale.
const SCALE: f32 = 1.0;
const WHITE: Color = Color::rgb(255, 255, 255);

/// A white square, a white icon and a white glyph of the quick settings, side by side on
/// black, `opacity` applied as a panel does: built inside `with_opacity`, drawn by a `Reveal`
/// moved by `offset`.
fn view<'a>(opacity: f32, offset: f32) -> Element<'a, ()> {
    color::with_opacity(opacity, || {
        let square =
            container(iced::widget::space().width(24.0).height(24.0)).style(|_| container::Style {
                background: Some(iced::Background::Color(WHITE.into())),
                ..container::Style::default()
            });
        let line = row![
            square,
            ui::icon(Icon::Check, 24.0, WHITE),
            panels::glyph(Glyph::Sun, WHITE)
        ]
        .spacing(16.0);
        reveal(line).opacity(opacity).offset(offset, 0.0).into()
    })
}

/// The pixels of `view`, RGBA, at `SCALE`.
fn render(opacity: f32, offset: f32) -> Vec<u8> {
    let Some(mut renderer) = iced::futures::executor::block_on(iced::Renderer::new(
        typography::UI,
        iced::Pixels(typography::UI_SIZE),
        Some("tiny-skia"),
    )) else {
        panic!("tiny-skia starts without a screen");
    };
    let mut ui = UserInterface::build(view(opacity, offset), SIZE, Cache::default(), &mut renderer);
    let mut messages = Vec::new();
    let _ = ui.update(
        &[],
        mouse::Cursor::Unavailable,
        &mut renderer,
        &mut clipboard::Null,
        &mut messages,
    );
    let theme = theme::theme();
    ui.draw(
        &mut renderer,
        &theme,
        &iced::advanced::renderer::Style {
            text_color: iced::Color::WHITE,
        },
        mouse::Cursor::Unavailable,
    );
    let size = Size::new(SIZE.width as u32, SIZE.height as u32);
    renderer.screenshot(size, SCALE, iced::Color::BLACK)
}

/// The brightest red of the columns `from..to`, and the first column lit in them.
fn brightest(pixels: &[u8], from: usize, to: usize) -> (u8, Option<usize>) {
    let width = SIZE.width as usize;
    let mut max = 0;
    let mut first: Option<usize> = None;
    for (i, pixel) in pixels.chunks(4).enumerate() {
        let x = i % width;
        if (from..to).contains(&x) {
            max = max.max(pixel[0]);
            if pixel[0] > 16 {
                first = Some(first.map_or(x, |f| f.min(x)));
            }
        }
    }
    (max, first)
}

#[test]
fn icons_fade_with_the_square_next_to_them() {
    let pixels = render(0.5, 0.0);
    let (square, _) = brightest(&pixels, 0, 24);
    let (icon, _) = brightest(&pixels, 40, 64);
    let (glyph, _) = brightest(&pixels, 80, 98);
    // The square is half faded: 255 × 0.5 over black.
    assert!((120..=136).contains(&square), "square at {square}");
    for (name, value) in [("icon", icon), ("glyph", glyph)] {
        assert!(value > 40, "{name} drawn: {value}");
        assert!(
            value <= square + 8,
            "{name} at {value}, brighter than the square at {square}"
        );
    }
    // At full opacity, the same icons are at full strength.
    let full = render(1.0, 0.0);
    assert!(brightest(&full, 40, 64).0 > 200);
    assert!(brightest(&full, 80, 98).0 > 200);
}

#[test]
fn icons_move_with_the_square_next_to_them() {
    let still = render(1.0, 0.0);
    let moved = render(1.0, 10.0);
    let spans = [(0, 34), (34, 74), (74, 120)];
    for (from, to) in spans {
        let (_, before) = brightest(&still, from, to);
        let (_, after) = brightest(&moved, from, to + 10);
        let (Some(before), Some(after)) = (before, after) else {
            panic!("nothing drawn in {from}..{to}");
        };
        assert_eq!(after, before + 10, "{from}..{to}");
    }
}
