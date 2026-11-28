//! The outputs and the inputs of the sound in the control centre (D-20g), as the Sound module of
//! macOS: the chevron of the volume row opens them, the output in use and the input in use
//! checked, a click moving the sound to another (speakers, headphones, HDMI, Bluetooth), and the
//! microphone's slider and mute switch. Everything goes through `pactl` on a thread of its own
//! (`blocking`), one call of the microphone's slider at a time, and the page follows the
//! changes made elsewhere (`pactl subscribe`, `watch`).

use iced::widget::{Column, column, row, space, text};
use iced::{Alignment, Element, Length, Task};
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, Glyph, text as words};
use sherlock_ui::scroll::scroll;
use sherlock_ui::{metrics, typography};

use super::{Load, Message, Page, Quick, Service, blocking};
use crate::system;
use crate::system::audio::{Device, Endpoint, Pactl, Sound};

/// Outputs and inputs listed at most: a sound server could name hundreds.
pub const MAX_DEVICES: usize = 16;

/// The outputs and the inputs, the ones in use, and the microphone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub outputs: Vec<Endpoint>,
    pub inputs: Vec<Endpoint>,
    pub output: String,
    pub input: String,
    /// The volume and mute switch of the input in use, without one `None`.
    pub microphone: Option<Sound>,
}

/// Reads them all through `pactl`: blocking, for a thread of its own.
pub fn read(pactl: &Pactl) -> system::Result<Devices> {
    let mut outputs = pactl.endpoints(Device::Output)?;
    outputs.truncate(MAX_DEVICES);
    let mut inputs = pactl.endpoints(Device::Microphone).unwrap_or_default();
    inputs.truncate(MAX_DEVICES);
    let output = pactl.default_name(Device::Output).unwrap_or_default();
    let input = pactl.default_name(Device::Microphone).unwrap_or_default();
    let microphone = if inputs.is_empty() {
        None
    } else {
        pactl.sound(Device::Microphone).ok()
    };
    Ok(Devices {
        outputs,
        inputs,
        output,
        input,
        microphone,
    })
}

impl Quick {
    /// Reads the outputs and the inputs, off the interface thread.
    pub(super) fn read_devices(&self) -> Task<Message> {
        let pactl = self.pactl.clone();
        blocking(
            move || read(&pactl),
            |r| Message::Audio(Load::from_result(r)),
        )
    }

    /// The page of the outputs and inputs, or back.
    pub(super) fn show_sound(&mut self, show: bool) -> Task<Message> {
        let page = if show { Page::Sound } else { Page::Main };
        self.turn_page(page, show);
        if show {
            self.read(Service::Microphone)
        } else {
            Task::none()
        }
    }

    /// Moves the sound to the output, or takes the input, `name`.
    pub(super) fn choose(&mut self, device: Device, name: String) -> Task<Message> {
        if let Load::Ready(devices) = &mut self.audio {
            match device {
                Device::Output => devices.output.clone_from(&name),
                Device::Microphone => devices.input.clone_from(&name),
            }
        }
        let pactl = self.pactl.clone();
        blocking(
            move || pactl.set_default(device, &name),
            |r| match r {
                // The output and its volume, and the microphone, read again.
                Ok(()) => Message::Changed(Service::Sound),
                Err(problem) => Message::Done(Service::Microphone, Err(problem)),
            },
        )
    }

    /// The microphone's slider moved: one value at a time to the server, the latest winning.
    pub(super) fn microphone_volume(&mut self, value: u8) -> Task<Message> {
        self.held = Some(Service::Microphone);
        if let Load::Ready(Devices {
            microphone: Some(sound),
            ..
        }) = &mut self.audio
        {
            sound.volume = u16::from(value);
        }
        match self.microphone_calls.want(value) {
            Some(value) => self.send_microphone(value),
            None => Task::none(),
        }
    }

    pub(super) fn send_microphone(&self, value: u8) -> Task<Message> {
        let pactl = self.pactl.clone();
        blocking(
            move || pactl.set_volume_of(Device::Microphone, value),
            |r| Message::Done(Service::Microphone, r),
        )
    }

    /// The microphone muted, or back.
    pub(super) fn mute_microphone(&mut self) -> Task<Message> {
        let Load::Ready(Devices {
            microphone: Some(sound),
            ..
        }) = &mut self.audio
        else {
            return Task::none();
        };
        sound.muted = !sound.muted;
        let muted = sound.muted;
        let pactl = self.pactl.clone();
        blocking(
            move || pactl.mute(Device::Microphone, muted),
            |r| Message::Done(Service::Microphone, r),
        )
    }

    /// A read of the devices: the microphone's slider stays where it is while held or on its
    /// way to the server.
    pub(super) fn devices_read(&mut self, mut read: Load<Devices>) -> Task<Message> {
        if let (Load::Ready(new), Some(old)) = (&mut read, self.audio.ready())
            && self.settling(Service::Microphone)
            && let (Some(new), Some(old)) = (&mut new.microphone, old.microphone)
        {
            new.volume = old.volume;
        }
        self.audio = read;
        self.read_ended(Service::Microphone)
    }

