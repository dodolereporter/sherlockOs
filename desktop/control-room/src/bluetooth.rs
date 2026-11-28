//! The Bluetooth page's view of BlueZ (lot D-26o), through `system_services::bluetooth`, the
//! code the bar's quick settings use: the adapter switched on and off, the paired devices
//! connected, disconnected, renamed and forgotten, the devices around found by a scan while the
//! page is open and paired with the agent's code. Every call runs on the system bus in a task,
//! off the interface thread, with a time limit: 5 s for a read, 60 s for an action, for polkit
//! to ask. The page follows what BlueZ announces (`bluetooth::changes`).

use std::sync::OnceLock;
use std::time::Duration;

use iced::Task;
use iced::futures::stream::{self, BoxStream};
use iced::futures::{Future, StreamExt};
use system_services::Problem;
use system_services::bluetooth::{self, Asked, Bluetooth, Pairing, Reply};
use zbus::Connection;

/// Longest wait for a read, and for an action (polkit may ask the user).
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(60);

async fn open(cell: &'static OnceLock<Connection>, timeout: Duration) -> zbus::Result<Connection> {
    if let Some(bus) = cell.get() {
        return Ok(bus.clone());
    }
    let bus = zbus::connection::Builder::system()?
        .method_timeout(timeout)
        .build()
        .await?;
    Ok(cell.get_or_init(|| bus).clone())
}

/// The system bus for reads, and for actions; opened on first use, following
/// `DBUS_SYSTEM_BUS_ADDRESS` (the tests' private bus).
pub async fn bus() -> zbus::Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    open(&BUS, READ_TIMEOUT).await
}

pub async fn action_bus() -> zbus::Result<Connection> {
    static BUS: OnceLock<Connection> = OnceLock::new();
    open(&BUS, ACTION_TIMEOUT).await
}

/// What the page knows of BlueZ.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Read {
    #[default]
    Loading,
    Ready(Bluetooth),
    /// No BlueZ, or no adapter.
    Unavailable,
}

/// The page's state.
#[derive(Debug, Default)]
pub struct State {
    pub read: Read,
    /// The device a connection, a disconnection or a pairing is under way for.
    pub busy: Option<String>,
    /// The agent's question while a device pairs, and how to answer it.
    pub asked: Option<(Asked, Reply)>,
    /// A paired device whose « Oublier » waits for its confirmation.
    pub forgetting: Option<String>,
    /// A paired device being renamed, and the name typed.
    pub renaming: Option<(String, String)>,
    /// The adapter a scan runs on, while the page is open.
    pub scanning: Option<String>,
    /// The last action refused or failed: said under the switch.
    pub failure: Option<Problem>,
    /// The pairing under way, stopped when the page is left.
    pub pairing: Option<iced::task::Handle>,
}

/// What the page asks, and what comes back.
#[derive(Debug, Clone)]
pub enum Msg {
    Read(Result<Bluetooth, Problem>),
    /// BlueZ announced a change: read again.
    Changed,
    Power(bool),
    /// A device: connected or disconnected when paired, else paired.
    Device(String),
    Pairing(Pairing),
    /// An action ended.
    Done(Result<(), Problem>),
    /// « Confirmer » or « Annuler » under the agent's question.
    Answer(bool),
    Forget(String),
    ConfirmForget(bool),
    Rename(String),
    Name(String),
    /// Enter or « OK » in the name field, or « Annuler » (`false`).
    ConfirmRename(bool),
}

