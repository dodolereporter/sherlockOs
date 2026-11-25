//! Galerie des composants, pour les comparer aux planches : `cargo run -p sherlock-ui
//! --example galerie`. Chaque composant y paraît dans ses cinq états, sur le bureau puis sur
//! le verre, avec le menu Fenêtre de la planche Menus, des cartes de la planche Réglages et le
//! logo. Les textes sont ceux des planches.

use iced::widget::{Column, button, column, container, row, scrollable, space, svg};
use iced::{Alignment, Element, Length, Subscription, Task, keyboard};

use crate::component::{self as ui, Trailing};
use crate::focus::{self, Direction};
use crate::icon::Icon;
use crate::{color, logo, metrics, theme};

/// Largeur d'une case du tableau des états, et de la colonne des noms.
const CELL: f32 = 180.0;
const NAME: f32 = 150.0;
/// Tailles d'affichage du logo.
const LOGO_SIZES: [f32; 4] = [96.0, 64.0, 32.0, 16.0];

/// État de la galerie : ce que les composants vivants modifient.
#[derive(Debug, Clone, Default)]
pub struct Gallery {
    variable_rate: bool,
    quiet: bool,
    search: String,
    submenu: bool,
    last: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Action(&'static str),
    VariableRate(bool),
    Quiet(bool),
    Search(String),
    Submenu,
    Focus(Direction),
}

/// Les cinq états montrés pour chaque composant.
#[derive(Debug, Clone, Copy)]
enum Look {
    Status(button::Status),
    Focus,
}

const LOOKS: [(&str, Look); 5] = [
    ("Normal", Look::Status(button::Status::Active)),
    ("Survol", Look::Status(button::Status::Hovered)),
    ("Appui", Look::Status(button::Status::Pressed)),
    ("Focus", Look::Focus),
    ("Désactivé", Look::Status(button::Status::Disabled)),
];

impl Gallery {
    /// Les interrupteurs allumés, comme sur la planche Réglages.
    pub fn new() -> Self {
        Self {
            variable_rate: true,
            quiet: true,
            last: "aucune",
            ..Self::default()
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Action(name) => self.last = name,
            Message::VariableRate(on) => self.variable_rate = on,
            Message::Quiet(on) => self.quiet = on,
            Message::Search(value) => self.search = value,
            Message::Submenu => self.submenu = !self.submenu,
            Message::Focus(direction) => return focus::advance(direction),
        }
        Task::none()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        keyboard::listen()
            .filter_map(focus::tab)
            .map(Message::Focus)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let intro = column![
            ui::title("Galerie de sherlock-ui"),
            ui::description("Tab et Maj+Tab déplacent le focus. Entrée ou Espace active."),
            ui::body(format!("Dernière action : {}", self.last)),
        ]
        .spacing(metrics::GAP_TIGHT);
        let page = column![
            intro,
            ui::section_label("Sur le fond du bureau"),
            states(),
            ui::section_label("Sur un panneau de verre"),
            ui::glass_panel(states()),
            ui::section_label("Planches Menus et Réglages, composants vivants"),
            row![self.menus(), self.settings()].spacing(metrics::PAGE_PADDING.1),
            ui::section_label("Logo"),
            logos(),
        ]
        .spacing(metrics::GAP_SECTION)
        .padding([metrics::PAGE_PADDING.0, metrics::PAGE_PADDING.1]);
        container(scrollable(page))
            .style(theme::desktop)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Le menu Fenêtre de la planche Menus, et son sous-menu « Déplacer vers le bureau ».
    fn menus(&self) -> Element<'_, Message> {
        let action = |name| Some(Message::Action(name));
        let entry =
            |label, keys| ui::menu_item(label, Trailing::Shortcut(keys), action(label)).into();
        let mut desk = ui::menu_item(
            "Déplacer vers le bureau",
            Trailing::Submenu,
            Some(Message::Submenu),
        );
        if self.submenu {
            desk = ui::open(desk);
        }
        let window = ui::menu(
            [
                entry("Réduire", "Super+M"),
                entry("Agrandir", "Super+Haut"),
                entry("Moitié gauche", "Super+Gauche"),
                entry("Moitié droite", "Super+Droite"),
                ui::menu_separator(),
                desk.into(),
                ui::menu_item(
                    "Toujours au premier plan",
                    Trailing::None,
                    action("premier plan"),
                )
                .into(),
                ui::menu_separator(),
                entry("Fermer la fenêtre", "Super+Q"),
            ],
            metrics::MENU_WIDTH,
        );
        let sub: Element<'_, Message> = if self.submenu {
            // Le sous-menu s'aligne sur son entrée, sous quatre entrées et un séparateur.
            let (top, _) = metrics::MENU_SEPARATOR_MARGIN;
            let offset = 4.0 * metrics::MENU_ITEM_HEIGHT + 2.0 * top + metrics::BORDER_WIDTH;
            let menu = ui::menu(
                [
                    ui::menu_item("1 · Code", Trailing::Checked, action("bureau 1")).into(),
                    ui::menu_item("2 · Web", Trailing::None, action("bureau 2")).into(),
                    ui::menu_item("3 · Jeu", Trailing::None, action("bureau 3")).into(),
                    ui::menu_separator(),
                    ui::menu_item("Nouveau bureau", Trailing::None, action("nouveau bureau"))
                        .into(),
                ],
                metrics::SUBMENU_WIDTH,
            );
            column![space().height(offset), menu].into()
        } else {
            space().into()
        };
        row![window, sub]
            .spacing(metrics::BORDER_WIDTH * 4.0)
            .into()
    }

