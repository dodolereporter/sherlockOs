//! Launcher state, messages, update and view.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use iced::keyboard::{self, key::Named};
use iced::widget::{
    Space, button, column, container, mouse_area, operation, row, svg, text, text_input,
};
use iced::window::Id;
use iced::{Alignment, Element, Event, Length, Subscription, Task, event};
use iced_exwlshell::to_layer_message;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::dock::text as dock_texts;
use sherlock_ui::icon::Icon;
use sherlock_ui::launcher::{self as look, Glyph, text as texts};
use sherlock_ui::logo::Logo;
use sherlock_ui::{color, metrics, typography};

use crate::source::{self, Action, Command, Hit, Section, Source, Tile};
use crate::system::{NoSession, Session};

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
    /// A source has more to say (the files, once walked): search again.
    Refresh,
    /// A right click on an application, by its place in the list: its menu.
    Menu(usize),
    /// « Garder dans le Dock » or « Retirer du Dock » in that menu.
    Pin,
}

/// What starts a program: `launch::spawn`, or a recorder in tests.
pub type Spawn = Arc<dyn Fn(&Command) -> Result<u32, crate::launch::LaunchError> + Send + Sync>;

/// How the calculator's result is copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clipboard {
    /// `wl-copy`: it keeps offering the text once the launcher is closed, so the launcher
    /// closes as after any start.
    WlCopy(OsString),
    /// iced's clipboard: a Wayland selection lives as long as its program, so the launcher
    /// stays open, the result marked « Copié », until Escape.
    Toolkit,
}

impl Clipboard {
    /// `wl-copy` when it is installed.
    pub fn detect() -> Self {
        if mime_apps::entry::is_executable("wl-copy") {
            Self::WlCopy("wl-copy".into())
        } else {
            Self::Toolkit
        }
    }
}

pub struct Launcher {
    sources: Vec<Box<dyn Source>>,
    spawn: Spawn,
    session: Arc<dyn Session>,
    clipboard: Clipboard,
    /// The result waiting for its confirmation (« Se déconnecter », « Redémarrer »,
    /// « Éteindre »), and when it was armed.
    armed: Option<(usize, Instant)>,
    /// The result just copied with iced's clipboard.
    pub copied: Option<usize>,
    pub query: String,
    pub sections: Vec<Section>,
    /// The chosen result, in the order of the list, sections one after the other.
    pub selected: usize,
    surface: Option<Id>,
    /// Asked to close: after a start, or on Escape.
    pub closing: bool,
    /// The application whose menu is open, by its place in the list, and whether it is pinned.
    pub menu: Option<(usize, bool)>,
    /// The dock's configuration file, where the menu pins.
    dock_file: Option<std::path::PathBuf>,
}

impl Launcher {
    pub fn new(sources: Vec<Box<dyn Source>>, spawn: Spawn) -> Self {
        Self {
            sources,
            spawn,
            session: Arc::new(NoSession),
            clipboard: Clipboard::Toolkit,
            armed: None,
            copied: None,
            query: String::new(),
            sections: Vec::new(),
            selected: 0,
            surface: None,
            closing: false,
            menu: None,
            dock_file: dock_config::path(),
        }
    }

    /// Pins to the dock in the file at `path` instead of the user's (the tests).
    pub fn with_dock_file(mut self, path: std::path::PathBuf) -> Self {
        self.dock_file = Some(path);
        self
    }

    /// Runs the system commands with `session`; without one, they are refused.
    pub fn with_session(mut self, session: Arc<dyn Session>) -> Self {
        self.session = session;
        self
    }

    pub fn with_clipboard(mut self, clipboard: Clipboard) -> Self {
        self.clipboard = clipboard;
        self
    }

    /// The result waiting for its confirmation.
    pub fn armed(&self) -> Option<usize> {
        self.armed.map(|(index, _)| index)
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
        // Anything but its own entry closes the menu; Escape closes it alone.
        let menu = self.menu.take();
        match message {
            Message::Menu(index) => {
                let id = self.hits().nth(index).and_then(|h| h.app.clone());
                if let (Some(id), Some(path)) = (id, self.dock_file.as_deref()) {
                    self.menu = Some((index, dock::pins::is_pinned(path, &id)));
                }
                return Task::none();
            }
            Message::Pin => {
                if let Some((index, pinned)) = menu {
                    self.pin(index, !pinned);
                }
                return Task::none();
            }
            Message::Key(Key::Escape) if menu.is_some() => return Task::none(),
            _ => {}
        }
        match message {
            Message::Surface(id) => {
                self.surface = Some(id);
                return operation::focus(QUERY);
            }
            Message::Query(query) => {
                let before = self.height();
                self.query = query;
                self.selected = 0;
                return self.search(before);
            }
            Message::Refresh => {
                let task = self.search(self.height());
                self.selected = self.selected.min(self.hits().count().saturating_sub(1));
                return task;
            }
            Message::Key(Key::Down) => {
                let last = self.hits().count().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                self.armed = None;
            }
            Message::Key(Key::Up) => {
                self.selected = self.selected.saturating_sub(1);
                self.armed = None;
            }
            Message::Key(Key::Enter) => return self.choose(self.selected),
            Message::Key(Key::Escape) => return self.close(),
            Message::Pick(index) => return self.choose(index),
            _ => {}
        }
        Task::none()
    }

