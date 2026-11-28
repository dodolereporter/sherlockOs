//! The gallery (lot E-13c), as in the Finder: the item selected large, with its name and what
//! it is, over a strip of every item, which scrolls sideways and follows the arrows.

use iced::widget::{Row, column, container, image, mouse_area, space};
use iced::{Alignment, Element, Length};

use sherlock_ui::animated::hoverable;
use sherlock_ui::component as ui;
use sherlock_ui::ellipsis::ellipsis;
use sherlock_ui::evidence as more;
use sherlock_ui::{color, metrics, theme, typography};

use crate::app::{App, Message, STRIP};
use crate::listing::Kind;
use crate::preview::Preview;

impl App {
    /// The item selected large over the strip.
    pub(super) fn gallery_view(&self) -> Element<'_, Message> {
        let stage: Element<'_, Message> = match self.single() {
            Some(entry) => {
                let tint = match entry.kind {
                    Kind::Folder => color::palette().blue_light,
                    _ => color::palette().text_secondary,
                };
                let picture: Element<'_, Message> = match &self.preview {
                    Preview::Image(handle) => image(handle.clone())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .content_fit(iced::ContentFit::Contain)
                        .into(),
                    Preview::Text(lines) => container(ui::code_block(lines))
                        .max_width(more::GALLERY_TEXT_WIDTH)
                        .into(),
                    Preview::Info => ui::icon(entry.icon(), more::GALLERY_ICON, tint).into(),
                };
                let caption = column![
                    ellipsis(crate::pathbar::visible(&entry.name)).font(typography::UI_SEMIBOLD),
                    ui::description(entry.labels.kind.clone()),
                ]
                .align_x(Alignment::Center);
                column![
                    container(picture).center(Length::Fill),
                    container(caption).center_x(Length::Fill),
                ]
                .spacing(metrics::PREVIEW_GAP)
                .padding(metrics::PANEL_PADDING)
                .into()
            }
            None => space().width(Length::Fill).height(Length::Fill).into(),
        };
        // Only the tiles in sight, between two spacers, as the list's rows.
        let (first, last) = self.strip_range();
        let step = more::GALLERY_TILE + more::GRID_GAP;
        let tiles = (first..last).map(|i| {
            let entry = &self.entries[i];
            let tint = match entry.kind {
                Kind::Folder => color::palette().blue_light,
                _ => color::palette().text_secondary,
            };
            let tile = container(ui::icon(entry.icon(), more::GALLERY_STRIP_ICON, tint))
                .center(more::GALLERY_TILE)
                .style(theme::list_row(self.selection.contains(i)));
            let tile =
                hoverable(tile, color::palette().hover, metrics::LIST_ROW_RADIUS).key(i as u64);
            mouse_area(tile)
                .on_press(Message::Click(i))
                .on_right_press(Message::RightClick(Some(i)))
                .on_middle_press(Message::MiddleClick(i))
                .into()
        });
        let strip = Row::new()
            .push(space().width(first as f32 * step))
            .extend(tiles)
            .push(space().width((self.entries.len() - last) as f32 * step))
            .spacing(more::GRID_GAP)
            .padding([more::GALLERY_STRIP_PADDING, 0.0]);
        // Sideways: iced's own scrollable, `sherlock_ui::scroll` only goes down.
        let strip = iced::widget::scrollable(strip)
            .id(STRIP)
            // The wheel and the arrows scroll it; no bar, as the Finder's strip.
            .direction(iced::widget::scrollable::Direction::Horizontal(
                iced::widget::scrollable::Scrollbar::hidden(),
            ))
            .on_scroll(|v| Message::StripScrolled(v.absolute_offset().x))
            .width(Length::Fill)
            .height(more::GALLERY_STRIP);
        column![stage, ui::divider(), strip]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}