    /// Deux cartes de la planche Réglages.
    fn settings(&self) -> Element<'_, Message> {
        let line = |label, help, control: ui::Control<'static, Message>| {
            row![
                column![
                    ui::body(label).font(crate::typography::UI_MEDIUM),
                    ui::description(help)
                ],
                space::horizontal(),
                Element::from(control),
            ]
            .align_y(Alignment::Center)
            .spacing(metrics::GAP_ROW)
            .height(metrics::SETTING_ROW_HEIGHT)
        };
        let divider = container(space())
            .width(Length::Fill)
            .height(metrics::BORDER_WIDTH)
            .style(theme::divider);
        let toggles = ui::card(column![
            line(
                "Fréquence variable",
                "L'écran suit le rythme du jeu.",
                ui::toggle(self.variable_rate, Some(Message::VariableRate)),
            ),
            divider,
            line(
                "Couper les notifications",
                "Sherlock garde tout pour après.",
                ui::toggle(self.quiet, Some(Message::Quiet)),
            ),
        ]);
        let driver = ui::card(
            row![
                column![
                    ui::body("Pilote NVIDIA")
                        .font(crate::typography::UI_SEMIBOLD)
                        .size(crate::typography::CARD_TITLE_SIZE),
                    ui::description("Version [VERSION ÉPINGLÉE], validée sur cette machine."),
                ],
                space::horizontal(),
                ui::secondary_button(
                    "Essayer une nouvelle version",
                    Some(Message::Action("nouvelle version"))
                ),
            ]
            .align_y(Alignment::Center)
            .spacing(metrics::GAP_ROW),
        );
        let search = ui::text_field("Rechercher un réglage", &self.search, Some(Message::Search));
        let actions = row![
            ui::primary_button("Continuer", Some(Message::Action("continuer"))),
            ui::icon_button(
                Icon::Search,
                "Rechercher",
                Some(Message::Action("rechercher"))
            ),
            ui::icon_button(
                Icon::Network,
                "Réseau connecté",
                Some(Message::Action("réseau"))
            ),
            ui::icon_button(Icon::Sound, "Son activé", Some(Message::Action("son"))),
        ]
        .spacing(metrics::GAP_INLINE)
        .align_y(Alignment::Center);
        column![search, toggles, driver, actions]
            .spacing(metrics::GAP_SECTION)
            .width(Length::Fill)
            .into()
    }
}

/// Fabrique la case d'un composant dans un état donné.
type Cell<'a> = fn(Look) -> Element<'a, Message>;

