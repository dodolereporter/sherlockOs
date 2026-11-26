//! The `sherlock_shell_v1` source against a fake compositor: the windows already open, a new
//! window, a closed one, and a compositor without the protocol.

use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use dock::shell::Shell;
use dock::windows::{Source, SourceError};
use shell_protocol::server::sherlock_shell_manager_v1::{self as manager, SherlockShellManagerV1};
use shell_protocol::server::sherlock_shell_toplevel_v1::{
    self as toplevel, SherlockShellToplevelV1,
};
use wayland_client::Connection;
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::{Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New};

/// What the test asks the fake compositor to do.
enum Change {
    Open(&'static str),
    Close(&'static str),
}

struct Compositor {
    /// Windows already open when a client binds: `app_id`, active.
    open: Vec<(&'static str, bool)>,
    managers: Vec<(Client, SherlockShellManagerV1)>,
    toplevels: Vec<(&'static str, SherlockShellToplevelV1)>,
}

impl Compositor {
    fn announce(&mut self, display: &DisplayHandle, app_id: &'static str, active: bool) {
        for (client, manager) in &self.managers {
            let Ok(toplevel) =
                client.create_resource::<SherlockShellToplevelV1, (), Self>(display, 1, ())
            else {
                continue;
            };
            manager.toplevel(&toplevel);
            toplevel.id(self.toplevels.len() as u32 + 1);
            toplevel.app_id(String::from(app_id));
            let state = if active {
                (toplevel::State::Activated as u32).to_ne_bytes().to_vec()
            } else {
                Vec::new()
            };
            toplevel.state(state);
            manager.done();
            self.toplevels.push((app_id, toplevel));
        }
    }

    fn apply(&mut self, display: &DisplayHandle, change: Change) {
        match change {
            Change::Open(app_id) => self.announce(display, app_id, false),
            Change::Close(app_id) => {
                for (_, toplevel) in self.toplevels.iter().filter(|(a, _)| *a == app_id) {
                    toplevel.closed();
                }
                for (_, manager) in &self.managers {
                    manager.done();
                }
                self.toplevels.retain(|(a, _)| *a != app_id);
            }
        }
    }
}

impl GlobalDispatch<SherlockShellManagerV1, ()> for Compositor {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        client: &Client,
        resource: New<SherlockShellManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        state.managers.push((client.clone(), manager));
        for (app_id, active) in state.open.clone() {
            state.announce(display, app_id, active);
        }
    }
}

impl Dispatch<SherlockShellManagerV1, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &SherlockShellManagerV1,
        _: manager::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<SherlockShellToplevelV1, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &SherlockShellToplevelV1,
        _: toplevel::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

struct NoData;

impl ClientData for NoData {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

/// The fake compositor on a thread, with one client connection.
struct Fake {
    connection: Option<Connection>,
    changes: Sender<Change>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(
        open: &[(&'static str, bool)],
        with_protocol: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut display = Display::<Compositor>::new()?;
        let mut handle = display.handle();
        if with_protocol {
            handle.create_global::<Compositor, SherlockShellManagerV1, ()>(1, ());
        }
        let (ours, theirs) = UnixStream::pair()?;
        handle.insert_client(theirs, Arc::new(NoData))?;
        let connection = Connection::from_socket(ours)?;
        let (changes, received): (Sender<Change>, Receiver<Change>) = channel();
        let mut state = Compositor {
            open: open.to_vec(),
            managers: Vec::new(),
            toplevels: Vec::new(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let _ = display.dispatch_clients(&mut state);
                while let Ok(change) = received.try_recv() {
                    let handle = display.handle();
                    state.apply(&handle, change);
                }
                let _ = display.flush_clients();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Ok(Self {
            connection: Some(connection),
            changes,
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn the_shell_windows_are_read_as_they_come_and_go() {
    let mut fake = Fake::start(&[("org.example.terminal", true)], true).expect("fake compositor");
    let connection = fake.connection.take().expect("connection");
    let mut shell = Shell::with_connection(connection).expect("manager bound");
    let windows = shell.read().expect("read");
    assert!(!windows.controllable, "read only until D-18c");
    assert_eq!(windows.of(|id| id == "org.example.terminal"), (1, true));

    // An application starts: its window comes.
    fake.changes
        .send(Change::Open("QuickExample"))
        .expect("sent");
    assert!(shell.wait(), "a change came");
    let windows = shell.read().expect("read");
    assert_eq!(windows.list.len(), 2);
    assert_eq!(windows.of(|id| id == "QuickExample"), (1, false));

    fake.changes
        .send(Change::Close("org.example.terminal"))
        .expect("sent");
    assert!(shell.wait());
    let windows = shell.read().expect("read");
    assert_eq!(windows.list.len(), 1);
    assert_eq!(windows.of(|id| id == "org.example.terminal"), (0, false));
}

#[test]
fn a_compositor_without_the_shell_protocol_is_reported() {
    let mut fake = Fake::start(&[], false).expect("fake compositor");
    let connection = fake.connection.take().expect("connection");
    let result = Shell::with_connection(connection);
    assert!(matches!(result, Err(SourceError::NoShellProtocol(_))));
}
