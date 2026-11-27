//! The Clipboard page: whether the compositor keeps what is copied for Super + V, and whether
//! it keeps it on disk (`clipboard.conf`, lot D-42).

use iced::Element;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::texts as t;

use crate::app::{App, Change, Message};
use crate::search::Setting;
use crate::settings::File;

impl App {
    pub(crate) fn clipboard(&self, width: f32) -> Vec<Element<'_, Message>> {
        let config = self.settings.clipboard;
        let history = ui::toggle(
            config.history,
            self.enabled(File::Clipboard, |on| {
                Message::Set(Change::ClipboardHistory(on))
            }),
        );
        // Keeping on disk means nothing while the history is off.
        let keep = ui::toggle(
            config.keep_on_disk,
            self.enabled(File::Clipboard, |on| {
                Message::Set(Change::ClipboardKeep(on))
            })
            .filter(|_| config.history),
        );
        let rows = vec![
            self.row(
                Setting::ClipboardHistory,
                width,
                t::CLIPBOARD_HISTORY_HELP,
                history.into(),
                false,
            ),
            self.row(
                Setting::ClipboardKeep,
                width,
                t::CLIPBOARD_KEEP_HELP,
                keep.into(),
                false,
            ),
        ];
        vec![self.card(Some(t::CLIPBOARD_HISTORY_CARD), rows)]
    }
}
