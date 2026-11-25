//! Vue de l'application, conforme à la planche Fichiers : barre de titre, barre d'outils avec
//! fil d'Ariane, barre latérale, liste, aperçu, barre d'état, menu du clic droit.

use iced::widget::{
    Column, Row, button, column, container, image, mouse_area, opaque, pin, row, scrollable, space,
    stack, text,
};
use iced::{Alignment, Element, Length, Padding};
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::texts::{self, LocalTime, files as t};
use sherlock_ui::theme::{self, Tone};
use sherlock_ui::{color, metrics, typography};

use crate::app::{App, Command, Location, Menu, MenuItem, Message, RENAME, SEARCH, WindowAction};
use crate::clock;
use crate::git::GitStatus;
use crate::listing::{Column as SortColumn, Entry, Kind};
use crate::preview::Preview;

/// Trait vertical entre deux panneaux.
fn vline<'a>() -> Element<'a, Message> {
    container(space())
        .width(metrics::BORDER_WIDTH)
        .height(Length::Fill)
        .style(theme::divider)
        .into()
}

/// Texte secondaire d'une cellule, à largeur fixe.
fn cell<'a>(content: String, width: f32) -> Element<'a, Message> {
    ui::body(content)
        .color(color::TEXT_SECONDARY)
        .wrapping(text::Wrapping::None)
        .width(width)
        .into()
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let now = clock::now();
        let mut body = Row::new()
            .push(self.sidebar())
            .push(vline())
            .push(self.list(now));
        if self.show_preview {
            body = body.push(vline()).push(self.preview_panel());
        }
        let page = column![
            self.title_bar(),
            ui::divider(),
            self.toolbar(),
            ui::divider(),
            body.height(Length::Fill),
            ui::divider(),
            self.status_bar(),
        ];
        let mut layers: Vec<Element<'_, Message>> = vec![
            container(page)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::window)
                .into(),
        ];
        if let Some(menu) = &self.menu {
            let catcher = mouse_area(space().width(Length::Fill).height(Length::Fill))
                .on_press(Message::CloseMenu)
                .on_right_press(Message::CloseMenu);
            layers.push(opaque(catcher));
            layers.push(self.context_menu(menu));
        }
        if self.confirm_empty {
            layers.push(self.dialog());
        }
        stack(layers).into()
    }

    fn title_bar(&self) -> Element<'_, Message> {
        let badge = container(space())
            .width(metrics::APP_BADGE)
            .height(metrics::APP_BADGE)
            .style(theme::app_badge);
        let controls = ui::window_controls(
            [t::MINIMIZE, t::MAXIMIZE, t::CLOSE],
            [
                Message::Window(WindowAction::Minimize),
                Message::Window(WindowAction::Maximize),
                Message::Window(WindowAction::Close),
            ],
        );
        let line = row![
            badge,
            ui::body(t::APP_NAME).font(typography::UI_SEMIBOLD),
            ui::body(self.location_name()).color(color::TEXT_TERTIARY),
            space::horizontal(),
            controls,
        ]
        .spacing(metrics::GAP_ICON)
        .align_y(Alignment::Center);
        let (left, right) = metrics::TITLE_BAR_PADDING;
        let bar = container(line)
            .width(Length::Fill)
            .height(metrics::TITLE_BAR_HEIGHT)
            .padding(Padding {
                left,
                right,
                ..Padding::ZERO
            })
            .align_y(Alignment::Center)
            .style(theme::title_bar);
        mouse_area(bar)
            .on_press(Message::Window(WindowAction::Drag))
            .on_double_click(Message::Window(WindowAction::Maximize))
            .into()
    }

    fn toolbar(&self) -> Element<'_, Message> {
        let back = ui::icon_button(
            Icon::Back,
            t::BACK,
            self.can_go_back().then_some(Message::Back),
        );
        let forward = ui::icon_button(
            Icon::Chevron,
            t::FORWARD,
            self.can_go_forward().then_some(Message::Forward),
        );
        let mut crumbs = self.breadcrumb();
        let folded = crumbs.len() > metrics::BREADCRUMB_MAX;
        if folded {
            crumbs.drain(1..crumbs.len() + 1 - metrics::BREADCRUMB_MAX);
        }
        let mut trail = Row::new()
            .spacing(metrics::BREADCRUMB_GAP)
            .align_y(Alignment::Center);
        let count = crumbs.len();
        for (i, (label, location)) in crumbs.into_iter().enumerate() {
            if i > 0 {
                let chevron =
                    ui::icon(Icon::Chevron, metrics::ICON_SIZE_TINY, color::TEXT_TERTIARY);
                trail = trail.push(chevron);
            }
            if folded && i == 1 {
                trail = trail
                    .push(ui::body(t::FOLDED).color(color::TEXT_TERTIARY))
                    .push(ui::icon(
                        Icon::Chevron,
                        metrics::ICON_SIZE_TINY,
                        color::TEXT_TERTIARY,
                    ));
            }
            let current = i + 1 == count;
            trail = trail.push(ui::crumb(label, current, Some(Message::Go(location))));
        }
        let trail = container(trail)
            .height(metrics::BREADCRUMB_HEIGHT)
            .padding([0.0, metrics::BREADCRUMB_PADDING])
            .align_y(Alignment::Center)
            .clip(true)
            .style(theme::inset);
        let mut line = row![back, forward, trail]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center);
        if let Some(branch) = self.repo.as_ref().and_then(|r| r.branch.as_deref()) {
            line = line.push(ui::chip(t::branch(branch)));
        }
        let search = ui::text_field(
            &t::search_placeholder(&self.location_name()),
            &self.search,
            Some(Message::Search),
        )
        .id(SEARCH)
        .width(metrics::SEARCH_WIDTH);
        let line = line.push(space::horizontal()).push(search);
        container(line)
            .width(Length::Fill)
            .height(metrics::TOOLBAR_HEIGHT)
            .padding([0.0, metrics::TOOLBAR_PADDING])
            .align_y(Alignment::Center)
            .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let mut nav = Column::new().push(ui::nav_section(t::FAVORITES));
        for place in &self.config.favorites {
            let here = Location::Folder(place.path.clone());
            let active = self.location == here;
            let go = place.path.is_dir().then_some(Message::Go(here));
            nav = nav.push(ui::nav_item(place.icon, place.label, None, active, go));
        }
        if self.config.trash.is_some() {
            let active = self.location == Location::Trash;
            nav = nav.push(ui::nav_section(t::SYSTEM)).push(ui::nav_item(
                Icon::Trash,
                t::TRASH,
                None,
                active,
                Some(Message::Go(Location::Trash)),
            ));
        }
        let (top, side, bottom) = metrics::SIDEBAR_PADDING;
        container(nav.spacing(metrics::GAP_TIGHT))
            .width(metrics::SIDEBAR_WIDTH)
            .height(Length::Fill)
            .padding(Padding {
                top,
                right: side,
                bottom,
                left: side,
            })
            .style(theme::sidebar)
            .into()
    }

    fn list(&self, now: Option<LocalTime>) -> Element<'_, Message> {
        let in_trash = self.location == Location::Trash;
        let [w_date, w_size, w_kind, w_last] = metrics::LIST_COLUMNS;
        let header = |label: &str, column: SortColumn, sortable: bool| {
            let sorted = (self.sort.column == column).then_some(self.sort.ascending);
            ui::column_header(label, sorted, sortable.then_some(Message::Sort(column)))
        };
        let (date_label, last_label) = if in_trash {
            (t::COLUMN_DELETED, t::COLUMN_ORIGIN)
        } else {
            (t::COLUMN_MODIFIED, t::COLUMN_GIT)
        };
        let headers = row![
            container(header(t::COLUMN_NAME, SortColumn::Name, true)).width(Length::Fill),
            container(header(date_label, SortColumn::Modified, true)).width(w_date),
            container(header(t::COLUMN_SIZE, SortColumn::Size, true)).width(w_size),
            container(header(t::COLUMN_KIND, SortColumn::Kind, true)).width(w_kind),
            container(header(last_label, SortColumn::Git, !in_trash)).width(w_last),
        ]
        .spacing(metrics::LIST_COLUMN_GAP)
        .align_y(Alignment::Center);
        let headers = container(headers)
            .height(metrics::LIST_HEADER_HEIGHT)
            .padding([0.0, metrics::LIST_ROW_PADDING])
            .align_y(Alignment::Center);

        let content: Element<'_, Message> = if self.entries.is_empty() {
            let empty = match (in_trash, self.search.is_empty()) {
                (_, false) => t::NO_MATCH,
                (true, true) => t::EMPTY_TRASH_VIEW,
                (false, true) => t::EMPTY_FOLDER,
            };
            container(ui::description(empty))
                .center(Length::Fill)
                .into()
        } else {
            let rows = self
                .entries
                .iter()
                .enumerate()
                .map(|(i, entry)| self.row(i, entry, now));
            scrollable(Column::with_children(rows).padding(Padding {
                top: metrics::LIST_TOP_GAP,
                ..Padding::ZERO
            }))
            .id(crate::app::LIST)
            .on_scroll(|v| Message::Scrolled(v.absolute_offset().y, v.bounds().height))
            .height(Length::Fill)
            .into()
        };
        let content = mouse_area(container(content).width(Length::Fill).height(Length::Fill))
            .on_press(Message::Blank)
            .on_right_press(Message::RightClick(None));
        column![headers, ui::divider(), content]
            .width(Length::Fill)
            .padding([0.0, metrics::LIST_PADDING])
            .into()
    }

    fn row<'a>(
        &'a self,
        i: usize,
        entry: &'a Entry,
        now: Option<LocalTime>,
    ) -> Element<'a, Message> {
        let selected = self.selection.contains(&i);
        let tint = match entry.kind {
            Kind::Folder => color::BLUE_LIGHT,
            _ => color::TEXT_SECONDARY,
        };
        let name: Element<'a, Message> = match &self.renaming {
            Some((index, value)) if *index == i => {
                ui::text_field("", value, Some(Message::RenameInput))
                    .id(RENAME)
                    .on_submit(Message::RenameSubmit)
                    .into()
            }
            _ => ui::body(entry.name.as_str())
                .font(typography::UI_MEDIUM)
                .wrapping(text::Wrapping::None)
                .into(),
        };
        let name = row![ui::icon(entry.icon(), metrics::LIST_ICON, tint), name]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .clip(true);
        let [w_date, w_size, w_kind, w_last] = metrics::LIST_COLUMNS;
        let date = entry
            .modified
            .and_then(clock::local)
            .zip(now)
            .map(|(when, now)| texts::short_date(when, now))
            .unwrap_or_default();
        let size = match entry.kind {
            Kind::Folder => entry.children.map(texts::items).unwrap_or_default(),
            Kind::File => texts::size(entry.size),
            Kind::Link => String::new(),
        };
        let last: Element<'a, Message> = match (&entry.origin, entry.git) {
            (Some(origin), _) => cell(self.origin_label(origin), w_last),
            (None, Some(status)) => {
                let (label, tone) = match status {
                    GitStatus::Modified => (t::GIT_MODIFIED, Tone::Warning),
                    GitStatus::New => (t::GIT_NEW, Tone::Success),
                    GitStatus::Ignored => (t::GIT_IGNORED, Tone::Neutral),
                };
                container(ui::badge(label, tone, selected))
                    .width(w_last)
                    .into()
            }
            (None, None) => space().width(w_last).into(),
        };
        let line = row![
            name,
            cell(date, w_date),
            cell(size, w_size),
            cell(entry.kind_label(), w_kind),
            last,
        ]
        .spacing(metrics::LIST_COLUMN_GAP)
        .align_y(Alignment::Center);
        let line = container(line)
            .width(Length::Fill)
            .height(metrics::LIST_ROW_HEIGHT)
            .padding([0.0, metrics::LIST_ROW_PADDING])
            .align_y(Alignment::Center)
            .style(theme::list_row(selected));
        mouse_area(line)
            .on_press(Message::Click(i))
            .on_double_click(Message::DoubleClick(i))
            .on_right_press(Message::RightClick(Some(i)))
            .into()
    }

    fn preview_panel(&self) -> Element<'_, Message> {
        let content: Element<'_, Message> = match self.single() {
            None => container(ui::description(t::NOTHING_SELECTED))
                .center(Length::Fill)
                .into(),
            Some(entry) => {
                let in_trash = self.location == Location::Trash;
                let tile = container(ui::icon(
                    entry.icon(),
                    metrics::PREVIEW_TILE_ICON,
                    color::DEEPEST,
                ))
                .center(metrics::PREVIEW_TILE)
                .style(theme::preview_tile);
                let size = match entry.kind {
                    Kind::Folder => entry.children.map(texts::items).unwrap_or_default(),
                    _ => texts::size(entry.size),
                };
                let subtitle = match entry.git {
                    Some(GitStatus::Modified) => t::in_git(&size, t::GIT_MODIFIED),
                    Some(GitStatus::New) => t::in_git(&size, t::GIT_NEW),
                    Some(GitStatus::Ignored) => t::in_git(&size, t::GIT_IGNORED),
                    None => size,
                };
                let title = column![
                    ui::body(entry.name.as_str())
                        .font(typography::UI_SEMIBOLD)
                        .size(typography::CARD_TITLE_SIZE),
                    ui::body(subtitle).color(color::TEXT_SECONDARY),
                ];
                let header = row![tile, title]
                    .spacing(metrics::PREVIEW_GAP)
                    .align_y(Alignment::Center);
                let body: Element<'_, Message> = match &self.preview {
                    Preview::Text(lines) => ui::code_block(lines).into(),
                    Preview::Image(path) => image(path.as_path()).width(Length::Fill).into(),
                    Preview::Info => self.info(entry),
                };
                let action = if in_trash {
                    ui::primary_button(t::RESTORE, Some(Message::Run(Command::Restore)))
                } else {
                    ui::primary_button(t::OPEN, Some(Message::Run(Command::Open)))
                };
                let copy =
                    ui::secondary_button(t::COPY_PATH, Some(Message::Run(Command::CopyPath)));
                let actions = column![action.width(Length::Fill), copy.width(Length::Fill)]
                    .spacing(metrics::BUTTON_STACK_GAP);
                column![header, body, actions]
                    .spacing(metrics::PREVIEW_GAP)
                    .into()
            }
        };
        container(content)
            .width(metrics::PREVIEW_WIDTH)
            .height(Length::Fill)
            .padding(metrics::PREVIEW_PADDING)
            .clip(true)
            .style(theme::sidebar)
            .into()
    }

    /// Informations d'un élément sans aperçu : date, type, emplacement.
    fn info<'a>(&self, entry: &'a Entry) -> Element<'a, Message> {
        let date = entry.modified.and_then(clock::local).map(texts::full_date);
        let date_label = match entry.origin {
            Some(_) => t::COLUMN_DELETED,
            None => t::COLUMN_MODIFIED,
        };
        let place = entry
            .origin
            .as_deref()
            .or(Some(entry.path.as_path()))
            .and_then(|p| p.parent())
            .map(|p| p.display().to_string());
        let lines = [
            (date_label, date),
            (t::COLUMN_KIND, Some(entry.kind_label())),
            (t::LOCATION, place),
        ];
        let lines = lines.into_iter().filter_map(|(label, value)| {
            let value = value?;
            Some(
                column![
                    ui::section_label(label),
                    ui::body(value).color(color::TEXT_SECONDARY)
                ]
                .into(),
            )
        });
        Column::with_children(lines)
            .spacing(metrics::PREVIEW_GAP)
            .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let (left, tint) = match &self.notice {
            Some(notice) if notice.error => (notice.text.clone(), color::SALMON),
            Some(notice) => (notice.text.clone(), color::TEXT_TERTIARY),
            None => (
                t::status(self.entries.len(), self.selection.len()),
                color::TEXT_TERTIARY,
            ),
        };
        let small = |content: String, tint| {
            ui::body(content)
                .size(typography::SMALL_SIZE)
                .color(tint)
                .wrapping(text::Wrapping::None)
        };
        let line = row![
            small(left, tint),
            space::horizontal(),
            small(t::HINT.into(), color::TEXT_TERTIARY),
        ]
        .align_y(Alignment::Center);
        container(line)
            .width(Length::Fill)
            .height(metrics::STATUS_BAR_HEIGHT)
            .padding([0.0, metrics::STATUS_BAR_PADDING])
            .align_y(Alignment::Center)
            .into()
    }

    fn context_menu(&self, menu: &Menu) -> Element<'_, Message> {
        let items = self.menu_items(menu.on_selection);
        let entries = items.iter().enumerate().map(|(i, item)| {
            let lit = !menu.submenu && menu.highlight == Some(i);
            menu_entry(*item, lit, menu.submenu, true)
        });
        let main = ui::menu(entries, metrics::CONTEXT_MENU_WIDTH);
        let mut layers: Vec<Element<'_, Message>> =
            vec![pin(main).x(menu.at.x).y(menu.at.y).into()];
        if menu.submenu {
            let before = items
                .iter()
                .position(|item| *item == MenuItem::OpenWith)
                .unwrap_or_default();
            let offset = App::menu_height(&items[..before])
                - 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
            let sub = App::submenu_items();
            let entries = sub
                .iter()
                .enumerate()
                .map(|(i, item)| menu_entry(*item, menu.highlight == Some(i), false, false));
            let x = menu.at.x + metrics::CONTEXT_MENU_WIDTH + metrics::SUBMENU_GAP;
            let sub = ui::menu(entries, metrics::CONTEXT_SUBMENU_WIDTH);
            layers.push(pin(sub).x(x).y(menu.at.y + offset).into());
        }
        stack(layers)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn dialog(&self) -> Element<'_, Message> {
        let actions = row![
            space::horizontal(),
            ui::secondary_button(t::CANCEL, Some(Message::CancelEmpty)),
            ui::danger_button(t::CONFIRM_EMPTY, Some(Message::ConfirmEmpty)),
        ]
        .spacing(metrics::BUTTON_STACK_GAP);
        let card = column![
            ui::body(t::EMPTY_TRASH_QUESTION)
                .font(typography::UI_SEMIBOLD)
                .size(typography::CARD_TITLE_SIZE),
            ui::description(t::empty_trash_detail(self.entries.len())),
            actions,
        ]
        .spacing(metrics::PREVIEW_GAP);
        let panel = ui::glass_panel(card).width(metrics::DIALOG_WIDTH);
        opaque(container(panel).center(Length::Fill).style(theme::scrim))
    }
}

