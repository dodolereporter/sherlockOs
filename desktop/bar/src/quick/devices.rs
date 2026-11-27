//! The Bluetooth devices of the control centre (D-20d), as for the Wi-Fi networks: the paired
//! ones, connected or disconnected with a click, forgotten after a confirmation; the ones a scan
//! finds while the list is open, paired with a click, the agent's questions in the panel.

use iced::futures::StreamExt;
use iced::futures::stream;
use iced::widget::{Column, column, container, row, space, text};
use iced::{Alignment, Element, Length, Task};
use sherlock_ui::component as ui;
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, Glyph, text as words};
use sherlock_ui::scroll::scroll;
use sherlock_ui::{metrics, typography};

use super::{Message, Page, Quick, Service, act, notice, on_system};
use crate::system::Problem;
use crate::system::bluetooth::{self, Asked, Device, Kind, Pairing};
use crate::system::{self};

impl Quick {
    /// Opens the list of devices and starts a scan, or goes back and stops it.
    pub(super) fn show_devices(&mut self, show: bool) -> Task<Message> {
        let page = if show { Page::Devices } else { Page::Main };
        self.turn_page(page, show);
        self.forgetting = None;
        if show {
            self.start_scan()
        } else {
            self.stop_scan()
        }
    }

    /// A scan for devices while the list is open and the adapter on; the devices found come
    /// with `InterfacesAdded`.
    pub(super) fn start_scan(&mut self) -> Task<Message> {
        let adapter = match self.bluetooth.ready() {
            Some(read) if read.powered && self.scanning.is_none() => read.adapter.clone(),
            _ => return Task::none(),
        };
        self.scanning = Some(adapter.clone());
        on_system(
            move |bus| async move { bluetooth::discover(&bus, &adapter, true).await },
            |r| match r {
                Ok(()) => Message::Changed(Service::Bluetooth),
                Err(problem) => Message::Done(Service::Bluetooth, Err(problem)),
            },
        )
    }

    pub(super) fn stop_scan(&mut self) -> Task<Message> {
        let Some(adapter) = self.scanning.take() else {
            return Task::none();
        };
        on_system(
            move |bus| async move { bluetooth::discover(&bus, &adapter, false).await },
            |r| {
                if let Err(problem) = r {
                    tracing::warn!(%problem, "the Bluetooth scan did not stop");
                }
                Message::Changed(Service::Bluetooth)
            },
        )
    }

    /// A click on a device: connected or disconnected when it is paired, else paired. One
    /// device at a time.
    pub(super) fn device(&mut self, path: String) -> Task<Message> {
        if self.device_busy.is_some() {
            return Task::none();
        }
        let Some(device) = self.find(&path).cloned() else {
            return Task::none();
        };
        self.forgetting = None;
        self.device_busy = Some(path.clone());
        if !device.paired {
            let pairing =
                stream::once(system::system_for_actions()).flat_map(move |bus| match bus {
                    Ok(bus) => bluetooth::pair(bus, path.clone()),
                    Err(problem) => stream::once(async { Pairing::Done(Err(problem)) }).boxed(),
                });
            let (task, handle) = Task::run(pairing, Message::Pairing).abortable();
            self.pairing = Some(handle);
            return task;
        }
        Task::perform(
            async move {
                let bus = system::system_for_actions().await?;
                if device.connected {
                    bluetooth::disconnect(&bus, &path).await
                } else {
                    bluetooth::connect(&bus, &path).await
                }
            },
            Message::DeviceDone,
        )
    }

    /// The connection, the disconnection or the pairing under way ended: only that frees the
    /// list for another device.
    pub(super) fn device_done(&mut self, result: Result<(), Problem>) -> Task<Message> {
        self.device_busy = None;
        self.pairing = None;
        self.asked = None;
        self.done(Service::Bluetooth, result)
    }

