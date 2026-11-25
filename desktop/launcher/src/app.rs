//! Launcher state, messages, update and view.

use std::sync::Arc;

use iced::keyboard::{self, key::Named};
use iced::widget::{Space, button, column, container, operation, row, text, text_input};
use iced::window::Id;
use iced::{Alignment, Element, Event, Length, Subscription, Task, event};
use iced_exwlshell::to_layer_message;
use sherlock_ui::component::{self as ui};
use sherlock_ui::icon::Icon;
use sherlock_ui::launcher::{self as look, text as texts};
use sherlock_ui::{color, metrics, typography};

use crate::source::{self, Action, Command, Hit, Section, Source};

/// Id of the search field, to give it the keyboard.
pub const QUERY: &str = "query";

/// The keys the launcher acts on, whatever has the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Escape,
}

#[to_layer_message(multi)]
#[derive(Debug, Clone)]
pub enum Message {
    Surface(Id),
    Query(String),
    Key(Key),
    /// A result clicked, by its place in the list.
    Pick(usize),
}

/// What starts a program: `launch::spawn`, or a recorder in tests.
pub type Spawn = Arc<dyn Fn(&Command) -> Result<u32, crate::launch::LaunchError> + Send + Sync>;

pub struct Launcher {
    sources: Vec<Box<dyn Source>>,
    spawn: Spawn,
    pub query: String,
    pub sections: Vec<Section>,
    /// The chosen result, in the order of the list, sections one after the other.
    pub selected: usize,
    surface: Option<Id>,
    /// Asked to close: after a start, or on Escape.
    pub closing: bool,
}

impl Launcher {
    pub fn new(sources: Vec<Box<dyn Source>>, spawn: Spawn) -> Self {
        Self {
            sources,
            spawn,
            query: String::new(),
            sections: Vec::new(),
            selected: 0,
            surface: None,
            closing: false,
        }
    }

    /// Every result, in the order of the list.
    pub fn hits(&self) -> impl Iterator<Item = &Hit> {
        self.sections.iter().flat_map(|s| s.hits.iter())
    }