/// Runs `call` on the system bus for actions; its result comes back as `Msg::Done`.
fn act<F>(call: impl FnOnce(Connection) -> F + Send + 'static) -> Task<Msg>
where
    F: Future<Output = system_services::Result<()>> + Send,
{
    Task::perform(
        async move {
            let bus = action_bus().await?;
            call(bus).await
        },
        Msg::Done,
    )
}

/// Reads the adapter and its devices.
pub fn read() -> Task<Msg> {
    Task::perform(
        async move {
            let bus = bus().await?;
            bluetooth::read(&bus).await
        },
        Msg::Read,
    )
}

/// What BlueZ announces, while the page is open; nothing without a bus.
pub fn changes() -> BoxStream<'static, Msg> {
    stream::once(async {
        match bus().await.map_err(Problem::from) {
            Ok(bus) => match bluetooth::changes(&bus).await {
                Ok(changes) => changes.map(|()| Msg::Changed).boxed(),
                Err(problem) => {
                    tracing::warn!(%problem, "the Bluetooth page will not follow BlueZ");
                    stream::empty().boxed()
                }
            },
            Err(problem) => {
                tracing::warn!(%problem, "no system bus for Bluetooth");
                stream::empty().boxed()
            }
        }
    })
    .flatten()
    .boxed()
}

impl State {
    fn ready(&self) -> Option<&Bluetooth> {
        match &self.read {
            Read::Ready(read) => Some(read),
            _ => None,
        }
    }

    pub fn device(&self, path: &str) -> Option<&bluetooth::Device> {
        self.ready()?.devices.iter().find(|d| d.path == path)
    }

    /// The page opens: it is read, and the scan starts once the adapter is on.
    pub fn open(&mut self) -> Task<Msg> {
        self.failure = None;
        read()
    }

    /// The page is left: the scan stops, a pairing is cancelled, what was typed goes.
    pub fn close(&mut self) -> Task<Msg> {
        self.forgetting = None;
        self.renaming = None;
        if let Some(handle) = self.pairing.take() {
            handle.abort();
        }
        if let Some((_, reply)) = self.asked.take() {
            reply.send(false);
        }
        self.busy = None;
        match self.scanning.take() {
            Some(adapter) => {
                act(move |bus| async move { bluetooth::discover(&bus, &adapter, false).await })
            }
            None => Task::none(),
        }
    }

    /// A scan while the page is open and the adapter on.
    fn scan(&mut self) -> Task<Msg> {
        let Some(read) = self.ready() else {
            return Task::none();
        };
        if !read.powered || self.scanning.is_some() {
            return Task::none();
        }
        let adapter = read.adapter.clone();
        self.scanning = Some(adapter.clone());
        act(move |bus| async move { bluetooth::discover(&bus, &adapter, true).await })
    }

    pub fn update(&mut self, message: Msg) -> Task<Msg> {
        match message {
            Msg::Read(read) => {
                self.read = match read {
                    Ok(read) => Read::Ready(read),
                    Err(problem) => {
                        tracing::warn!(%problem, "no Bluetooth");
                        Read::Unavailable
                    }
                };
                if self.ready().is_some_and(|r| !r.powered) {
                    self.scanning = None;
                }
                return self.scan();
            }
            Msg::Changed => return read(),
            Msg::Power(on) => {
                if let Some(read) = self.ready() {
                    let adapter = read.adapter.clone();
                    return act(
                        move |bus| async move { bluetooth::power(&bus, &adapter, on).await },
                    );
                }
            }
            Msg::Device(path) => return self.device_clicked(path),
            Msg::Pairing(event) => return self.pairing(event),
            Msg::Done(done) => {
                self.busy = None;
                match done {
                    Ok(()) => self.failure = None,
                    Err(problem) => {
                        tracing::warn!(%problem, "Bluetooth refused");
                        self.failure = Some(problem);
                    }
                }
                return read();
            }
            Msg::Answer(yes) => {
                if let Some((_, reply)) = self.asked.take() {
                    reply.send(yes);
                }
            }
            Msg::Forget(path) => {
                self.renaming = None;
                self.forgetting = Some(path);
            }
            Msg::ConfirmForget(yes) => {
                let path = self.forgetting.take();
                let adapter = self.ready().map(|r| r.adapter.clone());
                if let (true, Some(path), Some(adapter)) = (yes, path, adapter) {
                    return act(move |bus| async move {
                        bluetooth::forget(&bus, &adapter, &path).await
                    });
                }
            }
            Msg::Rename(path) => {
                self.forgetting = None;
                let name = self
                    .device(&path)
                    .map(|d| d.name.clone())
                    .unwrap_or_default();
                self.renaming = Some((path, name));
            }
            Msg::Name(typed) => {
                if let Some((_, name)) = &mut self.renaming {
                    *name = typed.chars().take(bluetooth::MAX_ALIAS_CHARS).collect();
                }
            }
            Msg::ConfirmRename(yes) => {
                if let (true, Some((path, name))) = (yes, self.renaming.take()) {
                    return act(
                        move |bus| async move { bluetooth::rename(&bus, &path, &name).await },
                    );
                }
                self.renaming = None;
            }
        }
        Task::none()
    }

