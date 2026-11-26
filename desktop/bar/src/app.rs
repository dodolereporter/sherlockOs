//! Bar state, messages, and its two views: the bar and the Sherlock menu.

use std::path::Path;
use std::time::Instant;

use iced::futures::channel::mpsc::UnboundedReceiver;
use iced::widget::{container, mouse_area, opaque, row};
use iced::window::Id;
use iced::{Alignment, Element, Length, Padding, Subscription, Task, keyboard, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{PixelSize, PopupAnchor, PopupGravity, Position};
use iced_exwlshell::to_layer_message;
use sherlock_ui::color::{self, Theme, ThemeFade};
use sherlock_ui::component::{self as ui};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::icon::Icon;
use sherlock_ui::surface::{self, Elevation};
use sherlock_ui::{metrics, strings, typography};

use crate::menu::{self, Action};
use crate::source::battery::{Battery, Charge, UPower};
use crate::source::clock::{LocalTime, SystemClock};
use crate::source::network::{Network, NetworkManager};
use crate::source::sound::{Pactl, Sound};
use crate::source::watch;
use crate::source::workspaces::{Placeholder, Workspaces};
use crate::user;

#[derive(Debug, Default)]
pub struct Bar {
    pub clock: Option<LocalTime>,
    /// `Some(None)`: the machine has no battery.
    pub battery: Option<Option<Battery>>,
    pub network: Option<Network>,
    pub sound: Option<Sound>,
    pub workspaces: Option<Workspaces>,
    pub user: String,
    /// Surface of the bar, which the menu attaches to.
    surface: Option<Id>,
    /// Surface of the Sherlock menu while it is open.
    menu: Option<Id>,
    /// Room around the menu in its surface: the gap below the bar, and its shadow on the GPU.
    pub menu_margins: Padding,
    /// Menu entry highlighted by the keyboard or the pointer.
    pub cursor: Option<usize>,
    /// Bar button that has the keyboard focus, as an index in `targets()`. The bar keeps it in
    /// its own state, like the menu cursor: the ring is drawn from the same value that Enter
    /// activates, in the same update. (iced's focus operations are applied after the redraw
    /// under `iced_exwlshell`, which left the ring one Tab behind.)
    focus: Option<usize>,
    /// The theme of the appearance file, shown with a fade.
    pub theme: ThemeFade,
}

/// A bar button that Tab can reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Sherlock,
    Workspace(usize),
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Clock(LocalTime),
    Battery(Option<Battery>),
    Network(Network),
    Sound(Sound),
    Workspaces(Workspaces),
    ToggleMenu,
    MenuKey(menu::Key),
    Hover(usize),
    Activate(Action),
    SelectWorkspace(usize),
    Focus(Direction),
    /// A mouse button was pressed somewhere in the bar or its menu.
    Pointer,
    Closed(Id),
    /// The theme of the appearance file, at start then at each change.
    Theme(Theme),
    Frame(Instant),
}

