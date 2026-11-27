//! The Default applications page: a menu of the installed applications for each kind of file
//! or link, grouped as the web, work and media.

use iced::Element;
use iced::widget::container;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::metrics;

use crate::app::{App, Message};
use crate::apps::{self, Kind};
use crate::search::Setting;

impl App {
    pub(crate) fn default_apps(&self, width: f32) -> Vec<Element<'_, Message>> {
        let Some(catalog) = &self.catalog else {
            let wait =
                container(ui::description(t::APPS_LOADING)).padding([metrics::GAP_ICON, 0.0]);
            return vec![self.card(None, vec![wait.into()])];
        };
        let row = |kind: Kind| -> Element<'_, Message> {
            let (choices, chosen) = apps::choices(catalog, kind);
            let help = kind.texts().1;
            let control: Element<'_, Message> = if choices.is_empty() {
                ui::description(t::NO_APP).into()
            } else {
                ui::dropdown(
                    choices,
                    chosen,
                    move |choice| Message::SetApp(kind, choice),
                    cr::APP_MENU_WIDTH,
                )
            };
            self.row(Setting::App(kind), width, help, control, true)
        };
        let group =
            |title, kinds: &[Kind]| self.card(Some(title), kinds.iter().map(|k| row(*k)).collect());
        vec![
            group(t::APPS_FOR_LINKS, &[Kind::Browser, Kind::Mail]),
            group(
                t::APPS_FOR_WORK,
                &[Kind::Editor, Kind::Terminal, Kind::Files],
            ),
            group(t::APPS_FOR_MEDIA, &[Kind::Images, Kind::Video, Kind::Music]),
        ]
    }
}
