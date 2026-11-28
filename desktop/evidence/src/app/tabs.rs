//! Tabs (lot D-27b), as in the Finder: each tab keeps its folder, its history, its selection
//! and its search. One listing is live, the active tab's; switching saves it and reads the other
//! tab's folder again, off the interface thread like any navigation.

use std::path::PathBuf;

use iced::Task;
use sherlock_ui::motion::Fade;

use super::{App, Location, Message, WindowAction};
use crate::listing::Kind;

/// Tabs open at most.
pub const MAX_TABS: usize = 24;

/// What a tab keeps while another one is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub location: Location,
    history: Vec<Location>,
    future: Vec<Location>,
    selected: Vec<PathBuf>,
    search: String,
}

impl Tab {
    fn at(location: Location) -> Self {
        Self {
            location,
            history: Vec::new(),
            future: Vec::new(),
            selected: Vec::new(),
            search: String::new(),
        }
    }
}

/// The tabs of the window: the active one is live in `App`, its entry here only a placeholder.
#[derive(Debug, Clone, Default)]
pub struct Tabs {
    pub list: Vec<Tab>,
    pub active: usize,
    /// The tab being dragged, by its place.
    pub dragging: Option<usize>,
    /// The tab bar coming in or going out.
    pub fade: Option<Fade>,
}

impl App {
    /// The tabs as shown, the active one with its live location.
    pub(crate) fn tab_list(&self) -> Vec<Location> {
        if self.tabs.list.is_empty() {
            return vec![self.location.clone()];
        }
        self.tabs
            .list
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                if i == self.tabs.active {
                    self.location.clone()
                } else {
                    tab.location.clone()
                }
            })
            .collect()
    }

    /// How far the tab bar is in, from 0 to 1: it shows from two tabs on.
    pub(crate) fn tab_bar_level(&self) -> f32 {
        let rest = if self.tabs.list.len() > 1 { 1.0 } else { 0.0 };
        self.tabs
            .fade
            .as_ref()
            .map_or(rest, |f| f.value(self.now()))
    }

    /// The live state, as a tab.
    fn snapshot(&self) -> Tab {
        Tab {
            location: self.location.clone(),
            history: self.history.clone(),
            future: self.future.clone(),
            selected: self.selected_paths(),
            search: self.search.clone(),
        }
    }

    fn bar_shown(&self) -> bool {
        self.tabs.list.len() > 1
    }

    fn fade_bar(&mut self, was: bool) {
        let now = self.bar_shown();
        if was != now {
            let from = self.tab_bar_level();
            let to = if now { 1.0 } else { 0.0 };
            self.tabs.fade = Some(Fade::new(from, to, self.now(), sherlock_ui::motion::FADE));
        }
    }

    /// Opens tabs on `locations` after the active one; `show`: the first becomes active.
    pub(super) fn open_tabs(&mut self, locations: Vec<Location>, show: bool) -> Task<Message> {
        let was = self.bar_shown();
        if self.tabs.list.is_empty() {
            self.tabs.list.push(self.snapshot());
        }
        let room = MAX_TABS.saturating_sub(self.tabs.list.len());
        let at = self.tabs.active + 1;
        let new: Vec<Tab> = locations.into_iter().take(room).map(Tab::at).collect();
        if new.is_empty() {
            return Task::none();
        }
        self.tabs.list.splice(at..at, new);
        self.fade_bar(was);
        if show {
            self.switch_tab(at)
        } else {
            Task::none()
        }
    }

    /// Shows tab `index`, the active one saved first.
    pub(super) fn switch_tab(&mut self, index: usize) -> Task<Message> {
        if index >= self.tabs.list.len() || index == self.tabs.active {
            return Task::none();
        }
        let current = self.snapshot();
        if let Some(slot) = self.tabs.list.get_mut(self.tabs.active) {
            *slot = current;
        }
        self.tabs.active = index;
        let tab = self.tabs.list[index].clone();
        self.location = tab.location;
        self.history = tab.history;
        self.future = tab.future;
        let task = self.reset_and_load(&tab.selected);
        self.search = tab.search;
        task
    }

    /// Closes tab `index`; the last one closes the window, as Ctrl+W does in the Finder.
    pub(super) fn close_tab(&mut self, index: usize) -> Task<Message> {
        if self.tabs.list.len() <= 1 {
            return self.update(Message::Window(WindowAction::Close));
        }
        let was = self.bar_shown();
        if index == self.tabs.active {
            // The neighbour on the right, else on the left, takes its place.
            let next = if index + 1 < self.tabs.list.len() {
                index + 1
            } else {
                index - 1
            };
            let task = self.switch_tab(next);
            self.tabs.list.remove(index);
            if self.tabs.active > index {
                self.tabs.active -= 1;
            }
            self.fade_bar(was);
            return task;
        }
        self.tabs.list.remove(index);
        if self.tabs.active > index {
            self.tabs.active -= 1;
        }
        self.fade_bar(was);
        Task::none()
    }

    /// The next or previous tab, around (Ctrl+Tab, Ctrl+Maj+Tab).
    pub(super) fn cycle_tab(&mut self, back: bool) -> Task<Message> {
        let count = self.tabs.list.len();
        if count < 2 {
            return Task::none();
        }
        let next = if back {
            (self.tabs.active + count - 1) % count
        } else {
            (self.tabs.active + 1) % count
        };
        self.switch_tab(next)
    }

    /// A dragged tab over place `to`: it moves there.
    pub(super) fn drag_tab(&mut self, to: usize) {
        let Some(from) = self.tabs.dragging else {
            return;
        };
        if to >= self.tabs.list.len() || to == from {
            return;
        }
        let tab = self.tabs.list.remove(from);
        self.tabs.list.insert(to, tab);
        if self.tabs.active == from {
            self.tabs.active = to;
        } else if from < self.tabs.active && to >= self.tabs.active {
            self.tabs.active -= 1;
        } else if from > self.tabs.active && to <= self.tabs.active {
            self.tabs.active += 1;
        }
        self.tabs.dragging = Some(to);
    }

    /// The folders selected, each in a tab of its own (Enter on several folders, the middle
    /// click on one).
    pub(super) fn selected_folders(&self) -> Vec<Location> {
        self.selection
            .iter()
            .filter_map(|i| self.entries.get(i))
            .filter(|e| e.kind == Kind::Folder)
            .map(|e| Location::Folder(e.path.clone()))
            .collect()
    }
}
