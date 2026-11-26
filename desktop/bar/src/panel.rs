//! The panels of the bar, each an `xdg_popup` of the bar surface placed under its right end:
//! the control centre (the notification centre, opened by the indicators) and the calendar
//! (opened by the clock). Their size, their place, and their views.

use iced::widget::{Column, column, container, mouse_area, opaque, row, scrollable, space, text};
use iced::{Alignment, Element, Length, Padding};
use sherlock_ui::component::{self as ui};
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, text as words};
use sherlock_ui::surface::{self, Elevation};
use sherlock_ui::{metrics, strings, typography};

use crate::app::Message;
use crate::calendar::{Date, Month};
use crate::history::{Entry, Group};

/// Which panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    /// Opened by the indicators: the notification centre.
    Control,
    /// Opened by the date and time.
    Calendar,
}

impl Panel {
    /// Size of the panel itself, without the room for its shadow.
    pub fn size(self) -> (f32, f32) {
        match self {
            Self::Control => (panels::CENTER_WIDTH, panels::CONTROL_HEIGHT),
            Self::Calendar => (panels::CALENDAR_WIDTH, panels::CALENDAR_HEIGHT),
        }
    }
}

/// Room around the panel in its surface. In software, only the gap below the bar: the surface
/// is opaque and holds no shadow (ADR 0018). On the GPU, the panel shadow of the board, cut on
/// the right where the screen ends.
pub fn margins(gpu: bool) -> Padding {
    if !gpu {
        return Padding {
            top: panels::BELOW_BAR,
            ..Padding::ZERO
        };
    }
    let shadow = Elevation::Panel.margins();
    Padding {
        right: shadow.right.min(metrics::TOP_BAR_MARGIN_SIDE),
        ..shadow
    }
}

/// Size of the panel surface, `margins` included.
pub fn surface_size(panel: Panel, margins: Padding) -> (u32, u32) {
    let (width, height) = panel.size();
    (
        (width + margins.left + margins.right).ceil() as u32,
        (height + margins.top + margins.bottom).ceil() as u32,
    )
}

/// Anchor rectangle of the panel surface in the bar surface (x, y, width, height), for a bar
/// `bar_width` wide whose side margins are `margin_side`. The panel ends where the floating bar
/// ends, 12 from the right edge of the screen, and starts `BELOW_BAR` under it. As for the
/// menu, the shadow starts above the bar surface, where xdg_positioner forbids an anchor
/// rectangle: the rectangle ends at the centre of the top edge of the surface, which grows
/// down from there, centred (anchor and gravity `Bottom`).
pub fn anchor_rect(
    panel: Panel,
    margins: Padding,
    bar_width: f32,
    margin_side: f32,
) -> (i32, i32, u32, u32) {
    let (width, _) = surface_size(panel, margins);
    let (panel_width, _) = panel.size();
    let right = bar_width - (metrics::TOP_BAR_MARGIN_SIDE - margin_side).max(0.0);
    let left = right - panel_width - margins.left;
    let top = metrics::TOP_BAR_HEIGHT + panels::BELOW_BAR - margins.top;
    let centre = left.round() as i32 + (width / 2) as i32;
    (centre - 1, 0, 2, top.max(1.0) as u32)
}

/// The panel itself in its surface (x, y, width, height): the input region of the popup.
pub fn input_rect(panel: Panel, margins: Padding) -> (i32, i32, i32, i32) {
    let (width, height) = panel.size();
    (
        margins.left as i32,
        margins.top as i32,
        width.ceil() as i32,
        height.ceil() as i32,
    )
}

/// A panel in its surface: the glass, its shadow on the GPU, and a click on the room around
/// it closing it, for a compositor that would ignore the input region.
pub fn frame<'a>(
    content: impl Into<Element<'a, Message>>,
    panel: Panel,
    margins: Padding,
) -> Element<'a, Message> {
    let (width, height) = panel.size();
    let glass = container(content)
        .width(width)
        .height(height)
        .padding(panels::PADDING + metrics::BORDER_WIDTH)
        .style(panels::panel);
    let shadowed = surface::shadowed(glass, panels::RADIUS, Elevation::Panel);
    let room = container(opaque(shadowed)).padding(margins);
    mouse_area(room).on_press(Message::ClosePanel).into()
}

