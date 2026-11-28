//! The Bluetooth page (lot D-26o): the switch, the paired devices (connect, disconnect, rename,
//! forget), the devices around found by the scan that runs while the page is open (pair), and
//! the agent's question while a device pairs.

use iced::widget::{Column, column, container, row, space, text};
use iced::{Alignment, Element, Length};
use sherlock_ui::component as ui;
use sherlock_ui::control_room::texts as t;
use sherlock_ui::panels::{self, Glyph, text as words};
use sherlock_ui::{color, ellipsis::ellipsis, metrics, typography};
use system_services::Problem;
use system_services::bluetooth::{Asked, Bluetooth, Device, Kind};

use crate::app::{App, Message};
use crate::bluetooth::{Msg, Read};
use crate::search::Setting;

fn bt(message: Msg) -> Message {
    Message::Bluetooth(message)
}

impl App {
    pub(crate) fn bluetooth_page(&self, width: f32) -> Vec<Element<'_, Message>> {
        let state = &self.bluetooth;
        let read = match &state.read {
            Read::Ready(read) => read,
            other => {
                let note = match other {
                    Read::Loading => t::BLUETOOTH_LOADING,
                    _ => t::BLUETOOTH_UNAVAILABLE,
                };
                let note = container(ui::description(note)).padding([metrics::GAP_ICON, 0.0]);
                return vec![self.card(None, vec![note.into()])];
            }
        };
        let switch = ui::toggle(read.powered, Some(|on| bt(Msg::Power(on))));
        let help = match &state.failure {
            Some(Problem::Refused(_)) => t::BLUETOOTH_REFUSED,
            Some(_) => t::BLUETOOTH_FAILED,
            None => t::BLUETOOTH_SWITCH_HELP,
        };
        let mut cards = vec![self.card(
            None,
            vec![self.row(Setting::Bluetooth, width, help, switch.into(), false)],
        )];
        if !read.powered {
            let note =
                container(ui::description(t::BLUETOOTH_OFF_NOTE)).padding([metrics::GAP_ICON, 0.0]);
            cards.push(self.card(None, vec![note.into()]));
            return cards;
        }
        if let Some(question) = self.question(read) {
            cards.push(question);
        }
        let (paired, found): (Vec<&Device>, Vec<&Device>) =
            read.devices.iter().partition(|d| d.paired);
        let mut mine: Vec<Element<'_, Message>> = paired.iter().map(|d| self.paired(d)).collect();
        if mine.is_empty() {
            mine.push(ui::description(t::NO_PAIRED).into());
        }
        cards.push(self.card(Some(t::MY_DEVICES), mine));
        let mut around: Vec<Element<'_, Message>> = found.iter().map(|d| self.found(d)).collect();
        if around.is_empty() {
            around.push(ui::description(t::NOTHING_NEARBY).into());
        }
        let searching = state.scanning.is_some().then(|| {
            row![
                ui::body(t::NEARBY)
                    .size(typography::CARD_TITLE_SIZE)
                    .font(typography::UI_SEMIBOLD),
                space::horizontal(),
                ui::description(t::SEARCHING),
            ]
            .align_y(Alignment::Center)
        });
        match searching {
            Some(head) => {
                around.insert(0, head.into());
                cards.push(self.card(None, around));
            }
            None => cards.push(self.card(Some(t::NEARBY), around)),
        }
        cards
    }

    /// A paired device: its glyph, its name, its state and battery, then « Connecter » or
    /// « Déconnecter », « Renommer » and « Oublier »; the name field or the question to forget
    /// under it when asked.
    fn paired<'a>(&'a self, device: &'a Device) -> Element<'a, Message> {
        let state = &self.bluetooth;
        let busy = state.busy.as_deref() == Some(device.path.as_str());
        let line = if busy {
            words::CONNECTING.to_owned()
        } else {
            words::device_line(device.connected, device.battery)
        };
        let path = device.path.clone();
        let action = if device.connected {
            t::DISCONNECT
        } else {
            t::CONNECT
        };
        let free = state.busy.is_none();
        let buttons = row![
            ui::secondary_button(action, free.then(|| bt(Msg::Device(path.clone())))),
            ui::secondary_button(t::RENAME, Some(bt(Msg::Rename(path.clone())))),
            ui::secondary_button(t::FORGET, Some(bt(Msg::Forget(path.clone())))),
        ]
        .spacing(metrics::GAP_TIGHT * 3.0);
        let mut lines = column![device_head(device, line), buttons].spacing(metrics::GAP_ICON);
        if let Some((renamed, name)) = &state.renaming
            && *renamed == device.path
        {
            let field = ui::text_field(t::DEVICE_NAME, name, Some(|n| bt(Msg::Name(n))))
                .on_submit(bt(Msg::ConfirmRename(true)))
                .width(Length::Fill);
            lines = lines.push(
                row![
                    field,
                    ui::primary_button(t::OK, Some(bt(Msg::ConfirmRename(true)))),
                    ui::secondary_button(t::CANCEL, Some(bt(Msg::ConfirmRename(false)))),
                ]
                .spacing(metrics::GAP_TIGHT * 3.0)
                .align_y(Alignment::Center),
            );
        }
        if state.forgetting.as_deref() == Some(device.path.as_str()) {
            lines = lines.push(
                column![
                    ui::description(words::forget_question(&device.name)),
                    row![
                        ui::danger_button(t::FORGET, Some(bt(Msg::ConfirmForget(true)))),
                        ui::secondary_button(t::CANCEL, Some(bt(Msg::ConfirmForget(false)))),
                    ]
                    .spacing(metrics::GAP_TIGHT * 3.0),
                ]
                .spacing(metrics::GAP_TIGHT * 2.0),
            );
        }
        container(lines).padding([metrics::GAP_ICON, 0.0]).into()
    }

    /// A device around: its glyph and its name, « Appairer ».
    fn found<'a>(&'a self, device: &'a Device) -> Element<'a, Message> {
        let state = &self.bluetooth;
        let busy = state.busy.as_deref() == Some(device.path.as_str());
        let line = if busy {
            words::PAIRING.to_owned()
        } else {
            String::new()
        };
        let pair = state
            .busy
            .is_none()
            .then(|| bt(Msg::Device(device.path.clone())));
        row![
            container(device_head(device, line)).width(Length::Fill),
            ui::secondary_button(t::PAIR, pair),
        ]
        .spacing(metrics::GAP_ROW)
        .align_y(Alignment::Center)
        .padding([metrics::GAP_ICON, 0.0])
        .into()
    }

    /// The agent's question while a device pairs: its code, « Confirmer » and « Annuler ».
    fn question<'a>(&'a self, read: &'a Bluetooth) -> Option<Element<'a, Message>> {
        let state = &self.bluetooth;
        let (asked, _) = state.asked.as_ref()?;
        let name = state
            .busy
            .as_deref()
            .and_then(|path| read.devices.iter().find(|d| d.path == path))
            .map_or("", |d| d.name.as_str());
        let (line, code, confirm) = match asked {
            Asked::Confirm(code) => (words::confirm_code(name), code, true),
            Asked::Type(code) => (words::type_code(name), code, true),
            Asked::Show(code) => (words::type_code(name), code, false),
        };
        let code = text(code.as_str())
            .font(typography::TITLE)
            .size(panels::CODE_SIZE);
        let mut buttons = row![].spacing(metrics::GAP_TIGHT * 3.0);
        if confirm {
            buttons = buttons.push(ui::primary_button(t::CONFIRM, Some(bt(Msg::Answer(true)))));
        }
        buttons = buttons.push(ui::secondary_button(
            t::CANCEL,
            Some(bt(Msg::Answer(false))),
        ));
        let lines = Column::new()
            .push(ui::body(line))
            .push(code)
            .push(buttons)
            .spacing(metrics::GAP_ICON);
        Some(self.card(None, vec![lines.into()]))
    }
}

/// The glyph of a kind of device, its name (« … » when too long) and `line` under it.
fn device_head<'a>(device: &'a Device, line: String) -> Element<'a, Message> {
    let p = color::palette();
    let mut texts = column![ellipsis(device.name.as_str()).font(typography::UI_MEDIUM)];
    if !line.is_empty() {
        texts = texts.push(
            ellipsis(line)
                .size(typography::SMALL_SIZE)
                .color(p.text_secondary),
        );
    }
    row![
        panels::glyph(glyph(device.kind), p.text_secondary),
        texts.width(Length::Fill)
    ]
    .spacing(metrics::GAP_ICON)
    .align_y(Alignment::Center)
    .into()
}

/// The glyph of a kind of device, as in the bar.
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
