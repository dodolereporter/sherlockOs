//! View of the gallery, shared by the example and by its tests. It lives outside the library:
//! the shell programs compile neither it nor its sample texts.
//!
//! Every component appears in its five states, on the desktop then on glass, with the
//! « Fenêtre » menu of the Menus design board, cards of the « Réglages » design board and the
//! logo. The texts are those of the design boards.

use iced::widget::{Column, button, column, container, row, scrollable, space, svg};
use iced::{Alignment, Element, Length, Subscription, Task, keyboard};

use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::focus::{self, Direction};
use sherlock_ui::icon::Icon;
use sherlock_ui::logo::Logo;
use sherlock_ui::{color, metrics, theme, typography};

/// Layout specific to the gallery. These sizes do not come from the design: they arrange the
/// table of states and the logos, and are used nowhere else.
pub mod layout {
    /// Width of a cell of the table of states, and of the names column.
    pub const CELL: f32 = 180.0;
    pub const NAME: f32 = 150.0;
    /// Gap between the menus column and the settings column.
    pub const COLUMN_GAP: f32 = 28.0;
    /// Display sizes of the logo.
    pub const LOGO_SIZES: [f32; 4] = [96.0, 64.0, 32.0, 16.0];
}

use layout::{CELL, NAME};

/// State of the gallery: what the live components change.
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

/// The five states shown for each component.
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
    /// The toggles on, as on the « Réglages » design board.
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
            row![self.menus(), self.settings()].spacing(layout::COLUMN_GAP),
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

    /// The « Fenêtre » menu of the Menus design board, and its « Déplacer vers le bureau »
    /// submenu.
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
            // The submenu lines up with its entry, below four entries and a separator.
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
        row![window, sub].spacing(metrics::SUBMENU_GAP).into()
    }

    /// Two cards of the « Réglages » design board.
    fn settings(&self) -> Element<'_, Message> {
        let toggles = ui::card(column![
            ui::setting_row(
                "Fréquence variable",
                "L'écran suit le rythme du jeu.",
                ui::toggle(self.variable_rate, Some(Message::VariableRate)),
            ),
            ui::divider(),
            ui::setting_row(
                "Couper les notifications",
                "Sherlock garde tout pour après.",
                ui::toggle(self.quiet, Some(Message::Quiet)),
            ),
        ]);
        let driver = ui::card(
            row![
                column![
                    ui::body("Pilote NVIDIA")
                        .font(typography::UI_SEMIBOLD)
                        .size(typography::CARD_TITLE_SIZE),
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

/// Builds the cell of a component in a given state.
type Cell<'a> = fn(Look) -> Element<'a, Message>;

/// The table of states: one row per component, one column per state.
fn states<'a>() -> Element<'a, Message> {
    let header = LOOKS
        .iter()
        .fold(row![space().width(NAME)], |r, (name, _)| {
            r.push(container(ui::section_label(name)).width(CELL))
        });
    let lines: [(&str, Cell<'a>); 13] = [
        ("Bouton principal", |look| {
            control(ui::primary_button("Relancer", enabled(look)), look)
        }),
        ("Bouton secondaire", |look| {
            control(ui::secondary_button("Voir les logs", enabled(look)), look)
        }),
        ("Bouton destructif", |look| {
            control(ui::danger_button("Vider", enabled(look)), look)
        }),
        ("Barre d'outils", |look| {
            control(
                ui::toolbar_button(Icon::Back, "Précédent", enabled(look)),
                look,
            )
        }),
        ("Icône seule", |look| {
            control(ui::icon_button(Icon::Close, "Fermer", enabled(look)), look)
        }),
        ("Interrupteur allumé", |look| {
            control(ui::toggle(true, toggled(look)), look)
        }),
        ("Interrupteur éteint", |look| {
            control(ui::toggle(false, toggled(look)), look)
        }),
        ("Entrée de menu", |look| {
            // A menu entry only lives in a menu: it is shown in one.
            let item = ui::menu_item("Réglages", Trailing::Shortcut("Super+,"), enabled(look));
            ui::menu([control(item, look)], CELL - 2.0 * metrics::GAP_INLINE).into()
        }),
        ("Entrée destructive", |look| {
            let item = ui::menu_item(
                "Mettre à la corbeille",
                Trailing::Shortcut("Suppr"),
                enabled(look),
            );
            ui::menu(
                [control(ui::danger(item), look)],
                CELL - 2.0 * metrics::GAP_INLINE,
            )
            .into()
        }),
        ("Navigation", |look| {
            control(
                ui::nav_item(Icon::Folder, "Documents", None, false, enabled(look)),
                look,
            )
        }),
        ("Navigation active", |look| {
            control(
                ui::nav_item(Icon::Home, "Accueil", None, true, enabled(look)),
                look,
            )
        }),
        ("Champ vide", |look| field(look, "")),
        ("Champ rempli", |look| field(look, "Wi-Fi")),
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

fn toggled(look: Look) -> Option<fn(bool) -> Message> {
    enabled(look).map(|_| Message::Quiet as fn(bool) -> Message)
}

fn control<'a>(control: ui::Control<'a, Message>, look: Look) -> Element<'a, Message> {
    match look {
        Look::Status(status) => control.status(status).into(),
        Look::Focus => control.focus_ring().into(),
    }
}

/// The text field has neither a pressed state nor a separate ring: its focus is its own state.
fn field<'a>(look: Look, value: &str) -> Element<'a, Message> {
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
    ui::text_field("Rechercher", value, on_input)
        .style(move |t, _| theme::text_field(t, status))
        .into()
}

/// The three versions of the logo, with a square then without, at several sizes. At 32 and
/// 16 px, the square takes its optical version.
fn logos<'a>() -> Element<'a, Message> {
    let tile = |sources: [&'static str; 4], behind: color::Color| -> Element<'a, Message> {
        let images = sources
            .into_iter()
            .zip(layout::LOGO_SIZES)
            .map(|(source, size)| {
                svg(svg::Handle::from_memory(source.as_bytes()))
                    .width(size)
                    .height(size)
                    .into()
            });
        container(
            row(images)
                .spacing(metrics::GAP_INLINE)
                .align_y(Alignment::End),
        )
        .padding(metrics::GAP_INLINE)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(behind.into())),
            border: iced::border::rounded(metrics::CARD_RADIUS),
            ..container::Style::default()
        })
        .into()
    };
    let lines = Logo::ALL.map(|logo| {
        let squares = layout::LOGO_SIZES.map(|size| logo.square(size));
        let bare = [logo.bare(); 4];
        let behind = if logo == Logo::Night {
            color::CREAM
        } else {
            color::DESKTOP
        };
        row![tile(squares, color::CARD), tile(bare, behind)]
            .spacing(metrics::GAP_ROW)
            .into()
    });
    Column::with_children(lines)
        .spacing(metrics::GAP_ROW)
        .into()
}