/// What the notification centre shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum History {
    /// Not read yet.
    #[default]
    Loading,
    Ready(Vec<Group>),
    /// The daemon does not answer.
    Unavailable,
}

/// The notification centre of the Panels board: its title and « Tout effacer », then a card per
/// application, newest first.
pub fn center(history: &History) -> Element<'_, Message> {
    let empty = !matches!(history, History::Ready(groups) if !groups.is_empty());
    let title = text(words::NOTIFICATIONS)
        .font(typography::TITLE)
        .size(panels::TITLE_SIZE);
    let clear = panels::clear_button(words::CLEAR_ALL, (!empty).then_some(Message::ClearHistory));
    let header = row![title, space::horizontal(), clear]
        .align_y(Alignment::Center)
        .padding(Padding {
            left: 2.0,
            right: 2.0,
            bottom: 2.0,
            top: 0.0,
        });
    let body: Element<'_, Message> = match history {
        History::Ready(groups) if !groups.is_empty() => {
            let cards = groups.iter().map(card);
            scrollable(Column::with_children(cards).spacing(panels::CENTER_GAP))
                .height(Length::Fill)
                .into()
        }
        History::Unavailable => notice(words::CENTER_UNAVAILABLE),
        History::Loading => space().height(Length::Fill).into(),
        History::Ready(_) => notice(words::NO_NOTIFICATION),
    };
    column![header, body]
        .spacing(panels::CENTER_GAP)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// A short line in the middle of an empty panel.
fn notice(line: &str) -> Element<'_, Message> {
    let line = ui::body(line)
        .style(panels::secondary_text)
        .align_x(Alignment::Center);
    container(line).center(Length::Fill).into()
}

/// The card of an application: its tile and name, then its newest notifications.
fn card(group: &Group) -> Element<'_, Message> {
    let initial: String = group
        .app
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default();
    let tile = container(
        text(initial)
            .font(typography::UI_SEMIBOLD)
            .size(panels::TILE_LETTER_SIZE),
    )
    .center(panels::TILE)
    .style(panels::tile);
    let small = |s: &str| -> Element<'_, Message> {
        text(s.to_owned())
            .font(typography::UI)
            .size(typography::SMALL_SIZE)
            .style(panels::secondary_text)
            .into()
    };
    let single = group.entries.len() == 1;
    let mut header = row![tile, small(&group.app)]
        .spacing(panels::HEADER_GAP)
        .align_y(Alignment::Center);
    if let (true, Some(first)) = (single, group.entries.first()) {
        header = header.push(space::horizontal()).push(small(&first.when));
    }
    let mut rows = vec![header.into()];
    for entry in group.entries.iter().take(panels::GROUP_MAX) {
        rows.push(notification(entry, !single));
    }
    let hidden = group.entries.len().saturating_sub(panels::GROUP_MAX);
    if hidden > 0 {
        rows.push(small(&words::more(hidden)));
    }
    container(Column::with_children(rows).spacing(panels::CARD_GAP))
        .width(Length::Fill)
        .padding(panels::CARD_PADDING)
        .style(panels::card)
        .into()
}