    /// The panel closes while a device pairs: the pairing stops at once. Dropping its stream
    /// cancels it in BlueZ and unregisters the agent (`bluetooth::pair`).
    pub(super) fn stop_pairing(&mut self) {
        if let Some((_, reply)) = self.asked.take() {
            reply.send(false);
        }
        if let Some(handle) = self.pairing.take() {
            handle.abort();
        }
        self.device_busy = None;
    }

    fn find(&self, path: &str) -> Option<&Device> {
        self.bluetooth
            .ready()
            .and_then(|read| read.devices.iter().find(|d| d.path == path))
    }

    /// What happens while a device pairs: a question of the agent, its cancellation, the end.
    pub(super) fn pairing(&mut self, event: Pairing) -> Task<Message> {
        match event {
            Pairing::Ask(asked, reply) => {
                if let Some((_, previous)) = self.asked.replace((asked, reply)) {
                    previous.send(false);
                }
                Task::none()
            }
            Pairing::Canceled => {
                self.asked = None;
                Task::none()
            }
            Pairing::Done(result) => self.device_done(result),
        }
    }

    /// « Confirmer » or « Annuler » under a question. A code only shown has nothing to answer:
    /// « Annuler » stops the pairing.
    pub(super) fn answer(&mut self, yes: bool) -> Task<Message> {
        let Some((asked, reply)) = self.asked.take() else {
            return Task::none();
        };
        if !matches!(asked, Asked::Show(_)) {
            reply.send(yes);
            return Task::none();
        }
        match (yes, self.device_busy.clone()) {
            (false, Some(device)) => on_system(
                move |bus| async move { bluetooth::cancel_pairing(&bus, &device).await },
                |_| Message::Changed(Service::Bluetooth),
            ),
            _ => Task::none(),
        }
    }

    /// « Oublier » confirmed or not.
    pub(super) fn forget(&mut self, yes: bool) -> Task<Message> {
        let path = self.forgetting.take();
        let adapter = self.bluetooth.ready().map(|read| read.adapter.clone());
        match (yes, path, adapter) {
            (true, Some(path), Some(adapter)) => act(Service::Bluetooth, move |bus| async move {
                bluetooth::forget(&bus, &adapter, &path).await
            }),
            _ => Task::none(),
        }
    }