impl Bar {
    pub fn new() -> (Self, Task<Message>) {
        let bar = Self {
            user: user::current(),
            ..Self::default()
        };
        (bar, Task::none())
    }

    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let appearance = match appearance_config::path() {
            Some(path) => Subscription::run_with(path, |path| appearance(path)).map(Message::Theme),
            None => Subscription::none(),
        };
        let frames = if self.theme.running() {
            window::frames().map(Message::Frame)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            appearance,
            frames,
            Subscription::run(|| watch(SystemClock::default(), "clock")).map(Message::Clock),
            Subscription::run(|| watch(UPower::default(), "battery")).map(Message::Battery),
            Subscription::run(|| watch(NetworkManager::default(), "network")).map(Message::Network),
            Subscription::run(|| watch(Pactl::default(), "sound")).map(Message::Sound),
            Subscription::run(|| watch(Placeholder, "workspaces")).map(Message::Workspaces),
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
            iced::event::listen_with(on_pointer),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => self.surface = Some(id),
            Message::Clock(time) => self.clock = Some(time),
            Message::Battery(battery) => self.battery = Some(battery),
            Message::Network(network) => self.network = Some(network),
            Message::Sound(sound) => self.sound = Some(sound),
            Message::Workspaces(workspaces) => self.workspaces = Some(workspaces),
            Message::ToggleMenu => return self.toggle_menu(),
            Message::MenuKey(key) if self.menu_open() => match key {
                menu::Key::Close => return self.close_menu(),
                menu::Key::Activate => {
                    if let Some(action) = self.cursor.and_then(|i| Action::ALL.get(i)) {
                        return self.update(Message::Activate(*action));
                    }
                }
                key => self.cursor = menu::move_cursor(self.cursor, key),
            },
            Message::MenuKey(menu::Key::Activate) => {
                if let Some(message) = self.activation() {
                    return self.update(message);
                }
            }
            Message::MenuKey(menu::Key::Close) => self.focus = None,
            Message::MenuKey(_) => {}
            Message::Hover(index) => self.cursor = Some(index),
            Message::Activate(action) => {
                // Nothing exists behind these entries yet: say so, never pretend.
                tracing::info!(?action, "Sherlock menu entry not available yet");
                return self.close_menu();
            }
            Message::SelectWorkspace(index) => {
                tracing::info!(
                    index,
                    "switching workspaces waits for the compositor's ext-workspace protocol"
                );
            }
            Message::Focus(direction) if self.menu_open() => {
                let key = match direction {
                    Direction::Next => menu::Key::Next,
                    Direction::Previous => menu::Key::Previous,
                };
                self.cursor = menu::move_cursor(self.cursor, key);
            }
            Message::Focus(direction) => {
                let count = self.targets().len();
                self.focus = match (direction, self.focus) {
                    _ if count == 0 => None,
                    (Direction::Next, Some(i)) if i + 1 < count => Some(i + 1),
                    (Direction::Next, _) => Some(0),
                    (Direction::Previous, Some(i)) if i > 0 && i <= count => Some(i - 1),
                    (Direction::Previous, _) => Some(count - 1),
                };
            }
            // Like `:focus-visible` in a browser: the mouse removes the keyboard ring.
            Message::Pointer => self.focus = None,
            Message::Theme(theme) => self.theme.set(theme, Instant::now()),
            Message::Frame(now) => {
                self.theme.tick(now);
            }
            Message::Closed(id) => {
                // The compositor closes the menu on a click elsewhere (popup_done).
                if self.menu == Some(id) {
                    self.menu = None;
                    self.cursor = None;
                }
            }
            _ => {}
        }
        Task::none()
    }

    /// The bar buttons, in Tab order: Sherlock, then the workspaces.
    pub fn targets(&self) -> Vec<Target> {
        let workspaces = self.workspaces.as_ref().map_or(0, |w| w.names.len());
        std::iter::once(Target::Sherlock)
            .chain((0..workspaces).map(Target::Workspace))
            .collect()
    }

    /// The bar button that has the keyboard focus: the one drawn with the ring.
    pub fn focused(&self) -> Option<Target> {
        self.focus.and_then(|i| self.targets().get(i).copied())
    }

    /// What Enter or Space does: activate the focused button.
    pub fn activation(&self) -> Option<Message> {
        self.focused().map(|target| match target {
            Target::Sherlock => Message::ToggleMenu,
            Target::Workspace(i) => Message::SelectWorkspace(i),
        })
    }

    fn toggle_menu(&mut self) -> Task<Message> {
        if self.menu_open() {
            return self.close_menu();
        }
        let Some(surface) = self.surface else {
            tracing::warn!("bar surface unknown: the menu cannot open");
            return Task::none();
        };
        // The bar has been drawn by then: its renderer is known.
        self.menu_margins = menu::margins(surface::gpu_drawn() == Some(true));
        let (width, height) = menu::surface_size(self.menu_margins);
        let (x, y, anchor_width, anchor_height) = menu::anchor_rect(self.menu_margins);
        let settings = IcedNewPopupSettings::new(
            surface,
            PixelSize::px(width, height),
            Position::new(x, y),
            PixelSize::px(anchor_width, anchor_height),
        )
        .anchor(PopupAnchor::Bottom)
        .gravity(PopupGravity::Bottom);
        let id = Id::unique();
        self.menu = Some(id);
        self.cursor = None;
        // Only the menu takes the pointer: the transparent room for the shadow lies over the
        // bar, whose buttons stay reachable through it.
        let (x, y, width, height) = menu::input_rect(self.menu_margins);
        let region = RegionActionCB::new(move |region| {
            if let Err(error) = region.add(x, y, width, height) {
                tracing::warn!(%error, "cannot limit the menu's input region");
            }
        });
        Task::done(Message::NewPopUp { settings, id }).chain(Task::done(Message::SetInputRegion {
            id,
            callback: region,
        }))
    }

    fn close_menu(&mut self) -> Task<Message> {
        self.cursor = None;
        match self.menu.take() {
            Some(id) => window::close(id),
            None => Task::none(),
        }
    }

    pub fn view(&self, id: Id) -> Element<'_, Message> {
        if self.menu == Some(id) {
            self.view_menu()
        } else {
            self.view_bar()
        }
    }

    pub fn view_bar(&self) -> Element<'_, Message> {
        let ring = |control: ui::Control<'static, Message>, target| {
            if self.focused() == Some(target) {
                control.focus_ring()
            } else {
                control
            }
        };
        let sherlock = ring(
            ui::sherlock_button(self.menu_open(), Some(Message::ToggleMenu)),
            Target::Sherlock,
        );
        let mut left = row![sherlock]
            .spacing(metrics::TOP_BAR_LEFT_GAP)
            .align_y(Alignment::Center);
        if let Some(app) = self.workspaces.as_ref().and_then(|w| w.app.as_deref()) {
            let (pad_left, pad_right) = metrics::APP_NAME_PADDING;
            let name = ui::body(app).font(typography::UI_SEMIBOLD);
            left = left.push(container(name).padding(Padding {
                left: pad_left,
                right: pad_right,
                ..Padding::ZERO
            }));
        }

        let chips = self.workspaces.iter().flat_map(|w| {
            w.names.iter().enumerate().map(move |(i, name)| {
                let label = strings::top_bar::workspace(i + 1, name.as_deref());
                let chip = ui::chip(label, i == w.active, Some(Message::SelectWorkspace(i)));
                ring(chip, Target::Workspace(i)).into()
            })
        });
        let center = row(chips)
            .spacing(metrics::CHIP_GAP)
            .align_y(Alignment::Center);

        let right = row(self.indicators())
            .spacing(metrics::STATUS_GAP)
            .align_y(Alignment::Center);

        // Both sides share the remaining space: the workspaces land in the middle.
        let (pad_left, pad_right) = metrics::TOP_BAR_PADDING;
        let line = row![
            container(left).width(Length::Fill),
            center,
            container(right)
                .width(Length::Fill)
                .align_right(Length::Fill),
        ]
        .align_y(Alignment::Center)
        .height(Length::Fill)
        .padding(Padding {
            left: pad_left + metrics::BORDER_WIDTH,
            right: pad_right + metrics::BORDER_WIDTH,
            ..Padding::ZERO
        });
        let bar = container(line)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(sherlock_ui::theme::top_bar);
        // Flat: no shadow, but drawing it tells the menu which renderer draws the bar.
        surface::shadowed(bar, metrics::TOP_BAR_RADIUS, Elevation::Flat).into()
    }

    /// Network, sound, battery, then date and time. An indicator not read yet is not shown.
    fn indicators(&self) -> Vec<Element<'_, Message>> {
        let p = color::palette();
        let tint = p.text_secondary;
        let mut items = Vec::new();
        if let Some(network) = &self.network {
            let glyph = match network {
                Network::Connected { .. } => Icon::Network,
                Network::Disconnected => Icon::NetworkOff,
            };
            items.push(ui::status(glyph, None, tint));
        }
        if let Some(sound) = &self.sound {
            let glyph = if sound.muted {
                Icon::SoundMuted
            } else {
                Icon::Sound
            };
            items.push(ui::status(glyph, None, tint));
        }
        if let Some(Some(battery)) = &self.battery {
            let glyph = match battery.charge {
                Charge::Charging => Icon::BatteryCharging,
                _ => Icon::Battery,
            };
            let tint = if battery.is_low() { p.amber_text } else { tint };
            let value = strings::percent(battery.percent);
            items.push(ui::status(glyph, Some(value), tint));
        }
        if let Some(clock) = &self.clock {
            items.push(ui::body(clock.date()).color(tint).into());
            let time = ui::body(clock.time())
                .font(typography::UI_SEMIBOLD)
                .color(p.text_primary);
            items.push(time.into());
        }
        items
    }

    pub fn view_menu(&self) -> Element<'_, Message> {
        let mut entries = vec![
            ui::menu_header(self.user.as_str(), strings::top_bar::SESSION_OPEN),
            ui::menu_separator(),
        ];
        for (i, action) in Action::ALL.into_iter().enumerate() {
            if action.starts_group() {
                entries.push(ui::menu_separator());
            }
            let mut item = ui::menu_item(
                action.label(),
                action.trailing(),
                Some(Message::Activate(action)),
            );
            if self.cursor == Some(i) {
                item = ui::open(item);
            }
            entries.push(mouse_area(item).on_enter(Message::Hover(i)).into());
        }
        let panel = ui::menu(entries, metrics::MENU_WIDTH).height(Length::Fill);
        let panel = surface::shadowed(panel, metrics::MENU_RADIUS, Elevation::Menu);
        // A click on the shadow, outside the menu, closes it like a click elsewhere, for a
        // compositor that would ignore the input region.
        let surface = container(opaque(panel)).padding(self.menu_margins);
        mouse_area(surface)
            .on_press(Message::MenuKey(menu::Key::Close))
            .into()
    }
}