/// Le tableau des états : une ligne par composant, une colonne par état.
fn states<'a>() -> Element<'a, Message> {
    let header = LOOKS
        .iter()
        .fold(row![space().width(NAME)], |r, (name, _)| {
            r.push(container(ui::section_label(name)).width(CELL))
        });
    let lines: [(&str, Cell<'a>); 6] = [
        ("Bouton principal", |look| {
            control(ui::primary_button("Relancer", enabled(look)), look)
        }),
        ("Bouton secondaire", |look| {
            control(ui::secondary_button("Voir les logs", enabled(look)), look)
        }),
        ("Icône seule", |look| {
            control(ui::icon_button(Icon::Close, "Fermer", enabled(look)), look)
        }),
        ("Interrupteur", |look| {
            control(
                ui::toggle(
                    true,
                    enabled(look).map(|_| Message::Quiet as fn(bool) -> Message),
                ),
                look,
            )
        }),
        ("Entrée de menu", |look| {
            // Une entrée de menu ne vit que dans un menu : elle s'y montre.
            let item = ui::menu_item("Réglages", Trailing::Shortcut("Super+,"), enabled(look));
            ui::menu([control(item, look)], CELL - 2.0 * metrics::GAP_INLINE).into()
        }),
        ("Champ de texte", field),
    ];
    let body = lines.iter().map(|(name, make)| {
        LOOKS
            .iter()
            .fold(
                row![container(ui::body(*name)).width(NAME)],
                |r, (_, look)| {
                    r.push(
                        container(make(*look))
                            .width(CELL)
                            .padding([0.0, metrics::GAP_INLINE]),
                    )
                },
            )
            .align_y(Alignment::Center)
            .height(metrics::BUTTON_HEIGHT + 2.0 * metrics::GAP_INLINE)
            .into()
    });
    Column::with_children(std::iter::once(header.into()).chain(body)).into()
}

fn enabled(look: Look) -> Option<Message> {
    match look {
        Look::Status(button::Status::Disabled) => None,
        _ => Some(Message::Action("galerie")),
    }
}

fn control<'a>(control: ui::Control<'a, Message>, look: Look) -> Element<'a, Message> {
    match look {
        Look::Status(status) => control.status(status).into(),
        Look::Focus => control.focus_ring().into(),
    }
}

/// Le champ de texte n'a ni survol appuyé ni anneau à part : son focus est son état propre.
fn field<'a>(look: Look) -> Element<'a, Message> {
    use iced::widget::text_input::Status;
    let status = match look {
        Look::Status(button::Status::Hovered) => Status::Hovered,
        Look::Focus | Look::Status(button::Status::Pressed) => {
            Status::Focused { is_hovered: false }
        }
        Look::Status(button::Status::Disabled) => Status::Disabled,
        Look::Status(_) => Status::Active,
    };
    let on_input = enabled(look).map(|_| Message::Search as fn(String) -> Message);
    ui::text_field("Rechercher", "", on_input)
        .style(move |t, _| theme::text_field(t, status))
        .into()
}

/// Les trois versions du logo, avec et sans carré, à plusieurs tailles.
fn logos<'a>() -> Element<'a, Message> {
    let versions = [
        (logo::COLOR, color::CARD),
        (logo::COLOR_BARE, color::DESKTOP),
        (logo::CREAM, color::CARD),
        (logo::CREAM_BARE, color::DESKTOP),
        (logo::NIGHT, color::CREAM),
        (logo::NIGHT_BARE, color::CREAM),
    ];
    let tiles: [Element<'a, Message>; 6] = versions.map(|(source, behind)| {
        let sizes: [Element<'a, Message>; 4] = LOGO_SIZES.map(|size| {
            svg(svg::Handle::from_memory(source.as_bytes()))
                .width(size)
                .height(size)
                .into()
        });
        container(
            row(sizes)
                .spacing(metrics::GAP_INLINE)
                .align_y(Alignment::End),
        )
        .padding(metrics::GAP_INLINE)
        .style(move |_| iced::widget::container::Style {
            background: Some(iced::Background::Color(behind.into())),
            border: iced::border::rounded(metrics::CARD_RADIUS),
            ..Default::default()
        })
        .into()
    });
    let [a, b, c, d, e, f] = tiles;
    column![
        row![a, b, c].spacing(metrics::GAP_ROW),
        row![d, e, f].spacing(metrics::GAP_ROW)
    ]
    .spacing(metrics::GAP_ROW)
    .into()
}
