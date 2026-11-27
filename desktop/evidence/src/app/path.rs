//! The path bar in text mode and the « Aller au dossier » sheet (lot E-04): Ctrl+L, a click on
//! the empty part of the breadcrumb, or a typed « / » or « ~ » turn it into a field; Maj+Ctrl+G
//! opens the sheet. Folders complete as one types; Enter opens, Escape gives the breadcrumb
//! back; a path that leads nowhere shakes the field.

use std::path::PathBuf;

use iced::Task;
use iced::keyboard::{Key, Modifiers, key::Named};
use iced::widget::operation;
use sherlock_ui::evidence as more;
use sherlock_ui::motion::Fade;
use sherlock_ui::texts::evidence as texts;

use super::{App, Location, Message, Notice, PATH};
use crate::pathbar::{self, Folders, Found};

/// The field and what it offers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathEdit {
    pub text: String,
    /// In the « Aller au dossier » sheet rather than in the toolbar.
    pub sheet: bool,
    /// Subfolders that complete the name being typed.
    pub suggestions: Vec<String>,
    /// The one picked with the arrows.
    pub highlight: Option<usize>,
    /// When the field last said no.
    pub shake: Option<std::time::Duration>,
}

impl PathEdit {
    /// How far the field is pushed sideways at `now`: three swings that die out.
    pub fn shake_offset(&self, now: std::time::Duration) -> f32 {
        let Some(at) = self.shake else { return 0.0 };
        let p = now.saturating_sub(at).as_secs_f32() / more::PATH_SHAKE.as_secs_f32();
        if p >= 1.0 {
            return 0.0;
        }
        let swing = (p * more::PATH_SHAKES * std::f32::consts::TAU).sin();
        more::PATH_SHAKE_AMPLITUDE * swing * (1.0 - p)
    }
}

impl App {
    /// The folder a relative path starts from.
    fn path_base(&self) -> PathBuf {
        self.current_dir()
            .unwrap_or_else(|| self.config.home.clone())
    }

    /// The current folder as the field first shows it, with a slash to type after.
    fn path_text(&self) -> String {
        match &self.location {
            Location::Trash => String::new(),
            Location::Folder(dir) if dir.as_os_str() == "/" => "/".into(),
            Location::Folder(dir) => format!("{}/", dir.display()),
        }
    }

    /// The field, in the toolbar or in the sheet. With `typed`, it starts with that text and
    /// its cursor at the end; otherwise with the current folder, selected.
    pub(super) fn start_path_edit(&mut self, sheet: bool, typed: Option<String>) -> Task<Message> {
        self.menu = None;
        self.renaming = None;
        self.folders = None;
        let select = match typed {
            Some(_) => operation::move_cursor_to_end(PATH),
            None => operation::select_all(PATH),
        };
        let complete = typed.is_some();
        self.path_edit = Some(PathEdit {
            text: typed.unwrap_or_else(|| self.path_text()),
            sheet,
            ..PathEdit::default()
        });
        self.path_leaving = None;
        self.switch_path(1.0);
        let suggestions = if complete {
            self.complete()
        } else {
            Task::none()
        };
        Task::batch([operation::focus(PATH), select, suggestions])
    }

    /// Back to the breadcrumb, the field fading out.
    pub(super) fn close_path_edit(&mut self) {
        if let Some(edit) = self.path_edit.take() {
            self.path_leaving = Some(edit);
            self.switch_path(0.0);
            self.show_suggestions(false);
        }
    }

    fn switch_path(&mut self, to: f32) {
        let now = self.now();
        let from = self.path_fade.as_ref().map_or(1.0 - to, |f| f.value(now));
        self.path_fade = Some(Fade::new(from, to, now, more::PATH_SWITCH));
    }

