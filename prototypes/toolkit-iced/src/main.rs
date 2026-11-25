//! Prototype jetable du lot D-00 : la barre du haut en iced, ancrée par layer-shell.
//!
//! Même cahier des charges que `essais/toolkit-slint` : barre de 36 px, horloge, trois
//! bureaux dont un actif, et un bouton « Fenêtre » qui ouvre un menu de cinq entrées
//! dont une a un sous-menu. Le menu et le sous-menu sont de vrais `xdg_popup` imbriqués,
//! rattachés à la surface layer-shell de la barre.

mod theme;

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use iced::widget::{button, column, container, mouse_area, row, space, stack, text};
use iced::window::Id;
use iced::{
    Alignment, Animation, Background, Border, Color, Element, Font, Length, Padding, Subscription,
    Task, Theme, animation, font, time, window,
};
use iced_layershell::actions::IcedNewPopupSettings;
use iced_layershell::build_pattern::daemon;
use iced_layershell::reexport::{Anchor, KeyboardInteractivity, Layer, PopupAnchor, PopupGravity};
use iced_layershell::settings::{LayerShellSettings, Settings, StartMode};
use iced_layershell::to_layer_message;

use theme as t;

fn main() -> Result<(), iced_layershell::Error> {
    // La barre n'a aucun champ de saisie : inutile de lancer le fil du presse-papiers.
    iced_layershell::disable_clipboard();

    daemon(Bar::new, Bar::namespace, Bar::update, Bar::view)
        .style(|_, _| iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: t::TEXT,
        })
        .subscription(Bar::subscription)
        .settings(Settings {
            default_font: Font::with_name(t::FONT_FAMILY),
            default_text_size: t::FONT_SIZE.into(),
            layer_settings: LayerShellSettings {
                size: Some((0, t::BAR_HEIGHT)),
                exclusive_zone: t::BAR_HEIGHT as i32,
                anchor: Anchor::Top | Anchor::Left | Anchor::Right,
                layer: Layer::Top,
                // haut, droite, bas, gauche
                margin: (t::BAR_MARGIN_TOP, t::BAR_MARGIN_SIDE, 0, t::BAR_MARGIN_SIDE),
                keyboard_interactivity: KeyboardInteractivity::None,
                start_mode: StartMode::Active,
                ..Default::default()
            },
            ..Default::default()
        })
        .run()
}

/// L'identifiant de la surface de la barre est choisi par iced_layershell : on le retient au
/// premier affichage pour y rattacher le menu.
static BAR_ID: OnceLock<Id> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Popup {
    Menu,
    SubMenu,
}

/// Mode automatique, qui sert aux mesures (variable `ESSAI_AUTO`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Auto {
    Off,
    Running { remaining: u32, step: u32 },
}

struct Bar {
    clock: String,
    date: String,
    active_desktop: usize,
    menu: Option<Id>,
    submenu: Option<Id>,
    menu_anim: Animation<bool>,
    now: Instant,
    auto: Auto,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
enum Message {
    Tick,
    Frame(Instant),
    SelectDesktop(usize),
    ToggleMenu,
    OpenSubMenu,
    CloseSubMenu,
    MenuItem,
    SubMenuItem(usize),
    PopupClosed(Id),
    AutoStep,
}

impl Bar {
    fn new() -> (Self, Task<Message>) {
        let (clock, date) = t::now_strings();
        let auto = match std::env::var_os("ESSAI_AUTO") {
            Some(_) => Auto::Running {
                remaining: t::AUTO_CYCLES,
                step: 0,
            },
            None => Auto::Off,
        };
        let bar = Self {
            clock,
            date,
            active_desktop: 0,
            menu: None,
            submenu: None,
            menu_anim: Animation::new(false)
                .duration(Duration::from_millis(t::MENU_ANIM_MS))
                .easing(animation::Easing::EaseOutCubic),
            now: Instant::now(),
            auto,
        };
        (bar, Task::none())
    }

    fn namespace() -> String {
        String::from("essai-barre-iced")
    }

    fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            window::close_events().map(Message::PopupClosed),
        ];
        // On ne demande des images que pendant l'animation : au repos, rien ne se redessine.
        if self.menu.is_some() && self.menu_anim.is_animating(self.now) {
            subs.push(window::frames().map(Message::Frame));
        }
        if self.auto != Auto::Off {
            subs.push(
                time::every(Duration::from_millis(t::AUTO_STEP_MS)).map(|_| Message::AutoStep),
            );
        }
        Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                (self.clock, self.date) = t::now_strings();
                Task::none()
            }
            Message::Frame(now) => {
                self.now = now;
                Task::none()
            }
            Message::SelectDesktop(index) => {
                self.active_desktop = index;
                Task::none()
            }
            Message::ToggleMenu => match self.menu {
                Some(_) => self.close_menus(),
                None => self.open_menu(),
            },
            Message::OpenSubMenu => self.open_submenu(),
            Message::CloseSubMenu => match self.submenu.take() {
                Some(id) => window::close(id),
                None => Task::none(),
            },
            Message::MenuItem => self.close_menus(),
            Message::SubMenuItem(index) => {
                self.active_desktop = index;
                self.close_menus()
            }
            Message::PopupClosed(id) => {
                // Le compositeur ferme le menu quand on clique ailleurs (popup_done).
                if self.menu == Some(id) {
                    self.menu = None;
                    if let Some(sub) = self.submenu.take() {
                        return window::close(sub);
                    }
                }
                if self.submenu == Some(id) {
                    self.submenu = None;
                }
                Task::none()
            }
            Message::AutoStep => self.auto_step(),
            _ => Task::none(),
        }
    }

    fn open_menu(&mut self) -> Task<Message> {
        let Some(bar) = BAR_ID.get().copied() else {
            return Task::none();
        };
        let id = Id::unique();
        self.menu = Some(id);
        self.now = Instant::now();
        self.menu_anim = Animation::new(false)
            .duration(Duration::from_millis(t::MENU_ANIM_MS))
            .easing(animation::Easing::EaseOutCubic)
            .go(true, self.now);
        let settings = IcedNewPopupSettings::new(bar, t::menu_size(), t::menu_anchor_rect())
            .anchor(PopupAnchor::BottomLeft)
            .gravity(PopupGravity::BottomRight);
        Task::done(Message::NewPopUp { settings, id })
    }

    fn open_submenu(&mut self) -> Task<Message> {
        let (Some(menu), None) = (self.menu, self.submenu) else {
            return Task::none();
        };
        let id = Id::unique();
        self.submenu = Some(id);
        let settings = IcedNewPopupSettings::new(menu, t::submenu_size(), t::submenu_anchor_rect())
            .anchor(PopupAnchor::TopRight)
            .gravity(PopupGravity::BottomRight);
        Task::done(Message::NewPopUp { settings, id })
    }

    fn close_menus(&mut self) -> Task<Message> {
        let mut tasks = Vec::new();
        // Le sous-menu se ferme avant son parent, comme l'exige xdg_popup.
        if let Some(id) = self.submenu.take() {
            tasks.push(window::close(id));
        }
        if let Some(id) = self.menu.take() {
            tasks.push(window::close(id));
        }
        Task::batch(tasks)
    }

    fn popup_kind(&self, id: Id) -> Option<Popup> {
        if self.menu == Some(id) {
            Some(Popup::Menu)
        } else if self.submenu == Some(id) {
            Some(Popup::SubMenu)
        } else {
            None
        }
    }

    /// Mode automatique, par pas de 250 ms : 4 s de repos, puis des cycles de 2 s où le menu
    /// s'ouvre, le sous-menu s'ouvre 750 ms plus tard, et tout se ferme à 1,25 s.
    fn auto_step(&mut self) -> Task<Message> {
        let Auto::Running { remaining, step } = self.auto else {
            return Task::none();
        };
        let step = step + 1;
        self.auto = Auto::Running { remaining, step };
        if step < t::AUTO_IDLE_STEPS {
            return Task::none();
        }
        match (step - t::AUTO_IDLE_STEPS) % 8 {
            0 if remaining == 0 => iced::exit(),
            0 => {
                self.auto = Auto::Running {
                    remaining: remaining - 1,
                    step,
                };
                self.open_menu()
            }
            3 => self.open_submenu(),
            5 => self.close_menus(),
            _ => Task::none(),
        }
    }

    fn view(&self, id: Id) -> Element<'_, Message> {
        match self.popup_kind(id) {
            Some(Popup::Menu) => self.view_menu(),
            Some(Popup::SubMenu) => self.view_submenu(),
            None => {
                let _ = BAR_ID.set(id);
                self.view_bar()
            }
        }
    }

    fn view_bar(&self) -> Element<'_, Message> {
        // Le bouton Sherlock n'est qu'une pastille ici : la mascotte n'est pas l'objet de l'essai.
        let sherlock = container(space())
            .width(t::SHERLOCK_W)
            .height(t::SHERLOCK_H)
            .style(|_| container::Style {
                background: Some(Background::Color(t::CREAM)),
                border: Border::default().rounded(t::SHERLOCK_RADIUS),
                ..Default::default()
            });

        let app_name = container(text(t::APP_NAME).font(bold()))
            .width(t::APP_NAME_W)
            .padding(Padding::default().left(8));

        let menu_open = self.menu.is_some();
        let window_button = button(container(text(t::MENU_LABEL)).center(Length::Fill))
            .width(t::MENU_BUTTON_W)
            .height(t::MENU_BUTTON_H)
            .padding(0)
            .on_press(Message::ToggleMenu)
            .style(move |_, status| {
                let lit = menu_open || matches!(status, button::Status::Hovered);
                button::Style {
                    background: lit.then_some(Background::Color(t::MENU_BUTTON_OPEN)),
                    text_color: if lit { t::TEXT } else { t::TEXT_2 },
                    border: Border::default().rounded(t::MENU_BUTTON_RADIUS),
                    ..Default::default()
                }
            });

        let left = row![sherlock, app_name, window_button]
            .spacing(t::LEFT_GAP)
            .align_y(Alignment::Center);

        let desktops = row(t::DESKTOPS.iter().enumerate().map(|(i, label)| {
            let active = i == self.active_desktop;
            let font = if active {
                bold()
            } else {
                Font::with_name(t::FONT_FAMILY)
            };
            button(container(text(*label).font(font)).center_y(Length::Fill))
                .height(t::DESKTOP_H)
                .padding([0.0, t::DESKTOP_PAD_X])
                .on_press(Message::SelectDesktop(i))
                .style(move |_, status| {
                    let hovered = matches!(status, button::Status::Hovered);
                    let background = if active {
                        t::ACCENT
                    } else if hovered {
                        t::DESKTOP_HOVER
                    } else {
                        t::DESKTOP_IDLE
                    };
                    button::Style {
                        background: Some(Background::Color(background)),
                        text_color: if active { t::TEXT_ON_ACCENT } else { t::TEXT_2 },
                        border: Border::default().rounded(t::DESKTOP_RADIUS),
                        ..Default::default()
                    }
                })
                .into()
        }))
        .spacing(t::LEFT_GAP)
        .align_y(Alignment::Center);

        let right = row![
            text(&self.date).color(t::TEXT_2),
            text(&self.clock).font(bold()).color(t::TEXT),
        ]
        .spacing(t::RIGHT_GAP)
        .align_y(Alignment::Center);

        let sides = row![left, space::horizontal(), right]
            .align_y(Alignment::Center)
            .height(Length::Fill)
            .padding(
                Padding::default()
                    .left(t::BAR_PAD_LEFT)
                    .right(t::BAR_PAD_RIGHT),
            );

        // Les bureaux sont centrés sur la barre, indépendamment des côtés.
        let center = container(desktops).center(Length::Fill);

        container(stack![sides, center])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                background: Some(Background::Color(t::BAR_GLASS)),
                border: Border {
                    color: t::BAR_BORDER,
                    width: 1.0,
                    radius: t::BAR_RADIUS.into(),
                },
                ..Default::default()
            })
            .into()
    }

    fn view_menu(&self) -> Element<'_, Message> {
        // Progression de l'animation d'ouverture, de 0 à 1.
        let p: f32 = self.menu_anim.interpolate(0.0, 1.0, self.now);
        let mut entries: Vec<Element<'_, Message>> = Vec::new();
        for (i, item) in t::MENU_ITEMS.iter().enumerate() {
            if i == t::MENU_SEPARATOR_BEFORE {
                entries.push(separator(p));
            }
            let trailing: Element<'_, Message> = if item.submenu {
                text("›").color(fade(t::TEXT_2, p)).into()
            } else {
                text(item.shortcut)
                    .size(t::SHORTCUT_SIZE)
                    .color(fade(t::TEXT_3, p))
                    .into()
            };
            let entry = if item.submenu {
                let lit = self.submenu.is_some();
                let entry = menu_entry(item.label, trailing, p, lit, Message::OpenSubMenu);
                mouse_area(entry).on_enter(Message::OpenSubMenu)
            } else {
                let entry = menu_entry(item.label, trailing, p, false, Message::MenuItem);
                mouse_area(entry).on_enter(Message::CloseSubMenu)
            };
            entries.push(entry.into());
        }
        let panel = container(column(entries))
            .width(Length::Fill)
            .padding(t::MENU_PADDING)
            .style(move |_| menu_style(p));
        // Le menu glisse de quelques pixels vers le bas en apparaissant.
        let slide = t::MENU_SLIDE * (1.0 - p);
        column![space().height(t::MENU_GAP - slide), panel].into()
    }

    fn view_submenu(&self) -> Element<'_, Message> {
        let items = t::DESKTOPS.iter().enumerate().map(|(i, label)| {
            let trailing: Element<'_, Message> = if i == self.active_desktop {
                text("✓").color(t::ACCENT).into()
            } else {
                space().into()
            };
            menu_entry(label, trailing, 1.0, false, Message::SubMenuItem(i))
        });
        let panel = container(column(items))
            .width(Length::Fill)
            .padding(t::MENU_PADDING)
            .style(|_| menu_style(1.0));
        row![space().width(t::SUBMENU_GAP), panel].into()
    }
}

