//! The Accessibility page (lot D-26p): reduce motion, reduce transparency, increase contrast,
//! and the visual effects (the light mode). In the appearance file, which every program of the
//! shell and the compositor follow live and apply to themselves.

use appearance_config::Effects;
use iced::Element;
use sherlock_ui::component as ui;
use sherlock_ui::control_room::texts as t;

use crate::app::{App, Change, Message};
use crate::search::Setting;
use crate::settings::File;

impl App {
    pub(crate) fn accessibility(&self, width: f32) -> Vec<Element<'_, Message>> {
        let a = self.settings.appearance.accessibility;
        let toggle = |on: bool, change: fn(bool) -> Message| {
            ui::toggle(on, self.enabled(File::Appearance, change)).into()
        };
        let effects = [
            (t::EFFECTS_FULL, Effects::Full),
            (t::EFFECTS_REDUCED, Effects::Reduced),
            (t::EFFECTS_NONE, Effects::None),
        ]
        .map(|(label, which)| {
            let choose = self.message(Change::Effects(which));
            (label, a.effects == which, choose)
        });
        let rows = vec![
            self.row(
                Setting::ReduceMotion,
                width,
                t::REDUCE_MOTION_HELP,
                toggle(a.reduce_motion, |on| Message::Set(Change::ReduceMotion(on))),
                false,
            ),
            self.row(
                Setting::ReduceTransparency,
                width,
                t::REDUCE_TRANSPARENCY_HELP,
                toggle(a.reduce_transparency, |on| {
                    Message::Set(Change::ReduceTransparency(on))
                }),
                false,
            ),
            self.row(
                Setting::IncreaseContrast,
                width,
                t::INCREASE_CONTRAST_HELP,
                toggle(a.increase_contrast, |on| {
                    Message::Set(Change::IncreaseContrast(on))
                }),
                false,
            ),
            self.row(
                Setting::Effects,
                width,
                t::EFFECTS_HELP,
                ui::segmented(effects.to_vec()),
                true,
            ),
        ];
        vec![self.card(Some(t::DISPLAY_CARD), rows)]
    }
}
