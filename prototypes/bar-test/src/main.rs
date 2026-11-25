//! Test bar for the compositor (lot D-12): a 36 px bar anchored at the top of every screen,
//! with its exclusive zone, and a button that opens a menu in an `xdg_popup` attached to the
//! bar. Not the real bar (lot D-20): colors and texts are written here on purpose.
//!
//! `cargo run --release` inside the compositor, for example:
//! `compositor --nested --outputs 2 prototypes/bar-test/target/release/bar-test`.
//! With `BAR_TEST_MENU=1`, the menu of the first bar opens by itself, for screenshots.

use iced::widget::{button, column, container, row, space, text};
use iced::window::Id;
use iced::{Alignment, Background, Border, Color, Element, Length, Task, window};
use iced_exwlshell::actions::IcedNewPopupSettings;
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, LayerSize, Margin, PixelSize, PopupAnchor,
    PopupGravity, Position,
};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use iced_exwlshell::to_layer_message;

const HEIGHT: u32 = 36;
const GLASS: Color = Color::from_rgba8(16, 26, 44, 0.82);
const MENU: Color = Color::from_rgba8(22, 34, 56, 0.96);
const EDGE: Color = Color::from_rgba8(255, 255, 255, 0.10);
const TEXT: Color = Color::from_rgb8(0xEE, 0xF2, 0xF7);

#[derive(Default)]
struct Bar {
    /// Bar surfaces, one per screen, in creation order.
    bars: Vec<Id>,
    menu: Option<Id>,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Surface(Id),
    Toggle(Id),
    Closed(Id),
}

impl Bar {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.bars.push(id);
                if self.bars.len() == 1 && std::env::var_os("BAR_TEST_MENU").is_some() {
                    return self.update(Message::Toggle(id));
                }
            }
            Message::Toggle(parent) => {
                if let Some(menu) = self.menu.take() {
                    return window::close(menu);
                }
                let settings = IcedNewPopupSettings::new(
                    parent,
                    PixelSize::px(220, 110),
                    Position::new(8, 0),
                    PixelSize::px(80, HEIGHT),
                )
                .anchor(PopupAnchor::BottomLeft)
                .gravity(PopupGravity::BottomRight);
                let id = Id::unique();
                self.menu = Some(id);
                return Task::done(Message::NewPopUp { settings, id });
            }
            // The compositor closes the menu on a click elsewhere (popup_done).
            Message::Closed(id) if self.menu == Some(id) => self.menu = None,
            _ => {}
        }
        Task::none()
    }

    fn view(&self, id: Id) -> Element<'_, Message> {
        if self.menu == Some(id) {
            let lines = column![text("Menu de la barre de test"), text("Clique ailleurs : il se ferme.")]
                .spacing(8);
            return container(lines)
                .padding(12)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| glass(MENU))
                .into();
        }
        let screen = self.bars.iter().position(|b| *b == id).map_or(0, |i| i + 1);
        let line = row![
            button(text("Menu")).on_press(Message::Toggle(id)),
            space::horizontal(),
            text(format!("Barre de test · écran {screen}")),
            space::horizontal(),
        ]
        .align_y(Alignment::Center)
        .padding([0, 8]);
        container(line)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| glass(GLASS))
            .into()
    }
}

fn glass(color: Color) -> container::Style {
    container::Style {
        background: Some(Background::Color(color)),
        border: Border { color: EDGE, width: 1.0, radius: 12.0.into() },
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

fn main() -> Result<(), iced_exwlshell::Error> {
    iced_exwlshell::disable_clipboard();
    iced_exwlshell::daemon(|| (Bar::default(), Task::none()), || "bar-test".to_owned(), Bar::update, Bar::view)
        .style(|_, _| iced::theme::Style { background_color: Color::TRANSPARENT, text_color: TEXT })
        .subscription(|_| window::close_events().map(Message::Closed))
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .wl_settings(ExWlSettings {
            layer_settings: LayerShellSettings {
                anchor: Anchor::Top | Anchor::Left | Anchor::Right,
                layer: Layer::Top,
                size: LayerSize::fill_width(HEIGHT),
                exclusive_zone: HEIGHT as i32,
                margin: Margin { top: 10, right: 12, bottom: 0, left: 12 },
                keyboard_interactivity: KeyboardInteractivity::OnDemand,
                start_mode: StartMode::AllScreens,
                ..LayerShellSettings::default()
            },
            ..ExWlSettings::default()
        })
        .run()
}