    pub(super) fn view_sound(&self) -> Element<'_, Message> {
        let back = ui::icon_button(Icon::Back, words::BACK, Some(Message::ShowSound(false)));
        let title = text(words::SOUND)
            .font(typography::TITLE)
            .size(panels::TITLE_SIZE);
        let head = row![back, title, space::horizontal()]
            .spacing(metrics::GAP_ICON)
            .align_y(Alignment::Center);
        let Some(devices) = self.audio.ready() else {
            let line = match &self.audio {
                Load::Failed(_) => words::NO_OUTPUT,
                _ => words::LOADING,
            };
            return column![head, super::notice(line)]
                .spacing(metrics::GAP_INLINE)
                .height(Length::Fill)
                .into();
        };
        let mut rows: Vec<Element<'_, Message>> = vec![ui::section_label(words::OUTPUT).into()];
        rows.extend(list(&devices.outputs, &devices.output, Device::Output));
        rows.push(ui::section_label(words::INPUT).into());
        if devices.inputs.is_empty() {
            rows.push(ui::description(words::NO_INPUT).into());
        }
        rows.extend(list(&devices.inputs, &devices.input, Device::Microphone));
        if let Some(sound) = devices.microphone {
            let (glyph, label) = if sound.muted {
                (Glyph::MicrophoneMuted, words::UNMUTE_MICROPHONE)
            } else {
                (Glyph::Microphone, words::MUTE_MICROPHONE)
            };
            let value = u8::try_from(sound.volume.min(100)).unwrap_or(100);
            rows.push(panels::held_slider_row(
                panels::glyph_button(glyph, label, Some(Message::MuteMicrophone)),
                value,
                Some(Message::MicrophoneVolume as fn(u8) -> Message),
                Some(Message::Release(Service::Microphone)),
            ));
        }
        let body =
            scroll(Column::with_children(rows).spacing(metrics::GAP_TIGHT)).height(Length::Fill);
        let mut page = column![head, body]
            .spacing(metrics::GAP_INLINE)
            .height(Length::Fill);
        if let Some(failed) = self.failed(Service::Microphone) {
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

/// The lines of `endpoints`, the one named `chosen` checked; a click takes another.
fn list<'a>(
    endpoints: &'a [Endpoint],
    chosen: &str,
    device: Device,
) -> impl Iterator<Item = Element<'a, Message>> {
    let chosen = chosen.to_owned();
    endpoints.iter().map(move |endpoint| {
        let (trailing, pick) = if endpoint.name == chosen {
            (Trailing::Checked, None)
        } else {
            let pick = match device {
                Device::Output => Message::ChooseOutput(endpoint.name.clone()),
                Device::Microphone => Message::ChooseInput(endpoint.name.clone()),
            };
            (Trailing::None, Some(pick))
        };
        // The one in use takes no click, but does not look disabled.
        let press = pick.or(Some(Message::ShowSound(true)));
        ui::menu_item_with(None, endpoint.description.as_str(), trailing, press).into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_microphone_does_not_jump_and_its_calls_go_one_at_a_time() {
        let mut quick = Quick::default();
        let devices = Devices {
            outputs: vec![Endpoint {
                name: String::from("speakers"),
                description: String::from("Haut-parleurs intégrés"),
            }],
            inputs: vec![Endpoint {
                name: String::from("mic"),
                description: String::from("Microphone intégré"),
            }],
            output: String::from("speakers"),
            input: String::from("mic"),
            microphone: Some(Sound {
                volume: 40,
                muted: false,
            }),
        };
        let _ = quick.update(Message::Audio(Load::Ready(devices.clone())));
        let _ = quick.update(Message::MicrophoneVolume(70));
        let _ = quick.update(Message::MicrophoneVolume(80));
        // A read meanwhile keeps the slider where it is held.
        let _ = quick.update(Message::Audio(Load::Ready(devices)));
        let held = quick
            .audio
            .ready()
            .and_then(|d| d.microphone)
            .map(|s| s.volume);
        assert_eq!(held, Some(80));
        assert!(quick.microphone_calls.busy());
        let _ = quick.update(Message::Done(Service::Microphone, Ok(())));
        let _ = quick.update(Message::Done(Service::Microphone, Ok(())));
        assert!(!quick.microphone_calls.busy());
        let _ = quick.update(Message::MuteMicrophone);
        let muted = quick
            .audio
            .ready()
            .and_then(|d| d.microphone)
            .map(|s| s.muted);
        assert_eq!(muted, Some(true));
        // Choosing another output shows it at once.
        let _ = quick.update(Message::ChooseOutput(String::from("hdmi")));
        assert_eq!(quick.audio.ready().map(|d| d.output.as_str()), Some("hdmi"));
    }
}