fn bold() -> Font {
    Font {
        weight: font::Weight::Semibold,
        ..Font::with_name(t::FONT_FAMILY)
    }
}

fn fade(color: Color, p: f32) -> Color {
    Color {
        a: color.a * p,
        ..color
    }
}

fn menu_style(p: f32) -> container::Style {
    container::Style {
        background: Some(Background::Color(fade(t::MENU_GLASS, p))),
        border: Border {
            color: fade(t::MENU_BORDER, p),
            width: 1.0,
            radius: t::MENU_RADIUS.into(),
        },
        ..Default::default()
    }
}

fn separator<'a>(p: f32) -> Element<'a, Message> {
    container(
        container(space())
            .width(Length::Fill)
            .height(1)
            .style(move |_| container::Style {
                background: Some(Background::Color(fade(t::SEPARATOR, p))),
                ..Default::default()
            }),
    )
    .padding([t::SEPARATOR_MARGIN_Y, t::SEPARATOR_MARGIN_X])
    .into()
}

fn menu_entry<'a>(
    label: &'a str,
    trailing: Element<'a, Message>,
    p: f32,
    highlighted: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let content = row![
        text(label).color(fade(t::TEXT, p)),
        space::horizontal(),
        trailing
    ]
    .align_y(Alignment::Center)
    .spacing(t::MENU_ITEM_GAP);
    button(container(content).center_y(Length::Fill))
        .width(Length::Fill)
        .height(t::MENU_ITEM_H)
        .padding([0.0, t::MENU_ITEM_PAD_X])
        .on_press(on_press)
        .style(move |_: &Theme, status| {
            let lit =
                highlighted || matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: lit.then_some(Background::Color(fade(t::SELECTION, p))),
                text_color: fade(t::TEXT, p),
                border: Border::default().rounded(t::MENU_ITEM_RADIUS),
                ..Default::default()
            }
        })
        .into()
}
