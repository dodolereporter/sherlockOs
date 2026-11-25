//! Application view, matching the Evidence mock-up: title bar, toolbar with breadcrumb,
//! sidebar, listing, preview, status bar, right-click menu.

use iced::widget::{
    Column, Row, button, column, container, image, mouse_area, opaque, pin, row, scrollable, space,
    stack, text,
};
use iced::{Alignment, Element, Length, Padding};
use std::time::Duration;

use sherlock_ui::animated::{hoverable, scaled};
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::texts::{self, evidence as t};
use sherlock_ui::theme::{self, Tone};
use sherlock_ui::{color, metrics, typography};

use crate::app::{App, Command, Location, Menu, MenuItem, Message, RENAME, SEARCH, WindowAction};
use crate::clock;
use crate::devices::Volume;
use crate::git::GitStatus;
use crate::listing::{Column as SortColumn, Entry, Kind};
use crate::preview::Preview;

/// Vertical rule between two panels.
fn vline<'a>() -> Element<'a, Message> {
    container(space())
        .width(metrics::BORDER_WIDTH)
        .height(Length::Fill)
        .style(theme::divider)
        .into()
}

/// Secondary text of a cell, with a fixed width.
fn cell<'a>(content: String, width: f32) -> Element<'a, Message> {
    ui::body(content)
        .color(color::TEXT_SECONDARY)
        .wrapping(text::Wrapping::None)
        .width(width)
        .into()
}

/// Secondary text of a cell, without a copy.
fn cell_ref(content: &str, width: f32) -> Element<'_, Message> {
    ui::body(content)
        .color(color::TEXT_SECONDARY)
        .wrapping(text::Wrapping::None)
        .width(width)
        .into()
}

/// Cuts a text to `max` characters, with an ellipsis.
fn elide(text: &str, max: usize) -> std::borrow::Cow<'_, str> {
    match text.char_indices().nth(max.saturating_sub(1)) {
        Some((cut, _)) if text.chars().count() > max => {
            std::borrow::Cow::Owned(format!("{}{}", &text[..cut], t::FOLDED))
        }
        _ => std::borrow::Cow::Borrowed(text),
    }
}

/// Veil of a background colour, for fades.
fn veil<'a>(fill: color::Color, alpha: f32) -> Element<'a, Message> {
    let fill = color::Color { a: alpha, ..fill };
    container(space())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| theme::veil(fill))
        .into()
}

/// Label and tone of the git status.
fn git_badge(status: GitStatus) -> (&'static str, Tone) {
    match status {
        GitStatus::Modified => (t::GIT_MODIFIED, Tone::Warning),
        GitStatus::New => (t::GIT_NEW, Tone::Success),
        GitStatus::Ignored => (t::GIT_IGNORED, Tone::Neutral),
    }
}

