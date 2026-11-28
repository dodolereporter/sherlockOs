//! The grid of icons (lot E-13): tiles of an icon over its name, as many on a line as the
//! listing's width holds, at the size of the slider above them. Only the lines in sight are
//! built, as the rows of the list are.

use iced::widget::{Column, Row, column, container, mouse_area, row, space};
use iced::{Alignment, Element, Length};

use sherlock_ui::animated::hoverable;
use sherlock_ui::component as ui;
use sherlock_ui::ellipsis::ellipsis;
use sherlock_ui::evidence as more;
use sherlock_ui::{color, metrics, theme, typography};

use crate::app::{App, Message, RENAME};
use crate::listing::{Entry, Kind};

impl App {
    /// Above the grid, where the list has its column headers: the slider of the icons' size.
    pub(super) fn grid_header(&self) -> Element<'_, Message> {
        let slider = ui::held_slider(
            self.icon_slider(),
            Some(Message::IconSize),
            Some(Message::IconSizeDone),
        );
        let line = row![
            space().width(Length::Fill),
            ui::description(more::ICON_SIZE),
            container(slider).width(more::ICON_SLIDER_WIDTH),
        ]
        .spacing(metrics::GAP_ICON)
        .align_y(Alignment::Center);
        container(line)
            .height(metrics::LIST_HEADER_HEIGHT)
            .padding([0.0, metrics::LIST_ROW_PADDING])
            .align_y(Alignment::Center)
            .into()
    }

    /// The lines of tiles in sight, between two spacers that hold the place of the others.
    pub(super) fn tiles(&self, slide: f32) -> Element<'_, Message> {
        let total = self.entries.len();
        let columns = self.grid_columns();
        let (_, line) = self.grid_line(0);
        let lines = total.div_ceil(columns);
        let (offset, _) = self.scroll;
        let first = ((offset - metrics::LIST_TOP_GAP) / line).floor().max(0.0) as usize;
        let first = first.saturating_sub(1).min(lines);
        let last = (((offset + self.size.height) / line).ceil() as usize + 1).min(lines);
        let before = metrics::LIST_TOP_GAP + first as f32 * line + slide;
        let after = (lines - last) as f32 * line;
        let built = (first..last).map(|l| {
            let tiles = (l * columns..((l + 1) * columns).min(total))
                .map(|i| self.tile(i, &self.entries[i]));
            let tiles = Row::with_children(tiles).spacing(more::GRID_GAP);
            container(tiles).height(line).into()
        });
        let grid = Column::new()
            .push(space().height(before))
            .extend(built)
            .push(space().height(after))
            .width(Length::Fill);
        // The lasso, while it is dragged, over the tiles.
        match self.lasso.as_ref().filter(|l| l.dragging) {
            Some(lasso) => {
                let (x, y, w, h) = lasso.rect();
                let rect = container(space())
                    .width(w)
                    .height(h)
                    .style(sherlock_ui::evidence::lasso);
                let layer = column![
                    space().height(y.max(0.0) + slide),
                    row![space().width(x.max(0.0)), rect]
                ];
                iced::widget::stack![grid, layer].width(Length::Fill).into()
            }
            None => grid.into(),
        }
    }

    fn tile<'a>(&'a self, i: usize, entry: &'a Entry) -> Element<'a, Message> {
        let selected = self.selection.contains(i);
        let size = self.view_memory.icon_size;
        let (width, height) = self.tile_size();
        let tint = match entry.kind {
            Kind::Folder => color::palette().blue_light,
            _ => color::palette().text_secondary,
        };
        let name: Element<'a, Message> = match &self.renaming {
            Some((path, value)) if *path == entry.path => {
                ui::text_field("", value, Some(Message::RenameInput))
                    .id(RENAME)
                    .on_submit(Message::RenameSubmit)
                    .into()
            }
            _ => container(
                ellipsis(crate::pathbar::visible(&entry.name)).font(typography::UI_MEDIUM),
            )
            .center_x(Length::Fill)
            .into(),
        };
        let body = column![
            container(ui::icon(entry.icon(), size, tint)).center_x(Length::Fill),
            name
        ]
        .spacing(metrics::GAP_TIGHT)
        .align_x(Alignment::Center);
        let tile = container(body)
            .width(width)
            .height(height)
            .padding(more::GRID_TILE_PADDING)
            .style(theme::list_row(selected));
        // The folder a drop would land in lights up (lot E-03).
        let tile: Element<'a, Message> = match self.drop_light() {
            Some((Some(target), light)) if target == i => container(tile)
                .style(sherlock_ui::drag::target(light))
                .into(),
            _ => tile.into(),
        };
        let tile = hoverable(tile, color::palette().hover, metrics::LIST_ROW_RADIUS).key(i as u64);
        mouse_area(tile)
            .on_press(Message::Click(i))
            .on_right_press(Message::RightClick(Some(i)))
            .on_middle_press(Message::MiddleClick(i))
            .into()
    }
}
