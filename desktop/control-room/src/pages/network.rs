//! The « Réseau » page (lot D-26n, audit C2): Wi-Fi on or off, the networks around, joining one
//! (a password, WEP, or an enterprise network as eduroam: what the bar sends here), a hidden
//! network by its name, and forgetting the network in use. Every call goes to NetworkManager,
//! off the interface thread (`crate::network`); polkit asks where its policy wants it.

use iced::widget::{Column, container, row, space};
use iced::{Alignment, Element, Length, Task};
use sherlock_ui::component as ui;
use sherlock_ui::control_room::texts as t;
use sherlock_ui::icon::Icon;
use sherlock_ui::metrics;
use system_services::Problem;
use system_services::network::{Credentials, Eap, Network, Security, Wifi};

use crate::app::{App, Message};
use crate::network as nm;
use crate::search::Setting;

/// What the page knows of the Wi-Fi.
#[derive(Debug, Default)]
pub struct Net {
    /// `None` while it is read the first time; `Some(None)`: NetworkManager does not answer.
    pub(crate) wifi: Option<Option<Wifi>>,
    pub(crate) form: Option<Form>,
    /// An action is on its way: the buttons wait.
    pub(crate) busy: bool,
    pub(crate) problem: Option<&'static str>,
}

/// How the network to join is protected, as the form asks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Open,
    Password,
    Wep,
    Enterprise,
}

/// The form of a network to join: one of the list (`point`), or a hidden one by its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    pub(crate) ssid: String,
    /// The network of the scan; `None` for a hidden one.
    pub(crate) network: Option<Network>,
    pub(crate) kind: Kind,
    pub(crate) method: Eap,
    pub(crate) identity: String,
    pub(crate) password: String,
    pub(crate) domain: String,
}

impl Form {
    fn new(network: Option<Network>, kind: Kind) -> Self {
        Self {
            ssid: network.as_ref().map(|n| n.ssid.clone()).unwrap_or_default(),
            network,
            kind,
            method: Eap::Peap,
            identity: String::new(),
            password: String::new(),
            domain: String::new(),
        }
    }

    /// Everything the network needs is typed.
    pub(crate) fn ready(&self) -> bool {
        let named = !self.ssid.trim().is_empty();
        named
            && match self.kind {
                Kind::Open => true,
                Kind::Password | Kind::Wep => !self.password.is_empty(),
                Kind::Enterprise => !self.identity.is_empty() && !self.password.is_empty(),
            }
    }