    /// Asks the sources again for the query; the panel follows the number of rows, `before`
    /// being its height as drawn.
    fn search(&mut self, before: f32) -> Task<Message> {
        self.sections = source::search(
            &self.sources,
            &self.query,
            look::SECTION_RESULTS,
            look::MAX_RESULTS,
        );
        self.armed = None;
        self.copied = None;
        if self.height() != before {
            return self.resize();
        }
        Task::none()
    }

    /// Pins result `index` to the dock, or unpins it: the dock reads its file again at once.
    fn pin(&self, index: usize, pinned: bool) {
        let id = self.hits().nth(index).and_then(|h| h.app.clone());
        let (Some(id), Some(path)) = (id, self.dock_file.as_deref()) else {
            return;
        };
        match dock::pins::set(path, &id, pinned) {
            Ok(_) => tracing::info!(id, pinned, "dock"),
            Err(error) => tracing::error!(%error, "cannot write the dock's file"),
        }
    }

    fn close(&mut self) -> Task<Message> {
        self.closing = true;
        iced::exit()
    }

    /// Runs result `index`; the launcher closes once it is done.
    fn choose(&mut self, index: usize) -> Task<Message> {
        // A second Enter, or Enter then a click, before the exit is handled starts nothing.
        if self.closing {
            return Task::none();
        }
        let Some(action) = self.hits().nth(index).map(|h| h.action.clone()) else {
            return Task::none();
        };
        match action {
            Action::Start(command) => self.start(&command),
            Action::Copy(value) => self.copy(index, value),
            Action::Session(power) if power.needs_confirmation() => match self.armed {
                // Confirmed by a new press, not too soon: a double click or a key bounce
                // arms, never confirms (a held Enter never gets here, see `on_event`).
                Some((armed, at)) if armed == index && at.elapsed() >= look::CONFIRM_DELAY => {
                    self.run_session(power)
                }
                Some((armed, _)) if armed == index => Task::none(),
                _ => {
                    self.armed = Some((index, Instant::now()));
                    Task::none()
                }
            },
            Action::Session(power) => self.run_session(power),
            Action::Disabled => Task::none(),
        }
    }

    fn run_session(&mut self, power: crate::source::Power) -> Task<Message> {
        match self.session.run(power) {
            Ok(()) => {
                tracing::info!(?power, "session command");
                self.close()
            }
            Err(error) => {
                tracing::error!(%error, "cannot run the session command");
                self.armed = None;
                Task::none()
            }
        }
    }

    fn copy(&mut self, index: usize, value: String) -> Task<Message> {
        match &self.clipboard {
            Clipboard::WlCopy(program) => {
                // `--`: the value is never read as an option.
                let command = Command {
                    program: program.clone(),
                    args: vec!["--".into(), value.into()],
                    dir: None,
                };
                self.start(&command)
            }
            Clipboard::Toolkit => {
                self.copied = Some(index);
                iced::clipboard::write(value)
            }
        }
    }

    fn start(&mut self, command: &Command) -> Task<Message> {
        match (self.spawn)(command) {
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

    /// The line under a result: its own, or what its first Enter or copy did.
    fn subtitle<'a>(&self, hit: &'a Hit, index: usize) -> Option<&'a str> {
        if self.armed() == Some(index) {
            Some(texts::CONFIRM)
        } else if self.copied == Some(index) {
            Some(texts::COPIED)
        } else {
            hit.subtitle.as_deref()
        }
    }