/// One notification of a card: title, body, and its buttons while it is open. `timed`: its
/// time on the right of its title, when the card holds several.
fn notification(entry: &Entry, timed: bool) -> Element<'_, Message> {
    let title = ui::body(entry.summary.as_str()).font(typography::UI_SEMIBOLD);
    let mut lines: Vec<Element<'_, Message>> = if timed {
        let when = text(entry.when.as_str())
            .font(typography::UI)
            .size(typography::SMALL_SIZE)
            .style(panels::secondary_text);
        vec![
            row![title.width(Length::Fill), when]
                .spacing(panels::HEADER_GAP)
                .into(),
        ]
    } else {
        vec![title.into()]
    };
    if !entry.body.is_empty() {
        lines.push(
            ui::body(entry.body.as_str())
                .style(panels::secondary_text)
                .into(),
        );
    }
    let buttons: Vec<Element<'_, Message>> = entry
        .actions
        .iter()
        .filter(|(key, _)| entry.open && key != "default")
        .take(sherlock_ui::notifications::MAX_BUTTONS)
        .enumerate()
        .map(|(i, (key, label))| {
            let invoke = Message::InvokeAction(entry.id, key.clone());
            panels::action_button(label.as_str(), i == 0, Some(invoke)).into()
        })
        .collect();
    if !buttons.is_empty() {
        let buttons = iced::widget::Row::with_children(buttons).spacing(panels::ACTION_GAP);
        lines.push(
            container(buttons)
                .padding(Padding {
                    top: 2.0,
                    ..Padding::ZERO
                })
                .into(),
        );
    }
    Column::with_children(lines)
        .spacing(panels::CARD_GAP)
        .into()
}

/// The calendar: the month shown and its arrows, the weekday heads, six weeks with today
/// marked, then the slot of the future agenda.
pub fn calendar(month: Month, today: Option<Date>) -> Element<'static, Message> {
    let title = text(words::month_title(month.month, month.year))
        .font(typography::TITLE)
        .size(panels::TITLE_SIZE);
    let previous = ui::icon_button(
        Icon::Back,
        words::PREVIOUS_MONTH,
        Some(Message::ShowMonth(month.previous())),
    );
    let next = ui::icon_button(
        Icon::Chevron,
        words::NEXT_MONTH,
        Some(Message::ShowMonth(month.next())),
    );
    let head = row![title, space::horizontal(), previous, next]
        .spacing(metrics::GAP_TIGHT)
        .align_y(Alignment::Center)
        .height(panels::MONTH_ROW);
    let weekdays = row(words::WEEKDAYS.iter().map(|day| {
        let day = text(*day)
            .font(typography::UI_MEDIUM)
            .size(typography::SMALL_SIZE)
            .style(panels::weekday_text);
        container(day)
            .center_x(panels::DAY_CELL)
            .center_y(panels::WEEKDAY_ROW)
            .into()
    }));
    let cells = month.grid(panels::WEEKS);
    let weeks = cells.chunks(7).map(|week| {
        row(week.iter().map(|cell| {
            let label = text(cell.date.day.to_string())
                .font(typography::UI)
                .size(typography::UI_SIZE);
            let day: Element<'static, Message> = if Some(cell.date) == today {
                container(label.font(typography::UI_SEMIBOLD))
                    .center(panels::DAY_MARK)
                    .style(panels::today)
                    .into()
            } else {
                label.style(panels::day_text(cell.in_month)).into()
            };
            container(day).center(panels::DAY_CELL).into()
        }))
        .into()
    });
    let agenda = column![
        ui::section_label(words::AGENDA),
        ui::description(words::AGENDA_SOON),
    ]
    .spacing(metrics::GAP_TIGHT * 2.0)
    .padding(Padding {
        top: metrics::GAP_INLINE,
        ..Padding::ZERO
    })
    .height(panels::AGENDA_HEIGHT);
    column![
        head,
        space().height(panels::CALENDAR_GAP),
        weekdays,
        Column::with_children(weeks),
        space().height(metrics::GAP_SECTION),
        ui::divider(),
        agenda,
    ]
    .width(Length::Fill)
    .into()
}

/// The date of today from the bar's clock, for the calendar.
pub fn today(clock: Option<&crate::source::clock::LocalTime>) -> Option<Date> {
    clock.map(|c| Date {
        year: c.year,
        month: c.month,
        day: c.day,
    })
}