    pub(crate) fn credentials(&self) -> Credentials {
        match self.kind {
            Kind::Open => Credentials::None,
            Kind::Password => Credentials::Password(self.password.clone()),
            Kind::Wep => Credentials::Wep(self.password.clone()),
            Kind::Enterprise => Credentials::Enterprise {
                method: self.method,
                identity: self.identity.trim().to_owned(),
                password: self.password.clone(),
                domain: Some(self.domain.trim().to_owned()).filter(|d| !d.is_empty()),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub enum Change {
    Read(Result<Wifi, Problem>),
    Tick,
    Toggle(bool),
    Scan,
    Pick(Network),
    /// Joined through its saved connection (`true`), or a form must ask.
    Rejoined(Network, Result<bool, Problem>),
    Other,
    Ssid(String),
    Kind(Kind),
    Method(Eap),
    Identity(String),
    Password(String),
    Domain(String),
    Join,
    Cancel,
    Forget(String),
    Done(Result<(), Problem>),
}

fn net(change: Change) -> Message {
    Message::Network(change)
}

fn read() -> Task<Message> {
    Task::perform(nm::read(), |read| net(Change::Read(read)))
}

/// What the page says of a problem.
fn say(problem: &Problem) -> &'static str {
    match problem {
        Problem::Refused(_) => t::NETWORK_REFUSED,
        Problem::Failed(text) if text == nm::WRONG_PASSWORD => t::NETWORK_WRONG_PASSWORD,
        _ => t::NETWORK_FAILED,
    }
}

impl App {
    /// Reads the Wi-Fi when the page opens.
    pub(crate) fn open_network(&mut self) -> Task<Message> {
        read()
    }

    pub(crate) fn update_network(&mut self, change: Change) -> Task<Message> {
        let n = &mut self.network;
        let device = n
            .wifi
            .as_ref()
            .and_then(|w| w.as_ref())
            .and_then(|w| w.device.clone());
        let act = |n: &mut Net, task: Task<Message>| {
            n.busy = true;
            n.problem = None;
            task
        };
        match change {
            Change::Read(read) => {
                n.wifi = Some(read.map_err(|e| tracing::warn!(%e, "Wi-Fi not read")).ok());
            }
            Change::Tick if !n.busy => return read(),
            Change::Tick => {}
            Change::Toggle(on) => {
                let done = Task::perform(nm::enable(on), |r| net(Change::Done(r)));
                return act(n, done);
            }
            Change::Scan => {
                if let Some(device) = device {
                    let done = Task::perform(nm::scan(device), |r| net(Change::Done(r)));
                    return act(n, done);
                }
            }
            Change::Pick(network) => {
                let Some(device) = device else {
                    return Task::none();
                };
                n.form = None;
                if network.security == Security::Open {
                    let done = nm::join(
                        device,
                        network.ssid.clone(),
                        Some(network.path.clone()),
                        Credentials::None,
                    );
                    return act(n, Task::perform(done, |r| net(Change::Done(r))));
                }
                let again = network.clone();
                let done = Task::perform(nm::rejoin(device, network), move |r| {
                    net(Change::Rejoined(again.clone(), r))
                });
                return act(n, done);
            }
            Change::Rejoined(network, joined) => {
                n.busy = false;
                match joined {
                    Ok(true) => return read(),
                    Ok(false) => {
                        // WEP and 802.1X say the same flags: the form starts on the one met most.
                        let kind = match network.security {
                            Security::Other => Kind::Enterprise,
                            _ => Kind::Password,
                        };
                        n.form = Some(Form::new(Some(network), kind));
                    }
                    Err(problem) => n.problem = Some(say(&problem)),
                }
            }
            Change::Other => n.form = Some(Form::new(None, Kind::Password)),
            Change::Cancel => n.form = None,
            Change::Ssid(text) => edit(n, |f| f.ssid = bounded(text, 32)),
            Change::Kind(kind) => edit(n, |f| f.kind = kind),
            Change::Method(method) => edit(n, |f| f.method = method),
            Change::Identity(text) => edit(n, |f| f.identity = bounded(text, 256)),
            Change::Password(text) => edit(n, |f| f.password = bounded(text, 256)),
            Change::Domain(text) => edit(n, |f| f.domain = bounded(text, 256)),
            Change::Join => {
                let (Some(form), Some(device)) = (n.form.clone(), device) else {
                    return Task::none();
                };
                if !form.ready() {
                    return Task::none();
                }
                let task = match (&form.network, form.kind) {
                    // Visible, with a password: WPA2 or WPA3 as the network says.
                    (Some(network), Kind::Password) => {
                        let done = nm::connect(device, network.clone(), form.password.clone());
                        Task::perform(done, |r| net(Change::Done(r)))
                    }
                    (network, _) => {
                        let point = network.as_ref().map(|n| n.path.clone());
                        let done = nm::join(
                            device,
                            form.ssid.trim().to_owned(),
                            point,
                            form.credentials(),
                        );
                        Task::perform(done, |r| net(Change::Done(r)))
                    }
                };
                n.form = None;
                return act(n, task);
            }
            Change::Forget(ssid) => {
                let done = Task::perform(nm::forget(ssid), |r| net(Change::Done(r)));
                return act(n, done);
            }
            Change::Done(result) => {
                n.busy = false;
                if let Err(problem) = result {
                    tracing::warn!(%problem, "Wi-Fi action");
                    n.problem = Some(say(&problem));
                }
                return read();
            }
        }
        Task::none()
    }

    pub(crate) fn network_page(&self, width: f32) -> Vec<Element<'_, Message>> {
        let n = &self.network;
        let note = |text| {
            let note = container(ui::description(text)).padding([metrics::GAP_ICON, 0.0]);
            vec![self.card(None, vec![note.into()])]
        };
        let wifi = match &n.wifi {
            None => return note(t::NETWORK_LOADING),
            Some(None) => return note(t::NETWORK_UNAVAILABLE),
            Some(Some(wifi)) => wifi,
        };
        if wifi.device.is_none() {
            return note(t::NO_WIFI);
        }
        let switch: fn(bool) -> Message = |on| net(Change::Toggle(on));
        let toggle = ui::toggle(
            wifi.enabled,
            (!n.busy && !wifi.hardware_off).then_some(switch),
        );
        let help = match wifi.hardware_off {
            true => t::WIFI_HARDWARE_OFF,
            false => t::WIFI_HELP,
        };
        let mut first = vec![self.row(Setting::Wifi, width, help, toggle.into(), false)];
        if let Some(problem) = n.problem {
            first.push(ui::description(problem).into());
        }
        let mut cards = vec![self.card(None, first)];
        if !wifi.enabled {
            return cards;
        }
        let mut rows: Vec<Element<'_, Message>> = Vec::new();
        if wifi.networks.is_empty() {
            rows.push(ui::description(t::NO_NETWORK).into());
        }
        for network in &wifi.networks {
            let detail = match (network.active, network.security) {
                (true, _) => t::CONNECTED,
                (_, Security::Open) => t::OPEN_NETWORK,
                (_, Security::Other) => t::ENTERPRISE_NETWORK,
                _ => t::PROTECTED_NETWORK,
            };
            let trailing: Option<Element<'_, Message>> = network.active.then(|| {
                let forget = (!n.busy).then(|| net(Change::Forget(network.ssid.clone())));
                ui::secondary_button(t::FORGET, forget).into()
            });
            let pick = (!n.busy && !network.active).then(|| net(Change::Pick(network.clone())));
            rows.push(
                ui::list_item(Icon::Network, &network.ssid, Some(detail), trailing, pick).into(),
            );
            if let Some(form) = &n.form
                && form
                    .network
                    .as_ref()
                    .is_some_and(|f| f.path == network.path)
            {
                rows.push(self.form(form));
            }
        }
        if let Some(form) = n.form.as_ref().filter(|f| f.network.is_none()) {
            rows.push(self.form(form));
        }
        let scan = (!n.busy).then(|| net(Change::Scan));
        let other = (!n.busy && n.form.is_none()).then(|| net(Change::Other));
        let buttons = row![
            ui::secondary_button(t::SCAN, scan),
            ui::secondary_button(t::OTHER_NETWORK, other),
            space::horizontal(),
        ]
        .spacing(metrics::GAP_ICON);
        rows.push(container(buttons).padding([metrics::GAP_ICON, 0.0]).into());
        cards.push(self.card(Some(t::NETWORKS), rows));
        cards
    }