    /// Height of the panel as it is now: the search row, then the sections, or one line
    /// saying nothing matches.
    pub fn height(&self) -> f32 {
        let rows = self.hits().count();
        if rows == 0 && !self.query.trim().is_empty() {
            return look::panel_height(0, 1);
        }
        look::panel_height(self.sections.len(), rows)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(on_event)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(id) => {
                self.surface = Some(id);
                return operation::focus(QUERY);
            }
            Message::Query(query) => {
                let before = self.height();
                self.sections = source::search(&self.sources, &query, look::MAX_RESULTS);
                self.query = query;
                self.selected = 0;
                if self.height() != before {
                    return self.resize();
                }
            }
            Message::Key(Key::Down) => {
                let last = self.hits().count().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
            }
            Message::Key(Key::Up) => self.selected = self.selected.saturating_sub(1),
            Message::Key(Key::Enter) => return self.choose(self.selected),
            Message::Key(Key::Escape) => return self.close(),
            Message::Pick(index) => return self.choose(index),
            _ => {}
        }
        Task::none()
    }

    fn close(&mut self) -> Task<Message> {
        self.closing = true;
        iced::exit()
    }

    /// Runs result `index`; the launcher closes once the program started.
    fn choose(&mut self, index: usize) -> Task<Message> {
        // A second Enter, or Enter then a click, before the exit is handled starts nothing.
        if self.closing {
            return Task::none();
        }
        let Some(Action::Start(command)) = self.hits().nth(index).map(|h| h.action.clone()) else {
            return Task::none();
        };
        match (self.spawn)(&command) {
            Ok(pid) => {
                tracing::info!(program = ?command.program, pid, "started");
                self.close()
            }
            Err(error) => {
                tracing::error!(%error, "cannot start the result");
                Task::none()
            }
        }
    }

    /// Tells the compositor the new height of the surface.
    fn resize(&self) -> Task<Message> {
        let Some(id) = self.surface else {
            return Task::none();
        };
        let settings = crate::surface(self.height());
        Task::done(Message::LayoutChange {
            id,
            anchor: settings.anchor,
            size: settings.size,
        })
    }

    pub fn view(&self, _: Id) -> Element<'_, Message> {
        self.view_panel()
    }

    pub fn view_panel(&self) -> Element<'_, Message> {
        let p = color::palette();
        let field = text_input(texts::PLACEHOLDER, &self.query)
            .id(QUERY)
            .on_input(Message::Query)
            .size(look::QUERY_SIZE)
            .padding(0)
            .style(look::query);
        let search = row![
            ui::icon(Icon::Search, look::SEARCH_ICON, p.accent_text),
            field,
            key_cap(texts::ESCAPE, false),
        ]
        .spacing(look::GAP)
        .align_y(Alignment::Center)
        .padding([0.0, look::SEARCH_PADDING])
        .height(look::SEARCH_HEIGHT);
        let divider = container(Space::new())
            .width(Length::Fill)
            .height(metrics::BORDER_WIDTH)
            .style(look::divider);
        let mut list = column![search, divider];
        let mut index = 0;
        for section in &self.sections {
            let (top, side, bottom) = look::SECTION_PADDING;
            let label = container(ui::section_label(section.title)).padding(iced::Padding {
                top,
                right: side,
                bottom,
                left: side,
            });
            list = list.push(label);
            for hit in &section.hits {
                list = list.push(self.view_hit(hit, index));
                index += 1;
            }
        }
        if index == 0 && !self.query.trim().is_empty() {
            let nothing = ui::body(texts::NOTHING).color(p.text_secondary);
            list = list.push(
                container(nothing)
                    .height(look::ROW_HEIGHT)
                    .padding([0.0, look::ROW_PADDING])
                    .center_y(look::ROW_HEIGHT),
            );
        }
        container(list)
            .width(look::WIDTH)
            .height(self.height())
            .padding(look::PADDING + metrics::BORDER_WIDTH)
            .style(look::panel)
            .into()
    }

    fn view_hit<'a>(&self, hit: &'a Hit, index: usize) -> Element<'a, Message> {
        let selected = index == self.selected;
        let initial = hit
            .title
            .chars()
            .next()
            .and_then(|c| c.to_uppercase().next())
            .unwrap_or('?');
        let tile = container(
            text(initial.to_string())
                .font(typography::UI_SEMIBOLD)
                .size(look::TILE_LETTER_SIZE),
        )
        .center(look::TILE)
        .style(look::tile);
        let mut words = column![ui::body(hit.title.as_str()).font(typography::UI_MEDIUM)];
        if let Some(subtitle) = &hit.subtitle {
            words = words.push(
                text(subtitle.as_str())
                    .size(typography::SMALL_SIZE)
                    .color(color::palette().text_secondary),
            );
        }
        let mut line = row![tile, words, Space::new().width(Length::Fill)]
            .spacing(look::GAP)
            .align_y(Alignment::Center);
        if selected {
            line = line.push(key_cap(texts::ENTER, true));
        }
        button(line)
            .on_press(Message::Pick(index))
            .width(Length::Fill)
            .height(look::ROW_HEIGHT)
            .padding([0.0, look::ROW_PADDING])
            .style(look::row(selected))
            .into()
    }
}

/// A key name in its cap, on the search row or on the selected result (`in_row`).
fn key_cap<'a>(name: &'a str, in_row: bool) -> Element<'a, Message> {
    let (vertical, side) = if in_row {
        look::KEY_PADDING_ROW
    } else {
        look::KEY_PADDING
    };
    container(text(name).size(look::KEY_SIZE))
        .padding([vertical, side])
        .style(look::key(in_row))
        .into()
}

/// The arrows, Enter and Escape, even when the search field took them.
pub fn on_event(event: Event, _: event::Status, _: Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    let key = match key {
        keyboard::Key::Named(Named::ArrowUp) => Key::Up,
        keyboard::Key::Named(Named::ArrowDown) => Key::Down,
        keyboard::Key::Named(Named::Enter) => Key::Enter,
        keyboard::Key::Named(Named::Escape) => Key::Escape,
        _ => return None,
    };
    Some(Message::Key(key))
}