/// When a notification came, as the centre shows it: its time on the day of `today`, its
/// short date before.
pub fn when(received: i64, today: Option<Date>) -> String {
    let Ok(tm) = appearance_config::local(received) else {
        return String::new();
    };
    let field = |v: i32| u8::try_from(v).unwrap_or_default();
    let date = Date {
        year: tm.tm_year + 1900,
        month: field(tm.tm_mon + 1),
        day: field(tm.tm_mday),
    };
    if Some(date) == today {
        strings::clock(field(tm.tm_hour), field(tm.tm_min))
    } else {
        let weekday = if tm.tm_wday == 0 {
            7
        } else {
            field(tm.tm_wday)
        };
        strings::short_date(weekday, date.day, date.month).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the panel lands on the screen, from its surface and its margins, under a bar
    /// floating on the reference screen.
    fn on_screen(panel: Panel, margins: Padding) -> (f32, f32) {
        let side = metrics::TOP_BAR_MARGIN_SIDE;
        let bar_width = metrics::REFERENCE_SCREEN.0 - 2.0 * side;
        let (width, _) = surface_size(panel, margins);
        let (x, _, w, top) = anchor_rect(panel, margins, bar_width, side);
        assert_eq!(w, 2);
        assert_eq!(width % 2, 0, "an odd width cannot be centred exactly");
        let surface_left = side + (x + 1) as f32 - (width / 2) as f32;
        let surface_top = metrics::TOP_BAR_MARGIN_TOP + top as f32;
        (surface_left + margins.left, surface_top + margins.top)
    }

    #[test]
    fn the_panels_land_where_the_board_puts_them_with_or_without_their_shadow() {
        let right = metrics::REFERENCE_SCREEN.0 - metrics::TOP_BAR_MARGIN_SIDE;
        for gpu in [false, true] {
            let margins = margins(gpu);
            for panel in [Panel::Control, Panel::Calendar] {
                let (left, top) = on_screen(panel, margins);
                assert_eq!(left + panel.size().0, right, "{panel:?}, gpu: {gpu}");
                assert_eq!(top, 54.0, "{panel:?}, gpu: {gpu}");
                let (x, y, _, height) = anchor_rect(panel, margins, right - 12.0, 12.0);
                assert!(x >= 0 && y >= 0 && height >= 1);
                assert!(height as f32 <= metrics::TOP_BAR_HEIGHT);
                // The input region is the panel alone.
                let (ix, iy, iw, ih) = input_rect(panel, margins);
                assert_eq!((ix, iy), (margins.left as i32, margins.top as i32));
                assert_eq!(
                    (iw, ih),
                    (panel.size().0 as i32, panel.size().1.ceil() as i32)
                );
            }
        }
        // The board's notification centre ends at the quick settings; alone, at the bar end.
        assert_eq!(on_screen(Panel::Control, margins(false)).0, right - 320.0);
    }

    #[test]
    fn under_an_attached_bar_the_panel_keeps_its_distance_to_the_edge() {
        let screen = metrics::REFERENCE_SCREEN.0;
        let margins = margins(false);
        let (x, _, _, _) = anchor_rect(Panel::Calendar, margins, screen, 0.0);
        let (width, _) = surface_size(Panel::Calendar, margins);
        let left = (x + 1) as f32 - (width / 2) as f32;
        assert_eq!(left + panels::CALENDAR_WIDTH, screen - 12.0);
    }

    #[test]
    fn the_time_of_a_notification_of_today_and_of_another_day() {
        // 2026-11-24 20:44 UTC: the local date is read from the clock of the machine.
        let received = 1_795_553_040;
        let Ok(tm) = appearance_config::local(received) else {
            panic!("local time");
        };
        let today = Date {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u8,
            day: tm.tm_mday as u8,
        };
        assert_eq!(
            when(received, Some(today)),
            strings::clock(tm.tm_hour as u8, tm.tm_min as u8)
        );
        let other = Date { day: 1, ..today };
        assert!(when(received, Some(other)).contains("nov."));
    }
}