/// Une entrée du menu ou du sous-menu. `lit` : surlignée au clavier. Survoler une entrée du
/// menu principal ouvre ou ferme le sous-menu, comme dans tout menu.
fn menu_entry<'a>(
    item: MenuItem,
    lit: bool,
    submenu_open: bool,
    main: bool,
) -> Element<'a, Message> {
    let light = |control: ui::Control<'a, Message>| {
        if lit {
            control.status(button::Status::Hovered)
        } else {
            control
        }
    };
    match item {
        MenuItem::Separator => ui::menu_separator(),
        MenuItem::Disabled(label) => ui::menu_item(label, Trailing::None, None).into(),
        MenuItem::OpenWith => {
            let mut control = ui::menu_item(
                t::OPEN_WITH,
                Trailing::Submenu,
                Some(Message::Submenu(!submenu_open)),
            );
            if submenu_open {
                control = ui::open(control);
            }
            mouse_area(light(control))
                .on_enter(Message::Submenu(true))
                .into()
        }
        MenuItem::Action {
            label,
            keys,
            command,
            danger,
            checked,
        } => {
            let trailing = match (checked, keys) {
                (true, _) => Trailing::Checked,
                (false, Some(keys)) => Trailing::Shortcut(keys),
                (false, None) => Trailing::None,
            };
            let mut control = ui::menu_item(label, trailing, Some(Message::Run(command)));
            if danger {
                control = ui::danger(control);
            }
            let control = light(control);
            if main && submenu_open {
                mouse_area(control).on_enter(Message::Submenu(false)).into()
            } else {
                control.into()
            }
        }
    }
}
