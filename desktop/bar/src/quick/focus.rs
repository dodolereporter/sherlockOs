//! Do not disturb in the control centre (D-23e), as the Focus tile of macOS: a click switches it
//! on until switched off, or off; its chevron opens the durations: one hour, until this
//! evening, until tomorrow morning. The notifications file is read and written on a thread of
//! its own (`blocking`), never on the interface's: a slow home folder leaves the panel alive.

use iced::widget::{Column, column, row, space, text};
use iced::{Alignment, Element, Length, Task};
use notifications_config::{NotificationsConfig, epoch_now};
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, text as words};
use sherlock_ui::{metrics, strings, typography};

use super::{Message, Page, Quick, Service, blocking};
use crate::system::Problem;

/// How long Do not disturb is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Off,
    /// Until switched off.
    On,
    Hour,
    /// Until `EVENING` today.
    Evening,
    /// Until `MORNING` tomorrow.
    Tomorrow,
}

/// The evening that « Jusqu'à ce soir » reaches, and the morning of « Jusqu'à demain matin »,
/// in hours of the local day.
pub const EVENING: u64 = 19;
pub const MORNING: u64 = 8;

/// The end of `span` asked at `now` (seconds since 1970), the local time being `hour:minute`:
/// `None` for no end. `Evening` past the evening has no end to give.
pub fn end(span: Span, now: u64, hour: u8, minute: u8) -> Option<u64> {
    let midnight = now.saturating_sub(u64::from(hour) * 3600 + u64::from(minute) * 60);
    match span {
        Span::Off | Span::On => None,
        Span::Hour => Some(now + 3600),
        Span::Evening => (u64::from(hour) < EVENING).then_some(midnight + EVENING * 3600),
        Span::Tomorrow => Some(midnight + 86_400 + MORNING * 3600),
    }
}

/// The local hour and minute at `epoch`, if the clock can say.
fn local(epoch: u64) -> Option<(u8, u8)> {
    let time = crate::source::clock::local(i64::try_from(epoch).ok()?).ok()?;
    Some((time.hour, time.minute))
}

impl Quick {
    /// Reads the notifications file, off the interface thread.
    pub(super) fn read_do_not_disturb(&self) -> Task<Message> {
        let Some(path) = self.settings.clone() else {
            return Task::none();
        };
        blocking(
            move || {
                NotificationsConfig::load(&path)
                    .map(|(config, _)| config)
                    .map_err(|e| Problem::Failed(e.to_string()))
            },
            |read| Message::DoNotDisturbRead(read, false),
        )
    }

    /// Writes `span` into the notifications file, off the interface thread.
    pub(super) fn set_do_not_disturb(&mut self, span: Span) -> Task<Message> {
        let Some(path) = self.settings.clone() else {
            return Task::none();
        };
        let now = epoch_now();
        let (hour, minute) = local(now).unwrap_or((12, 0));
        let wanted = NotificationsConfig {
            do_not_disturb: span != Span::Off,
            until: end(span, now, hour, minute),
        };
        // Shown at once; a failure to write brings the file's state back.
        self.do_not_disturb = Some(span != Span::Off);
        self.dnd_until = wanted.until;
        blocking(
            move || {
                let (mut config, _) = NotificationsConfig::load(&path).unwrap_or_default();
                config.do_not_disturb = wanted.do_not_disturb;
                config.until = wanted.until;
                config
                    .save(&path)
                    .map(|()| config)
                    .map_err(|e| Problem::Failed(e.to_string()))
            },
            |saved| Message::DoNotDisturbRead(saved, true),
        )
    }

    /// The file read, or written (`written`): what holds now, or the failure under the tile.
    pub(super) fn do_not_disturb_read(
        &mut self,
        read: Result<NotificationsConfig, Problem>,
        written: bool,
    ) -> Task<Message> {
        match read {
            Ok(config) => {
                self.do_not_disturb = Some(config.active_now());
                self.dnd_until = config.until.filter(|_| config.active_now());
                if self
                    .failure
                    .as_ref()
                    .is_some_and(|(s, _)| *s == Service::DoNotDisturb)
                {
                    self.failure = None;
                }
                Task::none()
            }
            Err(problem) => {
                tracing::warn!(%problem, "Do not disturb");
                if written {
                    self.failure = Some((Service::DoNotDisturb, problem));
                    return self.read_do_not_disturb();
                }
                self.do_not_disturb = Some(false);
                Task::none()
            }
        }
    }

    /// The durations page, or back to the main page.
    pub(super) fn show_focus(&mut self, show: bool) {
        let page = if show { Page::Focus } else { Page::Main };
        self.turn_page(page, show);
    }