    fn show_suggestions(&mut self, on: bool) {
        let now = self.now();
        let from = self.suggest_fade.as_ref().map_or(0.0, |f| f.value(now));
        let to = if on { 1.0 } else { 0.0 };
        if self.suggest_fade.as_ref().is_none_or(|f| f.target() != to) {
            self.suggest_fade = Some(Fade::new(from, to, now, more::PATH_SWITCH));
        }
    }

    /// How far the field has come in, from 0 (the breadcrumb) to 1.
    pub(crate) fn path_level(&self) -> f32 {
        let shown = if self.path_edit.is_some() { 1.0 } else { 0.0 };
        self.path_fade
            .as_ref()
            .map_or(shown, |f| f.value(self.now()))
    }

    pub(super) fn path_input(&mut self, text: String) -> Task<Message> {
        if let Some(edit) = &mut self.path_edit {
            edit.text = text;
            edit.highlight = None;
            edit.shake = None;
        }
        self.complete()
    }

    /// Fills the suggestions from the folder already read, or reads it off the interface thread.
    fn complete(&mut self) -> Task<Message> {
        let base = self.path_base();
        let home = self.config.home.clone();
        let hidden = self.show_hidden;
        let Some(edit) = &mut self.path_edit else {
            return Task::none();
        };
        let (typed, prefix) = pathbar::split(&edit.text);
        let typed = if typed.is_empty() { "." } else { &typed };
        let Some(dir) = pathbar::resolve(typed, &base, &home) else {
            return Task::none();
        };
        let task = match &self.folders {
            Some(folders) if folders.dir == dir => {
                edit.suggestions = pathbar::matches(folders, prefix, hidden);
                Task::none()
            }
            _ => {
                edit.suggestions.clear();
                self.folders_generation += 1;
                let generation = self.folders_generation;
                Task::perform(pathbar::read_folders(dir), move |folders| {
                    Message::PathFolders(generation, folders)
                })
            }
        };
        let on = self
            .path_edit
            .as_ref()
            .is_some_and(|e| !e.suggestions.is_empty());
        if on && let Some(edit) = &self.path_edit {
            self.suggestions_shown.clone_from(&edit.suggestions);
        }
        self.show_suggestions(on);
        task
    }

    pub(super) fn path_folders(
        &mut self,
        generation: u64,
        folders: Option<Folders>,
    ) -> Task<Message> {
        if generation != self.folders_generation || self.path_edit.is_none() {
            return Task::none();
        }
        match folders {
            Some(folders) => {
                self.folders = Some(folders);
                self.complete()
            }
            None => Task::none(),
        }
    }

    /// Tab: the name picked, the only one, or as far as all of them agree; then the arrows.
    fn path_tab(&mut self) -> Task<Message> {
        let Some(edit) = &mut self.path_edit else {
            return Task::none();
        };
        let (base, prefix) = pathbar::split(&edit.text);
        let pick = match (edit.highlight, edit.suggestions.as_slice()) {
            (_, []) => return Task::none(),
            (Some(i), names) => names.get(i).cloned(),
            (None, [only]) => Some(only.clone()),
            (None, names) => {
                let common = pathbar::common_prefix(names);
                if common.chars().count() > prefix.chars().count() {
                    let text = format!("{base}{common}");
                    return Task::batch([
                        self.path_input(text),
                        operation::move_cursor_to_end(PATH),
                    ]);
                }
                edit.highlight = Some(0);
                return Task::none();
            }
        };
        let Some(name) = pick else {
            return Task::none();
        };
        let text = format!("{base}{name}/");
        Task::batch([self.path_input(text), operation::move_cursor_to_end(PATH)])
    }

    /// A suggestion clicked: the field completes to it and opens it.
    pub(super) fn path_pick(&mut self, index: usize) -> Task<Message> {
        if let Some(edit) = &mut self.path_edit {
            edit.highlight = Some(index);
        }
        self.path_submit()
    }

