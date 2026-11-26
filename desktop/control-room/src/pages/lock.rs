//! The Lock page: after how long without input the session locks (`lock.conf`).

use iced::Element;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::{self as cr, texts as t};

use crate::app::{App, Change, Delay, Message, delays};
use crate::search::Setting;

impl App {
    pub(crate) fn lock(&self, width: f32) -> Vec<Element<'_, Message>> {
        let minutes = self.settings.lock.idle_minutes;
        let menu = ui::dropdown(
            delays(minutes),
            Some(Delay(minutes)),
            |delay| Message::Set(Change::LockAfter(delay)),
            cr::DELAY_WIDTH,
        );
        let row = self.row(Setting::LockAfter, width, t::LOCK_AFTER_HELP, menu, false);
        vec![self.card(Some(t::AUTO_LOCK), vec![row])]
    }
}