    /// The line of the tile: « Activé », « Jusqu'à 19:00 », « Désactivé ».
    pub(super) fn do_not_disturb_line(&self) -> String {
        match (self.do_not_disturb, self.dnd_until.and_then(local)) {
            (Some(true), Some((hour, minute))) => words::until(&strings::clock(hour, minute)),
            (Some(true), None) => words::ON.to_owned(),
            _ => words::OFF.to_owned(),
        }
    }

    pub(super) fn view_focus(&self) -> Element<'_, Message> {
        let on = self.do_not_disturb == Some(true);
        let back = ui::icon_button(Icon::Back, words::BACK, Some(Message::ShowFocus(false)));
        let title = text(words::DO_NOT_DISTURB)
            .font(typography::TITLE)
            .size(panels::TITLE_SIZE);
        let switch: Option<fn(bool) -> Message> =
            self.settings.as_ref().map(|_| switch as fn(bool) -> _);
        let head = row![back, title, space::horizontal(), ui::toggle(on, switch)]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center);
        let now = epoch_now();
        let (hour, minute) = local(now).unwrap_or((12, 0));
        // The duration whose end is the closest to the one asked, within the hour: checked.
        let closest = self.dnd_until.and_then(|until| {
            [Span::Hour, Span::Evening, Span::Tomorrow]
                .into_iter()
                .filter_map(|span| Some((span, end(span, now, hour, minute)?.abs_diff(until))))
                .filter(|(_, gap)| *gap < 3600)
                .min_by_key(|(_, gap)| *gap)
                .map(|(span, _)| span)
        });
        let mut rows: Vec<Element<'_, Message>> = Vec::new();
        for (span, label) in [
            (Span::Hour, words::FOR_AN_HOUR),
            (Span::Evening, words::UNTIL_EVENING),
            (Span::Tomorrow, words::UNTIL_TOMORROW),
        ] {
            // Past the evening, « Jusqu'à ce soir » has no evening left.
            if end(span, now, hour, minute).is_none() {
                continue;
            }
            let chosen = on && Some(span) == closest;
            let trailing = if chosen {
                Trailing::Checked
            } else {
                Trailing::None
            };
            let pick = Some(Message::DoNotDisturb(span));
            rows.push(ui::menu_item_with(None, label, trailing, pick).into());
        }
        let list = Column::with_children(rows).spacing(metrics::GAP_TIGHT);
        let mut page = column![head, list]
            .spacing(metrics::GAP_INLINE)
            .height(Length::Fill);
        if let Some(failed) = self.failed(Service::DoNotDisturb) {
            page = page.push(
                text(failed)
                    .font(typography::UI)
                    .size(typography::SMALL_SIZE)
                    .style(panels::error_text),
            );
        }
        page.into()
    }
}

fn switch(on: bool) -> Message {
    Message::DoNotDisturb(if on { Span::On } else { Span::Off })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_duration_ends_at_its_time() {
        // 1 000 000 s after 1970 at 10:30 local time: midnight 37 800 s before.
        let now = 1_000_000;
        let midnight = now - 37_800;
        assert_eq!(end(Span::Hour, now, 10, 30), Some(now + 3600));
        assert_eq!(end(Span::Evening, now, 10, 30), Some(midnight + 19 * 3600));
        assert_eq!(end(Span::Tomorrow, now, 10, 30), Some(midnight + 32 * 3600));
        assert_eq!(end(Span::On, now, 10, 30), None);
        // Past the evening, there is no evening to reach.
        assert_eq!(end(Span::Evening, now, 20, 0), None);
    }

    #[test]
    fn the_file_is_written_off_the_interface_and_read_back() {
        let dir = std::env::temp_dir().join(format!("bar-focus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("notifications").join("notifications.conf");
        let mut quick = Quick {
            settings: Some(file.clone()),
            ..Quick::default()
        };
        // Shown at once, written by the task (not run here).
        let _ = quick.update(Message::DoNotDisturb(Span::Hour));
        assert_eq!(quick.do_not_disturb, Some(true));
        assert!(quick.dnd_until.is_some());
        assert!(!file.exists(), "nothing written on the interface thread");
        // What the task reports.
        let saved = NotificationsConfig {
            do_not_disturb: true,
            until: Some(epoch_now() + 3600),
        };
        let _ = quick.update(Message::DoNotDisturbRead(Ok(saved), true));
        assert_eq!(quick.do_not_disturb, Some(true));
        let past = NotificationsConfig {
            do_not_disturb: true,
            until: Some(1),
        };
        let _ = quick.update(Message::DoNotDisturbRead(Ok(past), false));
        assert_eq!(quick.do_not_disturb, Some(false), "its end passed");
        let refused = Err(Problem::Failed(String::from("read-only")));
        let _ = quick.update(Message::DoNotDisturbRead(refused, true));
        assert!(matches!(quick.failure, Some((Service::DoNotDisturb, _))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