    /// The fields a network needs, then « Annuler » and « Rejoindre ».
    fn form<'a>(&'a self, form: &'a Form) -> Element<'a, Message> {
        let mut lines = Column::new().spacing(metrics::GAP_ICON);
        let visible = form.network.as_ref();
        if visible.is_none() {
            lines = lines.push(ui::text_field(
                t::NETWORK_NAME,
                &form.ssid,
                Some(|s| net(Change::Ssid(s))),
            ));
        }
        let kinds: &[(Kind, &str)] = match visible.map(|n| n.security) {
            None => &[
                (Kind::Open, t::SECURITY_NONE),
                (Kind::Password, t::SECURITY_PASSWORD),
                (Kind::Wep, t::SECURITY_WEP),
                (Kind::Enterprise, t::SECURITY_ENTERPRISE),
            ],
            Some(Security::Other) => &[
                (Kind::Enterprise, t::SECURITY_ENTERPRISE),
                (Kind::Wep, t::SECURITY_WEP),
            ],
            Some(_) => &[],
        };
        if !kinds.is_empty() {
            let choices = kinds
                .iter()
                .map(|(kind, label)| (*label, form.kind == *kind, Some(net(Change::Kind(*kind)))))
                .collect();
            lines = lines.push(ui::segmented(choices));
        }
        if form.kind == Kind::Enterprise {
            let methods = [(Eap::Peap, t::PEAP), (Eap::Ttls, t::TTLS)]
                .into_iter()
                .map(|(m, label)| (label, form.method == m, Some(net(Change::Method(m)))))
                .collect();
            lines = lines.push(ui::segmented(methods)).push(ui::text_field(
                t::IDENTITY,
                &form.identity,
                Some(|s| net(Change::Identity(s))),
            ));
        }
        if form.kind != Kind::Open {
            let label = match form.kind {
                Kind::Wep => t::WEP_KEY,
                _ => t::PASSWORD,
            };
            let field = ui::text_field(label, &form.password, Some(|s| net(Change::Password(s))))
                .secure(true)
                .on_submit(net(Change::Join));
            lines = lines.push(field);
        }
        if form.kind == Kind::Enterprise {
            lines = lines
                .push(ui::text_field(
                    t::DOMAIN,
                    &form.domain,
                    Some(|s| net(Change::Domain(s))),
                ))
                .push(ui::description(t::DOMAIN_HELP));
        }
        let join = (form.ready() && !self.network.busy).then(|| net(Change::Join));
        let buttons = row![
            space::horizontal(),
            ui::secondary_button(t::CANCEL, Some(net(Change::Cancel))),
            ui::primary_button(t::JOIN, join),
        ]
        .spacing(metrics::GAP_ICON)
        .align_y(Alignment::Center);
        container(lines.push(buttons))
            .width(Length::Fill)
            .padding([metrics::GAP_ICON, metrics::LIST_ROW_PADDING])
            .into()
    }
}

fn edit(n: &mut Net, change: impl FnOnce(&mut Form)) {
    if let Some(form) = n.form.as_mut() {
        change(form);
    }
}

/// `text` cut to `max` bytes, on a character.
fn bounded(mut text: String, max: usize) -> String {
    while text.len() > max {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_form_is_ready_once_its_network_has_what_it_needs() {
        let mut form = Form::new(None, Kind::Enterprise);
        assert!(!form.ready());
        form.ssid = String::from("eduroam");
        form.identity = String::from("moi@universite.fr");
        assert!(!form.ready());
        form.password = String::from("secret");
        assert!(form.ready());
        let Credentials::Enterprise { domain, .. } = form.credentials() else {
            panic!("enterprise");
        };
        assert_eq!(domain, None);
        form.kind = Kind::Open;
        assert_eq!(form.credentials(), Credentials::None);
        assert_eq!(bounded("é".repeat(20), 5), "éé");
    }
}
