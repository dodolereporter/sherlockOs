//! The columns view (lot E-13b): the folders above the one shown, the folder shown, and on its
//! right the content of the folder selected or the preview of the file selected.

use std::time::Duration;

use iced::widget::{Column, Row, container, mouse_area, row, space};
use iced::{Alignment, Element, Length};

use sherlock_ui::animated::hoverable;
use sherlock_ui::component as ui;
use sherlock_ui::ellipsis::ellipsis;
use sherlock_ui::evidence as more;
use sherlock_ui::icon::Icon;
use sherlock_ui::scroll::scroll;
use sherlock_ui::texts::evidence as t;
use sherlock_ui::{color, metrics, theme, typography};

use crate::app::{App, DirColumn, LIST, Location, Message};
use crate::listing::{Entry, Kind};

/// The name of a folder, for the header of its column.
fn folder_name(dir: &std::path::Path) -> String {
    dir.file_name().map_or_else(
        || t::ROOT.to_owned(),
        |name| crate::pathbar::visible(&name.to_string_lossy()).into_owned(),
    )
}

/// One line of a column: the icon, the name cut with « … », a chevron after a folder.
fn line<'a>(entry: &'a Entry, lit: bool, on_press: Message) -> Element<'a, Message> {
    let tint = match entry.kind {
        Kind::Folder => color::palette().blue_light,
        _ => color::palette().text_secondary,
    };
    let mut content = row![
        ui::icon(entry.icon(), metrics::LIST_ICON, tint),
        container(
            ellipsis(crate::pathbar::visible(&entry.name))
                .font(typography::UI_MEDIUM)
                .width(Length::Fill)
        )
        .width(Length::Fill),
    ]
    .spacing(metrics::GAP_ICON)
    .align_y(Alignment::Center);
    if entry.kind == Kind::Folder {
        content = content.push(ui::icon(
            Icon::Chevron,
            metrics::ICON_SIZE,
            color::palette().text_tertiary,
        ));
    }
    let body = container(content)
        .width(Length::Fill)
        .height(metrics::LIST_ROW_HEIGHT)
        .padding([0.0, metrics::LIST_ROW_PADDING])
        .align_y(Alignment::Center)
        .style(theme::list_row(lit));
    let body = hoverable(body, color::palette().hover, metrics::LIST_ROW_RADIUS);
    mouse_area(body).on_press(on_press).into()
}

/// A column other than the folder shown: its lines, lit on the way down, in a scroll of its
/// own.
fn side_column<'a>(
    column: &'a DirColumn,
    lit: impl Fn(&Entry) -> bool,
    on_press: impl Fn(usize) -> Message,
) -> Element<'a, Message> {
    let lines = Column::with_children(
        column
            .entries
            .iter()
            .enumerate()
            .map(|(i, entry)| line(entry, lit(entry), on_press(i))),
    )
    .padding([metrics::LIST_TOP_GAP, 0.0]);
    scroll(lines).height(Length::Fill).into()
}

impl App {
    /// The columns' headers, where the list has its own: each folder's name.
    pub(super) fn columns_header(&self) -> Element<'_, Message> {
        let mut names = Row::new();
        let shown = match &self.location {
            Location::Folder(dir) => Some(dir.as_path()),
            Location::Trash => None,
        };
        for dir in self
            .column_dirs
            .iter()
            .map(|c| c.dir.as_path())
            .chain(shown)
        {
            names = names.push(
                container(
                    ellipsis(folder_name(dir))
                        .font(typography::UI_SEMIBOLD)
                        .color(color::palette().text_secondary),
                )
                .width(more::COLUMN_WIDTH)
                .padding([0.0, metrics::LIST_ROW_PADDING]),
            );
        }
        container(names)
            .height(metrics::LIST_HEADER_HEIGHT)
            .align_y(Alignment::Center)
            .into()
    }

    /// The columns, side by side.
    pub(super) fn columns_view(&self, now: Duration, slide: f32) -> Element<'_, Message> {
        let rule = || {
            container(space().width(metrics::BORDER_WIDTH).height(Length::Fill))
                .style(theme::separator)
        };
        let mut columns = Row::new().height(Length::Fill);
        let way: Vec<&std::path::Path> = match &self.location {
            Location::Folder(dir) => dir.ancestors().collect(),
            Location::Trash => Vec::new(),
        };
        for (depth, column) in self.column_dirs.iter().enumerate() {
            let lit = |e: &Entry| way.contains(&e.path.as_path());
            let side = side_column(column, lit, move |i| Message::ColumnClick(depth, i));
            columns = columns
                .push(container(side).width(more::COLUMN_WIDTH - metrics::BORDER_WIDTH))
                .push(rule());
        }
        let shown: Element<'_, Message> = if self.entries.is_empty() {
            container(ui::description(t::EMPTY_FOLDER))
                .center(Length::Fill)
                .into()
        } else {
            scroll(self.shown_column(slide))
                .id(LIST)
                .on_scroll(|v| Message::Scrolled(v.absolute_offset().y, v.bounds().height))
                .height(Length::Fill)
                .into()
        };
        columns = columns
            .push(container(shown).width(more::COLUMN_WIDTH - metrics::BORDER_WIDTH))
            .push(rule());
        // On the right: what the folder selected holds, or the file's preview.
        let right: Element<'_, Message> = match (&self.peek, self.single()) {
            (Some(peek), _) => side_column(peek, |_| false, Message::PeekClick),
            (None, Some(_)) => self.preview_panel(now),
            (None, None) => space().into(),
        };
        columns.push(container(right).width(Length::Fill)).into()
    }

    /// The folder shown, as a column: only the lines in sight, as the list's rows.
    fn shown_column(&self, slide: f32) -> Element<'_, Message> {
        let height = metrics::LIST_ROW_HEIGHT;
        let total = self.entries.len();
        let (offset, _) = self.scroll;
        let first = ((offset - metrics::LIST_TOP_GAP) / height).floor().max(0.0) as usize;
        let first = first.saturating_sub(metrics::LIST_OVERSCAN).min(total);
        let last = (((offset + self.size.height) / height).ceil() as usize
            + metrics::LIST_OVERSCAN)
            .min(total);
        Column::new()
            .push(space().height(metrics::LIST_TOP_GAP + first as f32 * height + slide))
            .extend((first..last).map(|i| {
                let entry = &self.entries[i];
                let selected = self.selection.contains(i);
                let body = match &self.renaming {
                    Some((path, value)) if *path == entry.path => container(
                        ui::text_field("", value, Some(Message::RenameInput))
                            .id(crate::app::RENAME)
                            .on_submit(Message::RenameSubmit),
                    )
                    .height(height)
                    .align_y(Alignment::Center)
                    .into(),
                    _ => line(entry, selected, Message::Click(i)),
                };
                mouse_area(body)
                    .on_right_press(Message::RightClick(Some(i)))
                    .on_middle_press(Message::MiddleClick(i))
                    .into()
            }))
            .push(space().height((total - last) as f32 * height))
            .width(Length::Fill)
            .into()
    }
}
