//! The Lock page: after how long without input the session locks (`lock.conf`).

use iced::Element;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};

use crate::app::{App, Change, Delay, Message, delays};
use crate::search::Setting;
use crate::settings::File;

impl App {
    pub(crate) fn lock(&self, width: f32) -> Vec<Element<'_, Message>> {
        let minutes = self.settings.lock.idle_minutes;
        let menu = match self.can_change(File::Lock) {
            true => ui::dropdown(
                delays(minutes),
                Some(Delay(minutes)),
                |delay| Message::Set(Change::LockAfter(delay)),
                cr::DELAY_WIDTH,
            ),
            false => ui::description(Delay(minutes).to_string()).into(),
        };
        let row = self.row(Setting::LockAfter, width, t::LOCK_AFTER_HELP, menu, false);
        vec![self.card(Some(t::AUTO_LOCK), vec![row])]
    }
}