/// The themes of the appearance file at `path`, live (`appearance_config::follow_themes`).
pub fn appearance(path: &Path) -> UnboundedReceiver<Theme> {
    let (sender, receiver) = iced::futures::channel::mpsc::unbounded();
    let follow = appearance_config::follow_themes(path.to_path_buf(), move |theme| {
        sender.unbounded_send(theme).is_ok()
    });
    if let Err(error) = follow {
        tracing::warn!(%error, "the bar will not follow the appearance file");
    }
    receiver
}

/// Any mouse button press, captured by a widget or not.
fn on_pointer(event: iced::Event, _: iced::event::Status, _: Id) -> Option<Message> {
    matches!(
        event,
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(_))
    )
    .then_some(Message::Pointer)
}

/// Tab moves the focus; Enter, Space and Escape go to the menu when it is open, to the focused
/// bar button otherwise.
fn on_key(event: keyboard::Event) -> Option<Message> {
    focus::tab(event.clone())
        .map(Message::Focus)
        .or_else(|| menu::key(event).map(Message::MenuKey))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_menu() -> (Bar, Id) {
        let id = Id::unique();
        let bar = Bar {
            menu: Some(id),
            ..Bar::default()
        };
        (bar, id)
    }

    #[test]
    fn arrows_and_tab_walk_the_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        assert_eq!(bar.cursor, Some(1));
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.cursor, Some(0));
        let _ = bar.update(Message::MenuKey(menu::Key::Last));
        assert_eq!(bar.cursor, Some(Action::ALL.len() - 1));
    }

    #[test]
    fn enter_activates_the_highlighted_entry_and_closes_the_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::Hover(4));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert!(!bar.menu_open());
        assert_eq!(bar.cursor, None);
    }

    #[test]
    fn escape_closes_the_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert!(!bar.menu_open());
    }

    #[test]
    fn menu_keys_do_nothing_while_the_menu_is_closed() {
        let mut bar = Bar::default();
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert_eq!(bar.cursor, None);
        assert!(!bar.menu_open());
    }

    #[test]
    fn the_compositor_can_close_the_menu() {
        let (mut bar, id) = with_menu();
        let _ = bar.update(Message::Hover(2));
        let _ = bar.update(Message::Closed(Id::unique()));
        assert!(bar.menu_open());
        let _ = bar.update(Message::Closed(id));
        assert!(!bar.menu_open());
        assert_eq!(bar.cursor, None);
    }

    fn with_workspaces() -> Bar {
        let mut bar = Bar::default();
        let workspaces = Workspaces::placeholder(Some("Code,Web,Jeu"), None);
        let _ = bar.update(Message::Workspaces(workspaces));
        bar
    }

    #[test]
    fn the_first_tab_already_shows_the_ring() {
        let mut bar = with_workspaces();
        assert_eq!(bar.focused(), None);
        let _ = bar.update(Message::Focus(Direction::Next));
        assert_eq!(bar.focused(), Some(Target::Sherlock));
    }

    #[test]
    fn the_ringed_button_is_the_one_enter_activates() {
        let mut bar = with_workspaces();
        let expected = [
            Target::Sherlock,
            Target::Workspace(0),
            Target::Workspace(1),
            Target::Workspace(2),
            Target::Sherlock,
        ];
        for target in expected {
            let _ = bar.update(Message::Focus(Direction::Next));
            // The view rings `focused()`; Enter activates `activation()`: they must agree.
            assert_eq!(bar.focused(), Some(target));
            let activated = bar.activation();
            match target {
                Target::Sherlock => assert!(matches!(activated, Some(Message::ToggleMenu))),
                Target::Workspace(i) => {
                    assert!(matches!(activated, Some(Message::SelectWorkspace(j)) if j == i));
                }
            }
        }
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.focused(), Some(Target::Workspace(2)));
    }

    #[test]
    fn enter_on_the_ringed_sherlock_button_opens_the_menu() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Surface(Id::unique()));
        let _ = bar.update(Message::Focus(Direction::Next));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert!(bar.menu_open());
    }

    #[test]
    fn a_click_clears_the_bar_focus() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Focus(Direction::Next));
        assert!(bar.focused().is_some());
        let _ = bar.update(Message::Pointer);
        assert_eq!(bar.focused(), None);
    }

    #[test]
    fn escape_clears_the_bar_focus() {
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Focus(Direction::Previous));
        assert_eq!(bar.focused(), Some(Target::Workspace(2)));
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert_eq!(bar.focused(), None);
    }

    /// The only test of this binary that changes the theme: the others draw nothing.
    #[test]
    fn a_new_theme_in_the_file_fades_the_bar_without_restarting() {
        use iced::futures::StreamExt;
        use sherlock_ui::color::Palette;

        let dir = std::env::temp_dir().join(format!("bar-theme-{}", std::process::id()));
        let file = dir.join("appearance.conf");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, "[Appearance]\nTheme=dark\n").unwrap();
        let mut themes = appearance(&file);
        let mut next = || iced::futures::executor::block_on(themes.next()).unwrap();
        let mut bar = with_workspaces();
        let _ = bar.update(Message::Theme(next()));
        assert_eq!(color::palette(), &Palette::DARK);

        std::fs::write(&file, "[Appearance]\nTheme=light\n").unwrap();
        let theme = next();
        assert_eq!(theme, Theme::Light);
        let start = Instant::now();
        let _ = bar.update(Message::Theme(theme));
        assert!(bar.theme.running(), "frames are asked for the fade");
        let _ = bar.update(Message::Frame(start + sherlock_ui::motion::THEME_FADE / 3));
        assert_ne!(color::palette(), &Palette::DARK);
        assert_ne!(color::palette(), &Palette::LIGHT);
        let _ = bar.update(Message::Frame(start + sherlock_ui::motion::THEME_FADE * 2));
        assert!(!bar.theme.running());
        assert_eq!(color::palette(), &Palette::LIGHT);
        color::set_active(Theme::Dark);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_surface_the_menu_does_not_open() {
        let mut bar = Bar::default();
        let _ = bar.update(Message::ToggleMenu);
        assert!(!bar.menu_open());
        let _ = bar.update(Message::Surface(Id::unique()));
        let _ = bar.update(Message::ToggleMenu);
        assert!(bar.menu_open());
    }
}
