//! Popups that leave the keyboard where it is (ADR 0029, lot E-02c), through the `grab` option
//! of the patched `iced_exwlshell` (`third_party/iced_exwlshell/SHERLOCKOS.md`): the path
//! field's completion list, which the field keeps typing into, and the tooltips of the toolbar's
//! buttons. Under winit both stay drawn inside the window.

use std::time::Duration;

use iced::widget::container;
use iced::{Element, Length, Point, Task, window};
use iced_exwlshell::actions::{IcedNewPopupSettings, RegionActionCB};
use iced_exwlshell::reexport::{PopupAnchor, PopupGravity};
use sherlock_ui::evidence as more;
use sherlock_ui::surface::Elevation;
use sherlock_ui::texts::evidence as t;
use sherlock_ui::{ellipsis, metrics, theme, typography};

use super::popup::{extent, input_region, menu_anchor, popup_size, position};
use super::{App, Message};

/// A tooltip waits this long under the pointer before it shows, then fades in.
pub const DELAY: Duration = Duration::from_millis(600);
pub const FADE: Duration = Duration::from_millis(120);

/// The toolbar buttons whose tooltip is a popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintTarget {
    Back,
    Forward,
    Search,
    Preview,
}

impl HintTarget {
    /// The label it shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Back => t::BACK,
            Self::Forward => t::FORWARD,
            Self::Search => t::SEARCH,
            Self::Preview => t::PREVIEW_BUTTON,
        }
    }
}

/// The pointer resting on a button: which, and since when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hover {
    pub target: HintTarget,
    pub since: Duration,
    /// Set by `App::settle`: the tooltip shows at once, fully.
    pub settled: bool,
}

impl Hover {
    /// How far its tooltip has faded in, from 0 (not shown yet) to 1.
    pub fn level(&self, now: Duration) -> f32 {
        if self.settled {
            return 1.0;
        }
        match now.checked_sub(self.since + DELAY) {
            Some(shown) => (shown.as_secs_f32() / FADE.as_secs_f32()).min(1.0),
            None => 0.0,
        }
    }
}

/// The two popups without a grab, when open.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Unfocused {
    /// The completion list, with its box's size in rows and in pixels, to make it again when
    /// either changes.
    pub suggestions: Option<(window::Id, usize, u32)>,
    pub tooltip: Option<(window::Id, HintTarget)>,
}

/// A popup to open: its settings and the size of its box, inside the shadow.
pub(crate) struct Planned {
    pub settings: IcedNewPopupSettings,
    pub width: f32,
    pub height: f32,
}

impl App {
    /// The top left corner of the toolbar button `target`, in the window.
    fn hint_anchor(&self, target: HintTarget) -> Point {
        let y = metrics::TITLE_BAR_HEIGHT
            + metrics::BORDER_WIDTH
            + (metrics::TOOLBAR_HEIGHT - metrics::ICON_BUTTON) / 2.0;
        let step = metrics::ICON_BUTTON + metrics::GAP_ICON;
        let right = self.size.width - metrics::TOOLBAR_PADDING - metrics::ICON_BUTTON;
        let x = match target {
            HintTarget::Back => metrics::TOOLBAR_PADDING,
            HintTarget::Forward => metrics::TOOLBAR_PADDING + step,
            HintTarget::Preview => right,
            HintTarget::Search => right - step,
        };
        Point::new(x, y)
    }

    /// The pointer came onto a toolbar button, or left it.
    pub(super) fn hover(&mut self, target: Option<HintTarget>) {
        self.hover = target.map(|target| Hover {
            target,
            since: self.now(),
            settled: false,
        });
    }

    /// Frames are needed: a tooltip waits, or fades in.
    pub(crate) fn hint_pending(&self) -> bool {
        self.main.is_some() && self.hover.is_some_and(|h| h.level(self.now()) < 1.0)
    }

    /// Opens or closes the completion list and the tooltip, with no grab.
    pub(super) fn sync_unfocused(&mut self, main: window::Id) -> Task<Message> {
        Task::batch([self.sync_suggestions(main), self.sync_tooltip(main)])
    }

    /// How many rows the completion list shows, while it shows or fades out.
    fn suggestion_count(&self) -> Option<usize> {
        self.path_edit
            .as_ref()
            .or(self.path_leaving.as_ref())
            .filter(|edit| !edit.sheet && self.suggest_level(edit) > 0.0)
            .map(|edit| {
                if edit.suggestions.is_empty() {
                    self.suggestions_shown.len()
                } else {
                    edit.suggestions.len()
                }
            })
            .filter(|&rows| rows > 0)
    }

