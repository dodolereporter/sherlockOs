//! The Touchpad and mouse page: the right click (two fingers, the macOS way, by default, or the
//! bottom right corner), tap to click, natural scrolling, speed, typing, the gestures (tap and
//! drag, drag lock, three fingers down; the others listed), and showing the clicks.

use desktop_config::touchpad::{Click, SwipeDown};
use iced::Element;
use iced::widget::column;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::texts as t;
use sherlock_ui::metrics;

use crate::app::{App, Change, Message};
use crate::search::Setting;
use crate::settings::File;

impl App {
    pub(crate) fn touchpad(&self, width: f32) -> Vec<Element<'_, Message>> {
        let pad = &self.settings.touchpad;
        let click = |label, which| -> Element<'_, Message> {
            let choose = self.message(Change::Click(which));
            ui::radio(label, pad.click == which, choose).into()
        };
        let clicks: Element<'_, Message> = column![
            click(t::TWO_FINGERS, Click::Fingers),
            click(t::CORNER, Click::Areas),
        ]
        .spacing(metrics::GAP_TIGHT)
        .into();
        let toggle = |on: bool, change: fn(bool) -> Message| {
            let file = match change(on) {
                Message::Set(change) => crate::app::file_of(change),
                _ => File::Touchpad,
            };
            ui::toggle(on, self.enabled(file, change)).into()
        };
        let tap = toggle(pad.tap_to_click, |on| Message::Set(Change::TapToClick(on)));
        let natural = toggle(pad.natural_scroll, |on| {
            Message::Set(Change::NaturalScroll(on))
        });
        let typing = toggle(pad.disable_while_typing, |on| {
            Message::Set(Change::Typing(on))
        });
        let shown = self.settings.pointer.show_clicks;
        let show_clicks = toggle(shown, |on| Message::Set(Change::ShowClicks(on)));
        // Tap and drag needs tap to click, and drag lock needs both: they wait for them.
        let tap_and_drag = ui::toggle(
            pad.tap_and_drag,
            self.enabled(File::Touchpad, |on| Message::Set(Change::TapAndDrag(on)))
                .filter(|_| pad.tap_to_click),
        )
        .into();
        let drag_lock = ui::toggle(
            pad.drag_lock,
            self.enabled(File::Touchpad, |on| Message::Set(Change::DragLock(on)))
                .filter(|_| pad.tap_to_click && pad.tap_and_drag),
        )
        .into();
        let swipe = [
            (t::SWIPE_NOTHING, SwipeDown::Nothing),
            (t::SWIPE_MINIMIZE, SwipeDown::Minimize),
        ]
        .map(|(label, what)| {
            let choose = self.message(Change::SwipeDown(what));
            (label, pad.swipe_down == what, choose)
        });
        let mut gestures = vec![
            self.row(
                Setting::SwipeDown,
                width,
                t::SWIPE_DOWN_HELP,
                ui::segmented(swipe.to_vec()),
                true,
            ),
            self.row(
                Setting::TapAndDrag,
                width,
                t::TAP_AND_DRAG_HELP,
                tap_and_drag,
                false,
            ),
            self.row(
                Setting::DragLock,
                width,
                t::DRAG_LOCK_HELP,
                drag_lock,
                false,
            ),
        ];
        gestures.extend(
            t::FIXED_GESTURES
                .iter()
                .map(|(fingers, action)| super::keyboard::line(fingers, action)),
        );
        let speed = ui::slider(
            self.speed(),
            self.enabled(File::Touchpad, |v| Message::Set(Change::Speed(v))),
        );
        vec![
            self.card(
                Some(t::CLICKS),
                vec![
                    self.row(
                        Setting::RightClick,
                        width,
                        t::RIGHT_CLICK_HELP,
                        clicks,
                        true,
                    ),
                    self.row(Setting::TapToClick, width, t::TAP_TO_CLICK_HELP, tap, false),
                ],
            ),
            self.card(
                Some(t::SCROLL_AND_SPEED),
                vec![
                    self.row(
                        Setting::NaturalScroll,
                        width,
                        t::NATURAL_SCROLL_HELP,
                        natural,
                        false,
                    ),
                    self.row(Setting::Speed, width, t::SPEED_HELP, speed, true),
                    self.row(Setting::Typing, width, t::TYPING_HELP, typing, false),
                ],
            ),
            self.card(Some(t::GESTURES), gestures),
            self.card(
                Some(t::POINTER),
                vec![self.row(
                    Setting::ShowClicks,
                    width,
                    t::SHOW_CLICKS_HELP,
                    show_clicks,
                    false,
                )],
            ),
        ]
    }
}
