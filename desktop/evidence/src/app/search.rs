//! The search's tokens and its reach (lot E-15): the bar under the toolbar while searching,
//! the menus of its tokens, and the walk through the subfolders, off the interface thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::Task;
use sherlock_ui::texts::evidence as texts;

use super::{App, Command, Location, MenuItem, Message};
use crate::filter::{DateFilter, KindFilter, Scope, SizeFilter};
use crate::listing::Entry;

/// The menu of a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Kind,
    Date,
    Size,
}

pub fn kind_label(kind: KindFilter) -> &'static str {
    match kind {
        KindFilter::Image => texts::FILTER_IMAGES,
        KindFilter::Document => texts::FILTER_DOCUMENTS,
        KindFilter::Video => texts::FILTER_VIDEOS,
        KindFilter::Audio => texts::FILTER_AUDIO,
        KindFilter::Archive => texts::FILTER_ARCHIVES,
        KindFilter::Folder => texts::FILTER_FOLDERS,
    }
}

pub fn date_label(date: DateFilter) -> &'static str {
    match date {
        DateFilter::Today => texts::FILTER_TODAY,
        DateFilter::Week => texts::FILTER_WEEK,
        DateFilter::Month => texts::FILTER_MONTH,
        DateFilter::Year => texts::FILTER_YEAR,
    }
}

pub fn size_label(size: SizeFilter) -> &'static str {
    match size {
        SizeFilter::Small => texts::FILTER_SMALL,
        SizeFilter::Medium => texts::FILTER_MEDIUM,
        SizeFilter::Large => texts::FILTER_LARGE,
    }
}

fn check(label: &'static str, command: Command, checked: bool) -> MenuItem {
    MenuItem::Action {
        label,
        keys: None,
        command,
        danger: false,
        checked,
    }
}

impl App {
    /// The bar of the tokens shows while searching, or while a token is chosen.
    pub fn filter_bar_shown(&self) -> bool {
        self.location != Location::Trash
            && (self.search_open
                || !self.search.is_empty()
                || self.filters.any()
                || self.filters.scope == Scope::Subfolders)
    }

    /// Height of the bar of the tokens, when it shows.
    pub fn filter_bar_height(&self) -> f32 {
        if self.filter_bar_shown() {
            sherlock_ui::evidence::FILTER_BAR_HEIGHT + sherlock_ui::metrics::BORDER_WIDTH
        } else {
            0.0
        }
    }

    /// The label of a token's chip: its name, and what it keeps.
    pub fn token_label(&self, token: Token) -> String {
        let chosen = match token {
            Token::Kind if !self.filters.kinds.is_empty() => self
                .filters
                .kinds
                .iter()
                .map(|k| kind_label(*k))
                .collect::<Vec<_>>()
                .join(", "),
            Token::Date if self.filters.date.is_some() => {
                self.filters.date.map(date_label).unwrap_or_default().into()
            }
            Token::Size if self.filters.size.is_some() => {
                self.filters.size.map(size_label).unwrap_or_default().into()
            }
            _ => texts::FILTER_ANY.into(),
        };
        let name = match token {
            Token::Kind => texts::FILTER_KIND,
            Token::Date => texts::FILTER_DATE,
            Token::Size => texts::FILTER_SIZE,
        };
        texts::token(name, &chosen)
    }

    /// The entries of a token's menu.
    pub(super) fn token_items(&self, token: Token) -> Vec<MenuItem> {
        match token {
            Token::Kind => KindFilter::ALL
                .into_iter()
                .map(|k| {
                    let on = self.filters.kinds.contains(&k);
                    check(kind_label(k), Command::FilterKind(k), on)
                })
                .collect(),
            Token::Date => std::iter::once(None)
                .chain(DateFilter::ALL.map(Some))
                .map(|d| {
                    let label = d.map_or(texts::FILTER_ANY, date_label);
                    check(label, Command::FilterDate(d), self.filters.date == d)
                })
                .collect(),
            Token::Size => std::iter::once(None)
                .chain(SizeFilter::ALL.map(Some))
                .map(|s| {
                    let label = s.map_or(texts::FILTER_ANY, size_label);
                    check(label, Command::FilterSize(s), self.filters.size == s)
                })
                .collect(),
        }
    }

    /// A token changed: what is shown follows, and the walk starts again if it looks in the
    /// subfolders.
    pub(super) fn filters_changed(&mut self) -> Task<Message> {
        self.refilter();
        self.walk_subfolders()
    }

    /// Looks through the subfolders, when the search reaches them; a walk still running stops.
    pub(super) fn walk_subfolders(&mut self) -> Task<Message> {
        self.deep_stop.store(true, Ordering::Relaxed);
        self.deep = None;
        let searching = !self.search.is_empty() || self.filters.any();
        let Location::Folder(dir) = &self.location else {
            return Task::none();
        };
        if self.filters.scope != Scope::Subfolders || !searching {
            self.refilter();
            return Task::none();
        }
        let stop = Arc::new(AtomicBool::new(false));
        self.deep_stop = stop.clone();
        self.deep_generation += 1;
        let (generation, dir, hidden) = (self.deep_generation, dir.clone(), self.show_hidden);
        let needle = self.search.to_lowercase();
        Task::perform(
            async move { crate::filter::walk(&dir, &needle, hidden, &stop) },
            move |found| Message::DeepFound(generation, found),
        )
    }

    pub(super) fn deep_found(&mut self, generation: u64, found: Vec<Entry>) {
        if generation == self.deep_generation && self.filters.scope == Scope::Subfolders {
            self.deep = Some(found);
            self.refilter();
        }
    }

    /// Forgets the tokens and stops the walk: another folder opens, or Escape.
    pub(super) fn clear_filters(&mut self) {
        self.deep_stop.store(true, Ordering::Relaxed);
        self.deep = None;
        self.filters = crate::filter::Filters::default();
    }
}