    pub(super) fn view_devices(&self) -> Element<'_, Message> {
        let read = self.bluetooth.ready();
        let powered = read.is_some_and(|b| b.powered);
        let back = ui::icon_button(Icon::Back, words::BACK, Some(Message::ShowDevices(false)));
        let title = text(words::BLUETOOTH)
            .font(typography::TITLE)
            .size(panels::TITLE_SIZE);
        let switch: Option<fn(bool) -> Message> = read.map(|_| switch_bluetooth as fn(bool) -> _);
        let head = row![
            back,
            title,
            space::horizontal(),
            ui::toggle(powered, switch)
        ]
        .spacing(metrics::GAP_ICON)
        .align_y(Alignment::Center);
        let body: Element<'_, Message> = match read {
            Some(read) if read.powered => self.device_list(&read.devices),
            Some(_) => notice(words::BLUETOOTH_OFF_LINE),
            None => notice(words::UNAVAILABLE),
        };
        let mut page = column![head, body]
            .spacing(metrics::GAP_INLINE)
            .height(Length::Fill);
        if let Some(question) = self.question() {
            page = page.push(question);
        }
        if let Some(failed) = self.failed(Service::Bluetooth) {
            page = page.push(
                text(failed)
                    .font(typography::UI)
                    .size(typography::SMALL_SIZE)
                    .style(panels::error_text),
            );
        }
        page = page.push(super::settings_link(words::BLUETOOTH_SETTINGS, None));
        page.into()
    }

    fn device_list<'a>(&'a self, devices: &'a [Device]) -> Element<'a, Message> {
        let mut rows: Vec<Element<'a, Message>> = Vec::new();
        let (paired, found): (Vec<&Device>, Vec<&Device>) = devices.iter().partition(|d| d.paired);
        if !paired.is_empty() {
            rows.push(ui::section_label(words::MY_DEVICES).into());
            rows.extend(paired.into_iter().map(|d| self.device_row(d)));
        }
        let mut other = row![ui::section_label(words::OTHER_DEVICES), space::horizontal()]
            .align_y(Alignment::Center);
        if self.scanning.is_some() {
            other = other.push(ui::description(words::SEARCHING));
        }
        rows.push(other.into());
        if found.is_empty() {
            rows.push(ui::description(words::NO_DEVICE).into());
        }
        rows.extend(found.into_iter().map(|d| self.device_row(d)));
        scroll(Column::with_children(rows).spacing(metrics::GAP_TIGHT))
            .height(Length::Fill)
            .into()
    }

    fn device_row<'a>(&'a self, device: &'a Device) -> Element<'a, Message> {
        let busy = self.device_busy.as_deref() == Some(device.path.as_str());
        let line = match (busy, device.paired) {
            (true, true) => words::CONNECTING.to_owned(),
            (true, false) => words::PAIRING.to_owned(),
            (false, _) => words::device_line(device.connected, device.battery),
        };
        let forget = device.paired.then(|| {
            let ask = Message::Forget(device.path.clone());
            ui::icon_button(Icon::Trash, words::FORGET, Some(ask)).into()
        });
        let click = Some(Message::Device(device.path.clone()));
        let line = panels::device_row(glyph(device.kind), &device.name, line, click, forget);
        if self.forgetting.as_deref() != Some(device.path.as_str()) {
            return line;
        }
        let buttons = row![
            ui::danger_button(words::FORGET, Some(Message::ConfirmForget(true))),
            ui::secondary_button(words::CANCEL, Some(Message::ConfirmForget(false))),
        ]
        .spacing(metrics::GAP_INLINE / 2.0);
        let ask = column![
            ui::description(words::forget_question(&device.name)),
            buttons
        ]
        .spacing(metrics::GAP_TIGHT * 2.0);
        column![
            line,
            container(ask).padding([metrics::GAP_TIGHT * 2.0, metrics::MENU_ITEM_PADDING])
        ]
        .into()
    }

    /// The question of the agent, in a card under the list: the code, « Confirmer » and
    /// « Annuler ». Never answered by itself.
    fn question(&self) -> Option<Element<'_, Message>> {
        let (asked, _) = self.asked.as_ref()?;
        let name = self
            .device_busy
            .as_deref()
            .and_then(|path| self.find(path))
            .map_or("", |d| d.name.as_str());
        let (line, code, confirm) = match asked {
            Asked::Confirm(code) => (words::confirm_code(name), code, true),
            Asked::Type(code) => (words::type_code(name), code, true),
            Asked::Show(code) => (words::type_code(name), code, false),
        };
        let code = text(code.as_str())
            .font(typography::TITLE)
            .size(panels::CODE_SIZE);
        let card = column![ui::body(line), code].spacing(panels::CARD_GAP);
        let mut buttons = row![].spacing(metrics::GAP_INLINE / 2.0);
        if confirm {
            buttons = buttons.push(ui::primary_button(
                words::CONFIRM_BUTTON,
                Some(Message::Answer(true)),
            ));
        }
        buttons = buttons.push(ui::secondary_button(
            words::CANCEL,
            Some(Message::Answer(false)),
        ));
        let card = container(card.push(buttons))
            .width(Length::Fill)
            .padding(panels::CARD_PADDING)
            .style(panels::card);
        Some(card.into())
    }
}

fn switch_bluetooth(_: bool) -> Message {
    Message::ToggleBluetooth
}

/// The glyph of a kind of device.
fn glyph(kind: Kind) -> Glyph {
    match kind {
        Kind::Audio => Glyph::Headphones,
        Kind::Mouse => Glyph::Mouse,
        Kind::Keyboard => Glyph::Keyboard,
        Kind::Phone => Glyph::Phone,
        Kind::Computer => Glyph::Computer,
        Kind::Gamepad => Glyph::Gamepad,
        Kind::Other => Glyph::Bluetooth,
    }
}
