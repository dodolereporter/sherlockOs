//! The Keyboard page: the layout, how a held key repeats, a field to try them, and the
//! shortcuts of the desktop (`keyboard.conf`, read live by the compositor, lot D-26c).

use iced::widget::{Column, row, space};
use iced::{Alignment, Element, Length};
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};
use sherlock_ui::{color, metrics, typography};

use crate::app::{App, Change, LayoutChoice, Message};
use crate::search::Setting;
use crate::settings::File;

impl App {
    pub(crate) fn keyboard(&self, width: f32) -> Vec<Element<'_, Message>> {
        let kb = &self.settings.keyboard;
        let chosen = LayoutChoice::of(kb);
        let layout = match self.can_change(File::Keyboard) {
            true => ui::dropdown(
                LayoutChoice::offered(),
                Some(chosen),
                |choice| Message::Set(Change::Layout(choice)),
                cr::LAYOUT_WIDTH,
            ),
            false => ui::description(chosen.to_string()).into(),
        };
        let delay = valued(
            ui::slider(
                self.repeat_delay(),
                self.enabled(File::Keyboard, |v| Message::Set(Change::RepeatDelay(v))),
            ),
            t::milliseconds(kb.repeat_delay),
        );
        let rate = valued(
            ui::slider(
                self.repeat_rate(),
                self.enabled(File::Keyboard, |v| Message::Set(Change::RepeatRate(v))),
            ),
            t::per_second(kb.repeat_rate),
        );
        let trial = ui::text_field(t::TRY_KEYS_PLACEHOLDER, &self.trial, Some(Message::TryKeys))
            .width(Length::Fill)
            .into();
        let mut shortcuts = vec![self.row(
            Setting::Shortcuts,
            width,
            t::SHORTCUTS_HELP,
            space().into(),
            false,
        )];
        shortcuts.extend(
            t::SHORTCUT_LIST
                .iter()
                .map(|(keys, action)| line(action, keys)),
        );
        vec![
            self.card(
                Some(t::TYPING_CARD),
                vec![
                    self.row(Setting::Layout, width, t::LAYOUT_HELP, layout, true),
                    self.row(
                        Setting::RepeatDelay,
                        width,
                        t::REPEAT_DELAY_HELP,
                        delay,
                        true,
                    ),
                    self.row(Setting::RepeatRate, width, t::REPEAT_RATE_HELP, rate, true),
                    self.try_row(width, trial),
                ],
            ),
            self.card(None, shortcuts),
        ]
    }

    /// The field where the keyboard is tried, under its name: no setting of the search.
    fn try_row<'a>(&self, width: f32, field: Element<'a, Message>) -> Element<'a, Message> {
        let label = Column::new()
            .push(ui::body(t::TRY_KEYS).font(typography::UI_MEDIUM))
            .push(
                ui::body(t::TRY_KEYS_HELP)
                    .size(typography::SMALL_SIZE)
                    .color(color::palette().text_secondary),
            )
            .width(Length::Fill);
        let line: Element<'a, Message> = if width < cr::STACK_WIDTH {
            Column::new()
                .push(label)
                .push(field)
                .spacing(metrics::GAP_ICON)
                .into()
        } else {
            let field = iced::widget::container(field).width(cr::CONTROL_WIDTH);
            row![label, field]
                .spacing(metrics::GAP_ROW)
                .align_y(Alignment::Center)
                .into()
        };
        iced::widget::container(line)
            .padding([metrics::GAP_ICON, 0.0])
            .into()
    }
}

/// A slider with its value on its right.
fn valued<'a>(slider: Element<'a, Message>, value: String) -> Element<'a, Message> {
    let value = ui::ellipsis(value)
        .size(typography::SMALL_SIZE)
        .color(color::palette().text_secondary);
    row![
        iced::widget::container(slider).width(Length::Fill),
        iced::widget::container(value).width(cr::VALUE_WIDTH),
    ]
    .spacing(metrics::GAP_ICON)
    .align_y(Alignment::Center)
    .into()
}

/// A line of a list that only tells: what is done on the left, cut with « … », how on the
/// right. The shortcuts here, the gestures that do not change on the touchpad page.
pub(crate) fn line<'a>(what: &'a str, how: &'a str) -> Element<'a, Message> {
    let p = color::palette();
    row![
        ui::ellipsis(what).color(p.text_primary).width(Length::Fill),
        ui::ellipsis(how)
            .font(typography::UI_MEDIUM)
            .color(p.text_secondary),
    ]
    .spacing(metrics::GAP_ROW)
    .align_y(Alignment::Center)
    .height(metrics::LIST_ROW_HEIGHT)
    .into()
}
