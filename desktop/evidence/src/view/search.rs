//! The bar of the search's tokens (lot E-15), under the toolbar while searching: « Type »,
//! « Date » and « Taille », each opening its menu, where the search looks, and a way to
//! forget them all.

use iced::widget::{Row, column, container, space};
use iced::{Alignment, Element, Length};

use sherlock_ui::component as ui;
use sherlock_ui::evidence as more;
use sherlock_ui::metrics;
use sherlock_ui::texts::evidence as t;

use crate::app::{App, Message, Token};
use crate::filter::Scope;

impl App {
    pub(super) fn filter_bar(&self) -> Element<'_, Message> {
        if !self.filter_bar_shown() {
            return space().height(0).into();
        }
        let filters = &self.filters;
        let tokens = [
            (Token::Kind, !filters.kinds.is_empty()),
            (Token::Date, filters.date.is_some()),
            (Token::Size, filters.size.is_some()),
        ];
        let mut line = Row::new()
            .spacing(metrics::GAP_TIGHT)
            .align_y(Alignment::Center);
        for (token, set) in tokens {
            line = line.push(ui::chip(
                self.token_label(token),
                set,
                Some(Message::TokenMenu(token)),
            ));
        }
        if filters.any() {
            line = line.push(ui::chip(
                t::FILTER_CLEAR,
                false,
                Some(Message::ClearFilters),
            ));
        }
        let scope = ui::segmented(vec![
            (
                t::SCOPE_FOLDER,
                filters.scope == Scope::Folder,
                Some(Message::SetScope(Scope::Folder)),
            ),
            (
                t::SCOPE_SUBFOLDERS,
                filters.scope == Scope::Subfolders,
                Some(Message::SetScope(Scope::Subfolders)),
            ),
        ]);
        let line = line
            .push(space().width(Length::Fill))
            .push(container(scope).width(more::SCOPE_WIDTH));
        column![
            container(line)
                .height(more::FILTER_BAR_HEIGHT)
                .padding([0.0, metrics::TOOLBAR_PADDING])
                .align_y(Alignment::Center),
            ui::divider(),
        ]
        .into()
    }
}