    fn view_hit<'a>(&self, hit: &'a Hit, index: usize) -> Element<'a, Message> {
        let selected = index == self.selected;
        // « Demander à Sherlock » reads as any other result: Sherlock's tile tells it apart, the
        // cream row of the board stood out too much on the glass (UI-03).
        let sherlock = hit.tile == Tile::Sherlock;
        let p = color::palette();
        let (title_font, title_color, line_color) = if sherlock {
            (typography::UI_SEMIBOLD, p.text_primary, p.text_secondary)
        } else {
            (typography::UI_MEDIUM, p.text_primary, p.text_secondary)
        };
        let mut words = column![
            ui::body(hit.title.as_str())
                .font(title_font)
                .color(title_color)
        ];
        if let Some(subtitle) = self.subtitle(hit, index) {
            words = words.push(
                text(subtitle)
                    .size(typography::SMALL_SIZE)
                    .color(line_color),
            );
        }
        let mut line = row![
            tile(&hit.tile, &hit.title),
            words,
            Space::new().width(Length::Fill)
        ]
        .spacing(look::GAP)
        .align_y(Alignment::Center);
        if let Some((_, pinned)) = self.menu.filter(|(at, _)| *at == index) {
            // The menu, inside the row on its right: it never leaves the panel.
            let label = if pinned {
                dock_texts::UNPIN
            } else {
                dock_texts::KEEP
            };
            let item = ui::menu_item(label, Trailing::None, Some(Message::Pin));
            line = line.push(ui::floating_menu([item.into()], look::MENU_WIDTH));
        } else if selected && hit.action != Action::Disabled {
            line = line.push(key_cap(texts::ENTER, true));
        }
        let row = button(line)
            .on_press(Message::Pick(index))
            .width(Length::Fill)
            .height(look::ROW_HEIGHT)
            .padding([0.0, look::ROW_PADDING]);
        let row: Element<'a, Message> = row.style(look::row(selected)).into();
        if hit.app.is_some() {
            mouse_area(row).on_right_press(Message::Menu(index)).into()
        } else {
            row
        }
    }
}

/// The tile left of a result: an application's icon or initial, a glyph, or Sherlock.
fn tile<'a>(tile: &Tile, title: &str) -> Element<'a, Message> {
    match tile {
        // In a box of the tile's size, as every other tile: icons centred, titles aligned (UI-03).
        Tile::App(Some(path)) => container(
            svg(icon_handle(path))
                .width(look::APP_ICON)
                .height(look::APP_ICON),
        )
        .center(look::TILE)
        .into(),
        Tile::App(None) => {
            let initial = title
                .chars()
                .next()
                .and_then(|c| c.to_uppercase().next())
                .unwrap_or('?');
            container(
                text(initial.to_string())
                    .font(typography::UI_SEMIBOLD)
                    .size(look::TILE_LETTER_SIZE),
            )
            .center(look::TILE)
            .style(look::tile)
            .into()
        }
        Tile::Glyph(glyph) => {
            let stroke = look::Colors::of(color::palette()).glyph_tile(*glyph).1;
            container(
                svg(glyph_handle(*glyph))
                    .width(look::TILE_GLYPH)
                    .height(look::TILE_GLYPH)
                    .style(move |_, _| svg::Style {
                        color: Some(stroke.into()),
                    }),
            )
            .center(look::TILE)
            .style(look::glyph_tile(*glyph))
            .into()
        }
        Tile::Sherlock => container(ui::logo(Logo::Color, look::TILE))
            .center(look::TILE)
            .into(),
    }
}

/// The SVG of a glyph, built once.
fn glyph_handle(glyph: Glyph) -> svg::Handle {
    static HANDLES: std::sync::OnceLock<Vec<svg::Handle>> = std::sync::OnceLock::new();
    let all = HANDLES.get_or_init(|| {
        Glyph::ALL
            .iter()
            .map(|g| svg::Handle::from_memory(g.svg().into_bytes()))
            .collect()
    });
    Glyph::ALL
        .iter()
        .position(|g| *g == glyph)
        .and_then(|i| all.get(i))
        .cloned()
        .unwrap_or_else(|| svg::Handle::from_memory(glyph.svg().into_bytes()))
}

/// An SVG icon as it is; a PNG inside a small SVG, which the SVG renderer reads by path, as
/// the dock does: iced's `image` feature would bring its own decoders.
pub fn icon_handle(path: &Path) -> svg::Handle {
    if path.extension().is_some_and(|e| e == "svg") {
        return svg::Handle::from_path(path);
    }
    let href = path
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;");
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"{href}\"/></svg>"
    );
    svg::Handle::from_memory(document.into_bytes())
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

/// The arrows, Enter and Escape, even when the search field took them. A held Enter repeats
/// nothing: every Enter is a new press, so a key held down never confirms a session command.
pub fn on_event(event: Event, _: event::Status, _: Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, repeat, .. }) = event else {
        return None;
    };
    let key = match key {
        keyboard::Key::Named(Named::ArrowUp) => Key::Up,
        keyboard::Key::Named(Named::ArrowDown) => Key::Down,
        keyboard::Key::Named(Named::Enter) if !repeat => Key::Enter,
        keyboard::Key::Named(Named::Escape) => Key::Escape,
        _ => return None,
    };
    Some(Message::Key(key))
}