    /// Enter: the folder picked or typed opens; a file opens in its application and is
    /// selected in its folder; nothing there shakes the field.
    pub(super) fn path_submit(&mut self) -> Task<Message> {
        let base = self.path_base();
        let Some(edit) = &mut self.path_edit else {
            return Task::none();
        };
        if let Some(name) = edit.highlight.and_then(|i| edit.suggestions.get(i)) {
            let (typed, _) = pathbar::split(&edit.text);
            edit.text = format!("{typed}{name}/");
            edit.highlight = None;
        }
        let Some(path) = pathbar::resolve(&edit.text, &base, &self.config.home) else {
            return Task::done(Message::PathResolved(PathBuf::new(), Found::Nothing));
        };
        Task::perform(pathbar::kind_of(path.clone()), move |kind| {
            Message::PathResolved(path.clone(), kind)
        })
    }

    pub(super) fn path_resolved(&mut self, path: PathBuf, found: Found) -> Task<Message> {
        if self.path_edit.is_none() {
            return Task::none();
        }
        match found {
            Found::Folder => {
                self.close_path_edit();
                self.go(Location::Folder(path))
            }
            Found::File => {
                self.close_path_edit();
                let Some(parent) = path.parent().map(PathBuf::from) else {
                    return Task::none();
                };
                let open = self.open_path(&path);
                let show = if self.location == Location::Folder(parent.clone()) {
                    self.load(std::slice::from_ref(&path))
                } else {
                    self.go_select(Location::Folder(parent), vec![path])
                };
                Task::batch([show, open])
            }
            Found::Nothing | Found::Denied => {
                let now = self.now();
                if let Some(edit) = &mut self.path_edit {
                    edit.shake = Some(now);
                }
                let text = match found {
                    Found::Denied => texts::PATH_DENIED,
                    _ => texts::PATH_NOT_FOUND,
                };
                self.notice = Some(Notice {
                    text: text.into(),
                    error: true,
                });
                Task::none()
            }
        }
    }

    /// The keys of the open field that it leaves to the application.
    pub(super) fn path_key(&mut self, key: &Key, modifiers: Modifiers) -> Task<Message> {
        let count = self.path_edit.as_ref().map_or(0, |e| e.suggestions.len());
        let step = |edit: &mut PathEdit, down: bool| {
            edit.highlight = match (edit.highlight, down) {
                _ if count == 0 => None,
                (None, true) => Some(0),
                (None, false) => Some(count - 1),
                (Some(i), true) => Some((i + 1) % count),
                (Some(i), false) => Some((i + count - 1) % count),
            };
        };
        match key.as_ref() {
            Key::Named(Named::Tab) => return self.path_tab(),
            Key::Named(Named::Escape) => self.close_path_edit(),
            Key::Named(Named::ArrowDown | Named::ArrowUp) => {
                let down = key.as_ref() == Key::Named(Named::ArrowDown);
                if let Some(edit) = &mut self.path_edit {
                    step(edit, down);
                }
            }
            Key::Character(c) if modifiers.command() && c.eq_ignore_ascii_case("l") => {
                return operation::select_all(PATH);
            }
            _ => {}
        }
        Task::none()
    }

    /// Opens a file with the default application of its type, or xdg-open.
    fn open_path(&mut self, path: &std::path::Path) -> Task<Message> {
        let catalog = self.catalog.clone();
        let app = catalog.as_ref().and_then(|catalog| {
            let mime = catalog.mime.type_of(path, false);
            catalog.default_for(&mime).cloned()
        });
        let command = app.and_then(|app| catalog.as_ref()?.command(&app, &[path.to_path_buf()]));
        let mut process = match command {
            Some((program, args)) => {
                let mut process = std::process::Command::new(program);
                process.args(args);
                process
            }
            None => {
                let mut process = std::process::Command::new("xdg-open");
                process.arg(path);
                process
            }
        };
        process.current_dir(path.parent().unwrap_or(path));
        self.launch(process)
    }
}
