//! État de la barre, ses messages et ses deux vues : la barre et le menu Sherlock.

use iced::widget::{column, container, mouse_area, row, space};
use iced::window::Id;
use iced::{Alignment, Element, Length, Padding, Subscription, Task, keyboard, window};
use iced_exwlshell::actions::IcedNewPopupSettings;
use iced_exwlshell::reexport::{PixelSize, PopupAnchor, PopupGravity, Position};
use iced_exwlshell::to_layer_message;
use sherlock_ui::component::{self as ui};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::icon::Icon;
use sherlock_ui::{color, metrics, strings, typography};

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
    /// `Some(None)` : le poste n'a pas de batterie.
    pub battery: Option<Option<Battery>>,
    pub network: Option<Network>,
    pub sound: Option<Sound>,
    pub workspaces: Option<Workspaces>,
    pub user: String,
    /// Surface de la barre, à laquelle le menu se rattache.
    surface: Option<Id>,
    /// Surface du menu Sherlock quand il est ouvert.
    menu: Option<Id>,
    /// Entrée du menu surlignée au clavier ou à la souris.
    pub cursor: Option<usize>,
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
    Closed(Id),
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
        Subscription::batch([
            Subscription::run(|| watch(SystemClock::default(), "horloge")).map(Message::Clock),
            Subscription::run(|| watch(UPower::default(), "batterie")).map(Message::Battery),
            Subscription::run(|| watch(NetworkManager::default(), "réseau")).map(Message::Network),
            Subscription::run(|| watch(Pactl::default(), "son")).map(Message::Sound),
            Subscription::run(|| watch(Placeholder, "bureaux")).map(Message::Workspaces),
            window::close_events().map(Message::Closed),
            keyboard::listen().filter_map(on_key),
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
            Message::MenuKey(_) => {}
            Message::Hover(index) => self.cursor = Some(index),
            Message::Activate(action) => {
                // Rien n'existe encore derrière ces entrées : on le dit, on ne fait pas semblant.
                tracing::info!(?action, "entrée du menu Sherlock pas encore disponible");
                return self.close_menu();
            }
            Message::SelectWorkspace(index) => {
                tracing::info!(
                    index,
                    "changer de bureau attend le protocole ext-workspace du compositeur"
                );
            }
            Message::Focus(direction) if self.menu_open() => {
                let key = match direction {
                    Direction::Next => menu::Key::Next,
                    Direction::Previous => menu::Key::Previous,
                };
                self.cursor = menu::move_cursor(self.cursor, key);
            }
            Message::Focus(direction) => return focus::advance(direction),
            Message::Closed(id) => {
                // Le compositeur ferme le menu sur un clic ailleurs (popup_done).
                if self.menu == Some(id) {
                    self.menu = None;
                    self.cursor = None;
                }
            }
            _ => {}
        }
        Task::none()
    }

    fn toggle_menu(&mut self) -> Task<Message> {
        if self.menu_open() {
            return self.close_menu();
        }
        let Some(surface) = self.surface else {
            tracing::warn!("surface de la barre inconnue : le menu ne peut pas s'ouvrir");
            return Task::none();
        };
        let (width, height) = menu::surface_size();
        let (button_width, _) = metrics::SHERLOCK_BUTTON;
        let anchor = Position::new(metrics::TOP_BAR_PADDING.0 as i32, 0);
        let anchor_size = PixelSize::px(button_width as u32, metrics::TOP_BAR_HEIGHT as u32);
        let settings =
            IcedNewPopupSettings::new(surface, PixelSize::px(width, height), anchor, anchor_size)
                .anchor(PopupAnchor::BottomLeft)
                .gravity(PopupGravity::BottomRight);
        let id = Id::unique();
        self.menu = Some(id);
        self.cursor = None;
        Task::done(Message::NewPopUp { settings, id })
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
        let sherlock = ui::sherlock_button(self.menu_open(), Some(Message::ToggleMenu));
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
                ui::chip(label, i == w.active, Some(Message::SelectWorkspace(i))).into()
            })
        });
        let center = row(chips)
            .spacing(metrics::CHIP_GAP)
            .align_y(Alignment::Center);

        let right = row(self.indicators())
            .spacing(metrics::STATUS_GAP)
            .align_y(Alignment::Center);

        // Les deux côtés se partagent la place qui reste : les bureaux tombent au centre.
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
        container(line)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(sherlock_ui::theme::top_bar)
            .into()
    }

    /// Réseau, son, batterie, puis la date et l'heure. Un indicateur qu'on n'a pas encore lu
    /// ne s'affiche pas.
    fn indicators(&self) -> Vec<Element<'_, Message>> {
        let tint = color::TEXT_SECONDARY;
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
            let tint = if battery.is_low() { color::AMBER } else { tint };
            let value = strings::percent(battery.percent);
            items.push(ui::status(glyph, Some(value), tint));
        }
        if let Some(clock) = &self.clock {
            items.push(ui::body(clock.date()).color(tint).into());
            let time = ui::body(clock.time())
                .font(typography::UI_SEMIBOLD)
                .color(color::TEXT_PRIMARY);
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
        column![space().height(metrics::TOP_BAR_MENU_GAP), panel].into()
    }
}

/// Tab déplace le focus ; les autres touches utiles vont au menu.
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
    fn les_fleches_et_tab_parcourent_le_menu() {
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
    fn entree_active_l_entree_surlignee_et_ferme_le_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::Hover(4));
        let _ = bar.update(Message::MenuKey(menu::Key::Activate));
        assert!(!bar.menu_open());
        assert_eq!(bar.cursor, None);
    }

    #[test]
    fn echap_ferme_le_menu() {
        let (mut bar, _) = with_menu();
        let _ = bar.update(Message::MenuKey(menu::Key::Close));
        assert!(!bar.menu_open());
    }

    #[test]
    fn menu_ferme_les_touches_du_menu_ne_font_rien() {
        let mut bar = Bar::default();
        let _ = bar.update(Message::MenuKey(menu::Key::Next));
        assert_eq!(bar.cursor, None);
    }

    #[test]
    fn le_compositeur_peut_fermer_le_menu() {
        let (mut bar, id) = with_menu();
        let _ = bar.update(Message::Hover(2));
        let _ = bar.update(Message::Closed(Id::unique()));
        assert!(bar.menu_open());
        let _ = bar.update(Message::Closed(id));
        assert!(!bar.menu_open());
        assert_eq!(bar.cursor, None);
    }

    #[test]
    fn sans_surface_le_menu_ne_s_ouvre_pas() {
        let mut bar = Bar::default();
        let _ = bar.update(Message::ToggleMenu);
        assert!(!bar.menu_open());
        let _ = bar.update(Message::Surface(Id::unique()));
        let _ = bar.update(Message::ToggleMenu);
        assert!(bar.menu_open());
    }
}