    /// The completion list's popup, under the field as `view::path_suggestions` pins it
    /// inside the window; it leaves the keyboard to the field.
    pub(crate) fn suggestions_popup(&self, main: window::Id, rows: usize) -> Option<Planned> {
        let (left, span) = self.path_bar_span();
        let width = span.min(more::GO_TO_WIDTH);
        let shown = (rows as f32).min(more::COMPLETION_ROWS);
        let height = shown * metrics::MENU_ITEM_HEIGHT
            + 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
        let margins = Elevation::Menu.margins();
        let anchor = menu_anchor(Point::new(left, crate::view::folded_top()), margins);
        let settings = IcedNewPopupSettings::new(
            main,
            popup_size(width, height, margins)?,
            position(anchor),
            extent(anchor)?,
        )
        .anchor(PopupAnchor::TopLeft)
        .gravity(PopupGravity::BottomRight)
        .grab(false);
        Some(Planned {
            settings,
            width,
            height,
        })
    }

    /// The tooltip's popup, centred under its button, with its shadow around; flipped above
    /// it, or slid, at the screen's edge. It leaves the keyboard where it is.
    pub(crate) fn tooltip_popup(&self, main: window::Id, target: HintTarget) -> Option<Planned> {
        let edge = 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
        let width = ellipsis::width(target.label(), typography::UI, typography::UI_SIZE) + edge;
        let height = typography::UI_SIZE * typography::LINE_HEIGHT + edge;
        let margins = Elevation::Menu.margins();
        let at = self.hint_anchor(target);
        let button = (
            (at.x - margins.left).round() as i32,
            (at.y - margins.top).round() as i32,
            (metrics::ICON_BUTTON + margins.left + margins.right).round() as u32,
            (metrics::ICON_BUTTON + margins.top + margins.bottom).round() as u32,
        );
        let settings = IcedNewPopupSettings::new(
            main,
            popup_size(width, height, margins)?,
            position(button),
            extent(button)?,
        )
        .anchor(PopupAnchor::Bottom)
        .gravity(PopupGravity::Bottom)
        .grab(false);
        Some(Planned {
            settings,
            width,
            height,
        })
    }

    fn sync_suggestions(&mut self, main: window::Id) -> Task<Message> {
        let wanted = self.suggestion_count().map(|rows| {
            let (_, span) = self.path_bar_span();
            (rows, span.min(more::GO_TO_WIDTH).round() as u32)
        });
        let mut tasks = Vec::new();
        if let Some((id, rows, width)) = self.unfocused.suggestions
            && Some((rows, width)) != wanted
        {
            tasks.push(Task::done(Message::RemoveWindow(id)));
            self.unfocused.suggestions = None;
        }
        if let (Some((rows, made)), None) = (wanted, self.unfocused.suggestions)
            && let Some(Planned {
                settings,
                width,
                height,
            }) = self.suggestions_popup(main, rows)
        {
            let id = window::Id::unique();
            self.unfocused.suggestions = Some((id, rows, made));
            tasks.push(Task::done(Message::NewPopUp { settings, id }));
            tasks.push(input_region(id, width, height, Elevation::Menu.margins()));
        }
        Task::batch(tasks)
    }

    fn sync_tooltip(&mut self, main: window::Id) -> Task<Message> {
        let now = self.now();
        let wanted = self.hover.filter(|h| h.level(now) > 0.0).map(|h| h.target);
        let mut tasks = Vec::new();
        if let Some((id, shown)) = self.unfocused.tooltip
            && Some(shown) != wanted
        {
            tasks.push(Task::done(Message::RemoveWindow(id)));
            self.unfocused.tooltip = None;
        }
        if let (Some(target), None) = (wanted, self.unfocused.tooltip)
            && let Some(Planned { settings, .. }) = self.tooltip_popup(main, target)
        {
            let id = window::Id::unique();
            self.unfocused.tooltip = Some((id, target));
            tasks.push(Task::done(Message::NewPopUp { settings, id }));
            // The pointer goes through it: it never takes the hover from its button.
            tasks.push(Task::done(Message::SetInputRegion {
                id,
                callback: RegionActionCB::new(|_| {}),
            }));
        }
        Task::batch(tasks)
    }

    /// The view of the completion list or of the tooltip, or `None` when `id` is neither.
    pub(crate) fn unfocused_view(&self, id: window::Id) -> Option<Element<'_, Message>> {
        let margins = Elevation::Menu.margins();
        if self
            .unfocused
            .suggestions
            .is_some_and(|(list, ..)| list == id)
        {
            let panel = container(self.suggestions_panel()?)
                .padding(margins)
                .width(Length::Fill)
                .height(Length::Fill);
            return Some(panel.into());
        }
        let (_, target) = self.unfocused.tooltip.filter(|(tip, _)| *tip == id)?;
        let level = self.hover.map_or(1.0, |h| h.level(self.now()));
        let tip = container(sherlock_ui::component::body(target.label()))
            .padding(metrics::MENU_PADDING)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::tooltip);
        let tip = sherlock_ui::animated::reveal(tip).opacity(level);
        Some(container(tip).padding(margins).into())
    }
}
