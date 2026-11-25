//! The desktop background of the Main board (colour and three discs) on the background layer,
//! for screenshots of the bar in a compositor without a wallpaper (a nested KWin). Test tool,
//! not shipped: `cargo run -p bar --example backdrop`.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::Tree;
use iced::advanced::{Widget, mouse};
use iced::{Background, Element, Length, Rectangle, Renderer, Size, Theme, border};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::to_layer_message;
use sherlock_ui::{color, metrics};

#[to_layer_message]
#[derive(Debug, Clone)]
enum Message {}

struct Desktop;

impl<M> Widget<M, Theme, Renderer> for Desktop {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _: &Tree,
        renderer: &mut Renderer,
        _: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let bounds = layout.bounds();
        let fill = |renderer: &mut Renderer, bounds, radius, tint: color::Color| {
            let quad = Quad {
                bounds,
                border: border::rounded(radius),
                ..Quad::default()
            };
            renderer.fill_quad(quad, Background::Color(tint.into()));
        };
        fill(renderer, bounds, 0.0, color::DESKTOP);
        for (&(x, y, size), tint) in metrics::DESKTOP_DISC_GEOMETRY
            .iter()
            .zip(color::DESKTOP_DISCS)
        {
            let disc = Rectangle::new(iced::Point::new(x, y), Size::new(size, size));
            fill(renderer, disc, size / 2.0, tint);
        }
    }
}

fn namespace() -> String {
    String::from("backdrop")
}

fn update(_: &mut (), _: Message) {}

fn view(_: &()) -> Element<'_, Message> {
    Element::new(Desktop)
}

fn main() -> Result<(), iced_exwlshell::Error> {
    // Like the bar: never on the NVIDIA card (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    iced_exwlshell::disable_clipboard();
    iced_exwlshell::layershell::application(|| (), namespace, update, view)
        .wl_settings(ExWlSettings {
            layer_settings: LayerShellSettings {
                anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
                layer: Layer::Background,
                exclusive_zone: -1,
                keyboard_interactivity: KeyboardInteractivity::None,
                start_mode: StartMode::Active,
                ..LayerShellSettings::default()
            },
            ..ExWlSettings::default()
        })
        .run()
}