impl App {
    pub fn view(&self) -> Element<'_, Message> {
        let now = self.now();
        let mut body = Row::new()
            .push(self.sidebar())
            .push(vline())
            .push(self.list(now));
        if self.show_preview {
            body = body.push(vline()).push(self.preview_panel(now));
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
        let back = ui::toolbar_button(
            Icon::Back,
            t::BACK,
            self.can_go_back().then_some(Message::Back),
        );
        let forward = ui::toolbar_button(
            Icon::Chevron,
            t::FORWARD,
            self.can_go_forward().then_some(Message::Forward),
        );
        let mut crumbs = self.breadcrumb();
        let folded = crumbs.len() > metrics::BREADCRUMB_MAX;
        if folded {
            crumbs.drain(1..crumbs.len() + 1 - metrics::BREADCRUMB_MAX);
        }
        let chevron = || ui::icon(Icon::Chevron, metrics::ICON_SIZE_TINY, color::TEXT_TERTIARY);
        let mut trail = Row::new()
            .spacing(metrics::BREADCRUMB_GAP)
            .align_y(Alignment::Center);
        let count = crumbs.len();
        for (i, (label, location)) in crumbs.into_iter().enumerate() {
            if i > 0 {
                trail = trail.push(chevron());
            }
            if folded && i == 1 {
                trail = trail
                    .push(ui::body(t::FOLDED).color(color::TEXT_TERTIARY))
                    .push(chevron());
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
        let search = ui::search_field(
            &t::search_placeholder(&self.location_name()),
            &self.search,
            Some(Message::Search),
            SEARCH,
            metrics::SEARCH_WIDTH,
        );
        let line = line.push(space::horizontal()).push(search);
        container(line)
            .width(Length::Fill)
            .height(metrics::TOOLBAR_HEIGHT)
            .padding([0.0, metrics::TOOLBAR_PADDING])
            .align_y(Alignment::Center)
            .into()
    }

    /// The deepest favourite that contains the current location: under Documents, Documents
    /// is active. Home is active only for the home folder itself.
    fn active_place(&self) -> Option<usize> {
        let Location::Folder(dir) = &self.location else {
            return None;
        };
        self.config
            .favorites
            .iter()
            .enumerate()
            .filter(|(i, place)| dir.starts_with(&place.path) && (*i > 0 || *dir == place.path))
            .max_by_key(|(_, place)| place.path.components().count())
            .map(|(i, _)| i)
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let active = self.active_place();
        let mut nav = Column::new().push(ui::nav_section(t::FAVORITES));
        for (i, place) in self.config.favorites.iter().enumerate() {
            let here = Location::Folder(place.path.clone());
            let go = place.path.is_dir().then_some(Message::Go(here));
            nav = nav.push(ui::nav_item(
                place.icon,
                place.label,
                None,
                active == Some(i),
                go,
            ));
        }
        if !self.volumes.is_empty() {
            nav = nav.push(ui::nav_section(t::DEVICES));
            for volume in &self.volumes {
                nav = nav.push(self.device_item(volume));
            }
        }
        let root = std::path::PathBuf::from("/");
        nav = nav.push(ui::nav_section(t::SYSTEM)).push(ui::nav_item(
            Icon::SystemDisk,
            t::SYSTEM_DISK,
            None,
            self.location == Location::Folder(root.clone()),
            Some(Message::Go(Location::Folder(root))),
        ));
        if self.config.trash.is_some() {
            let active = self.location == Location::Trash;
            nav = nav.push(ui::nav_item(
                Icon::Trash,
                t::TRASH,
                None,
                active,
                Some(Message::Go(Location::Trash)),
            ));
        }
        let (top, side, bottom) = metrics::SIDEBAR_PADDING;
        container(nav)
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

    /// A volume of the sidebar: its icon, its name, its free space, and an eject button for
    /// a removable medium. A right click opens its menu.
    fn device_item<'a>(&'a self, volume: &'a Volume) -> Element<'a, Message> {
        let active = match (&volume.mount_point, &self.location) {
            (Some(point), Location::Folder(dir)) => dir.starts_with(point),
            _ => false,
        };
        let detail = volume.detail();
        let eject = volume.ejectable.then(|| {
            ui::icon_button(
                Icon::Eject,
                t::EJECT,
                Some(Message::EjectDevice(volume.id.clone())),
            )
        });
        // Room for the name: the item minus its icon, the detail and the eject button.
        let (_, side, _) = metrics::SIDEBAR_PADDING;
        let mut room = metrics::SIDEBAR_WIDTH
            - 2.0 * side
            - 2.0 * metrics::NAV_ITEM_PADDING
            - metrics::ICON_SIZE
            - metrics::GAP_ICON;
        if let Some(detail) = &detail {
            room -= detail.chars().count() as f32 * typography::UI_CHAR_WIDTH + metrics::GAP_ICON;
        }
        if eject.is_some() {
            room -= metrics::ICON_BUTTON;
        }
        let chars = (room / typography::UI_CHAR_WIDTH).max(3.0) as usize;
        let glyph = if volume.locked {
            Icon::Lock
        } else {
            volume.kind.icon()
        };
        let item = ui::nav_item_owned(
            glyph,
            elide(&volume.name, chars).into_owned(),
            detail,
            active,
            Some(Message::OpenDevice(volume.id.clone())),
        );
        let line: Element<'a, Message> = match eject {
            Some(eject) => row![container(item).width(Length::Fill), eject]
                .align_y(Alignment::Center)
                .into(),
            None => item.into(),
        };
        mouse_area(line)
            .on_right_press(Message::DeviceMenu(volume.id.clone()))
            .into()
    }

    fn list(&self, now: Duration) -> Element<'_, Message> {
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

        // The content of a folder that opens rises a few pixels while fading in. With no
        // group opacity in iced, the fade is a veil of the window background laid on top,
        // which fades away.
        let reveal = self.content_fade.as_ref().map_or(1.0, |f| f.value(now));
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
            scrollable(self.rows(now, (1.0 - reveal) * metrics::LIST_SLIDE))
                .id(crate::app::LIST)
                .on_scroll(|v| Message::Scrolled(v.absolute_offset().y, v.bounds().height))
                .height(Length::Fill)
                .into()
        };
        let content = mouse_area(container(content).width(Length::Fill).height(Length::Fill))
            .on_press(Message::Blank)
            .on_right_press(Message::RightClick(None));
        let content: Element<'_, Message> = if reveal < 1.0 {
            stack![content, veil(color::WINDOW, 1.0 - reveal)].into()
        } else {
            content.into()
        };
        column![headers, ui::divider(), content]
            .width(Length::Fill)
            .padding([0.0, metrics::LIST_PADDING])
            .into()
    }

    /// Only the visible rows, between two spacers that hold the place of the others: a
    /// listing of 5,000 files lays out only a few dozen widgets.
    fn rows(&self, now: Duration, slide: f32) -> Element<'_, Message> {
        let row_height = metrics::LIST_ROW_HEIGHT;
        let total = self.entries.len();
        let (offset, _) = self.scroll;
        let visible = self.size.height;
        // Only the visible rows (plus a margin) are built, between two spacers that hold the
        // place of the others: a folder of 5,000 files puts a few dozen widgets in the tree.
        let first = ((offset - metrics::LIST_TOP_GAP) / row_height)
            .floor()
            .max(0.0) as usize;
        let first = first.saturating_sub(metrics::LIST_OVERSCAN).min(total);
        let last = ((offset + visible) / row_height).ceil() as usize + metrics::LIST_OVERSCAN;
        let last = last.min(total);
        let before = metrics::LIST_TOP_GAP + first as f32 * row_height + slide;
        let after = (total - last) as f32 * row_height;
        let name_chars = self.name_chars();
        let rows = Column::new()
            .push(space().height(before))
            .extend((first..last).map(|i| self.row(i, &self.entries[i], name_chars)))
            .push(space().height(after))
            .width(Length::Fill);
        // A single selection is drawn by a highlight that glides under the rows. It is the
        // first layer of the stack, which sets the stack size, so it spans the whole list.
        // Every other layer of a stack, and every clipped container, costs the software
        // renderer a full clip mask: the rows are one layer, and nothing in them clips.
        match &self.highlight {
            Some(highlight) if self.selection.len() == 1 => {
                let y = highlight.value(now) as f32 + slide;
                let bar = container(space())
                    .width(Length::Fill)
                    .height(row_height)
                    .style(theme::list_row(true));
                let height = before + (total - first) as f32 * row_height;
                let layer = column![space().height(y.max(0.0)), bar]
                    .width(Length::Fill)
                    .height(height);
                stack![layer, rows].width(Length::Fill).into()
            }
            _ => rows.into(),
        }
    }

    /// Characters of a name that fit in the name column, estimated from the window width:
    /// a longer name is cut with an ellipsis rather than clipped, which would cost a mask.
    fn name_chars(&self) -> usize {
        let preview = if self.show_preview {
            metrics::PREVIEW_WIDTH + metrics::BORDER_WIDTH
        } else {
            0.0
        };
        let fixed: f32 = metrics::LIST_COLUMNS.iter().sum::<f32>()
            + 4.0 * metrics::LIST_COLUMN_GAP
            + 2.0 * (metrics::LIST_PADDING + metrics::LIST_ROW_PADDING)
            + metrics::LIST_ICON
            + metrics::GAP_ICON
            + metrics::SIDEBAR_WIDTH
            + metrics::BORDER_WIDTH
            + preview;
        ((self.size.width - fixed) / typography::UI_CHAR_WIDTH).max(4.0) as usize
    }

    fn row<'a>(&'a self, i: usize, entry: &'a Entry, name_chars: usize) -> Element<'a, Message> {
        let selected = self.selection.contains(&i);
        // A single selection is drawn by the gliding highlight.
        let painted = selected && self.selection.len() > 1;
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
            _ => ui::body(elide(&entry.name, name_chars))
                .font(typography::UI_MEDIUM)
                .wrapping(text::Wrapping::None)
                .into(),
        };
        let name = row![ui::icon(entry.icon(), metrics::LIST_ICON, tint), name]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center)
            .width(Length::Fill);
        let [w_date, w_size, w_kind, w_last] = metrics::LIST_COLUMNS;
        let last: Element<'a, Message> = match (&entry.origin, entry.git) {
            (Some(origin), _) => cell(self.origin_label(origin), w_last),
            (None, Some(status)) => {
                let (label, tone) = git_badge(status);
                container(ui::badge(label, tone, selected))
                    .width(w_last)
                    .into()
            }
            (None, None) => space().width(w_last).into(),
        };
        let line = row![
            name,
            cell_ref(&entry.labels.date, w_date),
            cell_ref(&entry.labels.size, w_size),
            cell_ref(&entry.labels.kind, w_kind),
            last,
        ]
        .spacing(metrics::LIST_COLUMN_GAP)
        .align_y(Alignment::Center);
        let line = container(line)
            .width(Length::Fill)
            .height(metrics::LIST_ROW_HEIGHT)
            .padding([0.0, metrics::LIST_ROW_PADDING])
            .align_y(Alignment::Center)
            .style(theme::list_row(painted));
        let line = hoverable(line, color::HOVER, metrics::LIST_ROW_RADIUS);
        mouse_area(line)
            .on_press(Message::Click(i))
            .on_double_click(Message::DoubleClick(i))
            .on_right_press(Message::RightClick(Some(i)))
            .into()
    }

    fn preview_panel(&self, now: Duration) -> Element<'_, Message> {
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
                let subtitle = match entry.git {
                    Some(status) => t::in_git(&entry.labels.size, git_badge(status).0),
                    None => entry.labels.size.clone(),
                };
                // Names are cut rather than clipped: a clip costs the renderer a mask.
                let room = metrics::PREVIEW_WIDTH
                    - 2.0 * metrics::PREVIEW_PADDING
                    - metrics::PREVIEW_TILE
                    - metrics::PREVIEW_GAP;
                let scale = typography::CARD_TITLE_SIZE / typography::UI_SIZE;
                let chars = (room / (typography::UI_CHAR_WIDTH * scale)) as usize;
                let title = column![
                    ui::body(elide(&entry.name, chars))
                        .font(typography::UI_SEMIBOLD)
                        .size(typography::CARD_TITLE_SIZE)
                        .wrapping(text::Wrapping::None),
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
                // The preview fades in: a veil of the panel background that fades away.
                let reveal = self.preview_fade.as_ref().map_or(1.0, |f| f.value(now));
                let body: Element<'_, Message> = if reveal < 1.0 {
                    stack![body, veil(color::SIDEBAR, 1.0 - reveal)].into()
                } else {
                    body
                };
                let action = if in_trash {
                    ui::primary_block(t::RESTORE, Some(Message::Run(Command::Restore)))
                } else {
                    ui::primary_block(t::OPEN, Some(Message::Run(Command::Open)))
                };
                let copy = ui::secondary_block(t::COPY_PATH, Some(Message::Run(Command::CopyPath)));
                let actions = column![action, copy].spacing(metrics::BUTTON_STACK_GAP);
                column![header, body, actions]
                    .spacing(metrics::PREVIEW_GAP)
                    .into()
            }
        };
        container(content)
            .width(metrics::PREVIEW_WIDTH)
            .height(Length::Fill)
            .padding(metrics::PREVIEW_PADDING)
            .style(theme::sidebar)
            .into()
    }

    /// Information of an item without a preview: date, kind, location.
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
            (t::COLUMN_KIND, Some(entry.labels.kind.clone())),
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
        // The menu opens by scaling from its top left corner, on a spring.
        let factor = self
            .menu_scale
            .as_ref()
            .map_or(1.0, |a| a.value(self.now()) as f32);
        let main = scaled(ui::menu(entries, metrics::CONTEXT_MENU_WIDTH), factor);
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

/// An entry of the menu or submenu. `lit`: highlighted from the keyboard. Hovering an entry of
/// the main menu opens or closes the submenu, as in any menu.
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
