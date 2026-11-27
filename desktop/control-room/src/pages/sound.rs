//! The Sound page: the output and the microphone to use, each with its volume.

use iced::Element;
use iced::widget::{Column, container};
use sherlock_ui::component as ui;
use sherlock_ui::control_room::texts as t;
use sherlock_ui::metrics;
use system_services::audio::Device;

use crate::app::{App, Message, Sound};
use crate::search::Setting;
use crate::sound::{Change, Side};

impl App {
    pub(crate) fn sound(&self, width: f32) -> Vec<Element<'_, Message>> {
        let audio = match &self.sound {
            Sound::Ready(audio) => audio,
            Sound::Loading | Sound::Unavailable => {
                let note = match self.sound {
                    Sound::Loading => t::SOUND_LOADING,
                    _ => t::SOUND_UNAVAILABLE,
                };
                let note = container(ui::description(note)).padding([metrics::GAP_ICON, 0.0]);
                return vec![self.card(None, vec![note.into()])];
            }
        };
        vec![
            self.side(width, Device::Output, &audio.output),
            self.side(width, Device::Microphone, &audio.input),
        ]
    }

    /// The card of one direction: its devices to choose from, then its volume.
    fn side<'a>(&'a self, width: f32, device: Device, side: &'a Side) -> Element<'a, Message> {
        let (title, which, help, volume) = match device {
            Device::Output => (
                t::OUTPUT,
                Setting::OutputDevice,
                t::OUTPUT_DEVICE_HELP,
                Setting::OutputVolume,
            ),
            Device::Microphone => (
                t::INPUT,
                Setting::InputDevice,
                t::INPUT_DEVICE_HELP,
                Setting::InputVolume,
            ),
        };
        if side.devices.is_empty() {
            let none = container(ui::description(t::NO_DEVICE)).padding([metrics::GAP_ICON, 0.0]);
            return self.card(Some(title), vec![none.into()]);
        }
        let choices = side.devices.iter().map(|endpoint| {
            let chosen = endpoint.name == side.default;
            let choose = Change::Default(device, endpoint.name.clone());
            ui::radio(
                &endpoint.description,
                chosen,
                Some(Message::SetSound(choose)),
            )
            .into()
        });
        let choices = Column::with_children(choices).spacing(metrics::GAP_TIGHT);
        let slider = match device {
            Device::Output => ui::slider(
                side.volume,
                Some(|v| Message::SetSound(Change::Volume(Device::Output, v))),
            ),
            Device::Microphone => ui::slider(
                side.volume,
                Some(|v| Message::SetSound(Change::Volume(Device::Microphone, v))),
            ),
        };
        self.card(
            Some(title),
            vec![
                self.row(which, width, help, choices.into(), true),
                self.row(volume, width, t::VOLUME_HELP, slider, true),
            ],
        )
    }
}