    /// A paired device connects or disconnects; another pairs, the agent asking the user.
    fn device_clicked(&mut self, path: String) -> Task<Msg> {
        if self.busy.is_some() {
            return Task::none();
        }
        let Some(device) = self.device(&path).cloned() else {
            return Task::none();
        };
        self.busy = Some(path.clone());
        if device.paired {
            return act(move |bus| async move {
                if device.connected {
                    bluetooth::disconnect(&bus, &path).await
                } else {
                    bluetooth::connect(&bus, &path).await
                }
            });
        }
        let pairing = stream::once(action_bus())
            .flat_map(move |bus| match bus {
                Ok(bus) => bluetooth::pair(bus, path.clone()),
                Err(error) => stream::iter([Pairing::Done(Err(Problem::from(error)))]).boxed(),
            })
            .boxed();
        let (task, handle) = Task::run(pairing, Msg::Pairing).abortable();
        self.pairing = Some(handle);
        task
    }

    fn pairing(&mut self, event: Pairing) -> Task<Msg> {
        match event {
            Pairing::Ask(asked, reply) => {
                self.asked = Some((asked, reply));
                Task::none()
            }
            Pairing::Canceled => {
                self.asked = None;
                Task::none()
            }
            Pairing::Done(done) => {
                self.asked = None;
                self.pairing = None;
                self.update(Msg::Done(done))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use system_services::bluetooth::{Device, Kind};

    fn ready() -> State {
        let device = |path: &str, name: &str, paired, connected| Device {
            path: path.to_owned(),
            name: name.to_owned(),
            kind: Kind::Audio,
            paired,
            connected,
            battery: Some(80),
        };
        State {
            read: Read::Ready(Bluetooth {
                adapter: String::from("/org/bluez/hci0"),
                powered: true,
                connected: 1,
                devices: vec![
                    device("/dev/1", "Casque", true, true),
                    device("/dev/2", "Enceinte", false, false),
                ],
            }),
            ..State::default()
        }
    }

    #[test]
    fn one_device_at_a_time_and_a_name_bounded() {
        let mut state = ready();
        let _ = state.update(Msg::Device(String::from("/dev/1")));
        assert_eq!(state.busy.as_deref(), Some("/dev/1"));
        // Another click waits for the first to end.
        let _ = state.update(Msg::Device(String::from("/dev/2")));
        assert_eq!(state.busy.as_deref(), Some("/dev/1"));
        let refused = Err(Problem::Refused(String::from("polkit")));
        let _ = state.update(Msg::Done(refused));
        assert!(state.busy.is_none() && state.failure.is_some());
        let _ = state.update(Msg::Rename(String::from("/dev/1")));
        assert_eq!(
            state.renaming.as_ref().map(|(_, n)| n.as_str()),
            Some("Casque")
        );
        let _ = state.update(Msg::Name("x".repeat(200)));
        let length = state.renaming.as_ref().map(|(_, n)| n.chars().count());
        assert_eq!(length, Some(bluetooth::MAX_ALIAS_CHARS));
        let _ = state.update(Msg::ConfirmRename(false));
        assert!(state.renaming.is_none());
        let _ = state.update(Msg::Forget(String::from("/dev/1")));
        let _ = state.update(Msg::ConfirmForget(false));
        assert!(state.forgetting.is_none());
        // Leaving the page cancels what was under way.
        state.scanning = Some(String::from("/org/bluez/hci0"));
        let _ = state.close();
        assert!(state.scanning.is_none() && state.busy.is_none());
    }
}
