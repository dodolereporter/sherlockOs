//! The `sherlock_shell_v1` source against a fake compositor: the windows already open, a new
//! window, a closed one, a covered screen, the requests and rectangles it sends, and a
//! compositor without the protocol.

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use dock::shell::Shell;
use dock::windows::{Control, Place, Request, Source, SourceError};
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
    /// `closed`, then `done`.
    Close(&'static str),
    /// `closed` alone: the batch is not over.
    Closing(&'static str),
    Done,
    /// A screen, and whether a maximized window covers it, then `done`.
    Cover(&'static str, bool),
}

/// What the client asked: the `app_id` of the window, and the request.
type Asked = Arc<Mutex<Vec<(&'static str, String)>>>;

struct Compositor {
    /// Windows already open when a client binds: `app_id`, active.
    open: Vec<(&'static str, bool)>,
    managers: Vec<(Client, SherlockShellManagerV1)>,
    toplevels: Vec<(&'static str, SherlockShellToplevelV1)>,
    asked: Asked,
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
                self.apply(display, Change::Closing(app_id));
                self.apply(display, Change::Done);
            }
            Change::Closing(app_id) => {
                for (_, toplevel) in self.toplevels.iter().filter(|(a, _)| *a == app_id) {
                    toplevel.closed();
                }
                self.toplevels.retain(|(a, _)| *a != app_id);
            }
            Change::Done => {
                for (_, manager) in &self.managers {
                    manager.done();
                }
            }
            Change::Cover(name, covered) => {
                let state = if covered {
                    (manager::OutputState::Maximized as u32)
                        .to_ne_bytes()
                        .to_vec()
                } else {
                    Vec::new()
                };
                for (_, manager) in &self.managers {
                    manager.output(String::from(name), state.clone());
                }
                self.apply(display, Change::Done);
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
        state: &mut Self,
        _: &Client,
        resource: &SherlockShellToplevelV1,
        request: toplevel::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let Some((app_id, _)) = state.toplevels.iter().find(|(_, t)| t == resource) else {
            return;
        };
        let asked = match request {
            toplevel::Request::Activate => String::from("activate"),
            toplevel::Request::Minimize => String::from("minimize"),
            toplevel::Request::Close => String::from("close"),
            toplevel::Request::SetRectangle {
                output,
                x,
                y,
                width,
                height,
            } => format!("rectangle {output} {x} {y} {width} {height}"),
            _ => return,
        };
        state
            .asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((app_id, asked));
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
    asked: Asked,
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
        let asked = Asked::default();
        let mut state = Compositor {
            open: open.to_vec(),
            managers: Vec::new(),
            toplevels: Vec::new(),
            asked: Arc::clone(&asked),
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
            asked,
            stop,
            thread: Some(thread),
        })
    }
}

impl Fake {
    /// What the client asked, once `count` requests came (5 s at most).
    fn asked(&self, count: usize) -> Vec<(&'static str, String)> {
        for _ in 0..500 {
            let asked = self
                .asked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if asked.len() >= count {
                return asked;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    assert!(windows.controllable);
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

#[test]
fn a_closed_window_is_published_with_its_batch_only() {
    let mut fake = Fake::start(
        &[("org.example.terminal", false), ("QuickExample", false)],
        true,
    )
    .expect("fake compositor");
    let connection = fake.connection.take().expect("connection");
    let mut shell = Shell::with_connection(connection).expect("manager bound");
    assert_eq!(shell.read().expect("read").list.len(), 2);

    // A window closes, and the batch goes on: nothing is published yet. Read halfway, the
    // dock would count one window less and take a window that only stayed for a new one.
    fake.changes
        .send(Change::Closing("org.example.terminal"))
        .expect("sent");
    let (published, waited) = channel();
    let waiting = std::thread::spawn(move || {
        let changed = shell.wait();
        let _ = published.send(changed);
        shell
    });
    assert!(
        waited.recv_timeout(Duration::from_millis(300)).is_err(),
        "published before done"
    );
    // The batch ends: now it is.
    fake.changes.send(Change::Done).expect("sent");
    assert_eq!(waited.recv_timeout(Duration::from_secs(5)), Ok(true));
    let mut shell = waiting.join().expect("waiting thread");
    let windows = shell.read().expect("read");
    assert_eq!(windows.list.len(), 1);
    assert_eq!(windows.of(|id| id == "org.example.terminal"), (0, false));
}

fn place(app_id: &'static str, rectangle: (i32, i32, i32, i32)) -> Place {
    Place {
        owns: Box::new(move |a| a == app_id),
        output: String::from("eDP-1"),
        rectangle,
    }
}

#[test]
fn the_windows_are_acted_on_and_placed_on_the_watching_connection() {
    let mut fake = Fake::start(
        &[("org.example.terminal", true), ("QuickExample", false)],
        true,
    )
    .expect("fake compositor");
    let connection = fake.connection.take().expect("connection");
    let mut shell = Shell::with_connection(connection).expect("manager bound");
    let control = shell.control();

    // The terminal is in front: a click minimizes it. « Quitter » on the other one closes it.
    control.send(Request::Minimize, Box::new(|a| a == "org.example.terminal"));
    control.send(Request::Close, Box::new(|a| a == "QuickExample"));
    control.send(Request::Activate, Box::new(|a| a == "org.example.terminal"));
    // Its icon: sent once, not again while it stays.
    control.place(vec![place("org.example.terminal", (10, 820, 52, 52))]);
    control.place(vec![place("org.example.terminal", (10, 820, 52, 52))]);
    let asked = fake.asked(4);
    assert_eq!(
        asked,
        [
            ("org.example.terminal", String::from("minimize")),
            ("QuickExample", String::from("close")),
            ("org.example.terminal", String::from("activate")),
            (
                "org.example.terminal",
                String::from("rectangle eDP-1 10 820 52 52")
            ),
        ]
    );

    // A second window of the terminal gets the rectangle of its icon as it comes.
    fake.changes
        .send(Change::Open("org.example.terminal"))
        .expect("sent");
    assert!(shell.wait());
    let asked = fake.asked(5);
    assert_eq!(
        asked.get(4),
        Some(&(
            "org.example.terminal",
            String::from("rectangle eDP-1 10 820 52 52")
        ))
    );

    // The icon moves: both windows follow. It goes: their rectangles are removed.
    control.place(vec![place("org.example.terminal", (70, 820, 52, 52))]);
    control.place(Vec::new());
    let asked = fake.asked(9);
    let rest: Vec<&str> = asked[5..].iter().map(|(_, r)| r.as_str()).collect();
    assert_eq!(
        rest,
        [
            "rectangle eDP-1 70 820 52 52",
            "rectangle eDP-1 70 820 52 52",
            "rectangle eDP-1 0 0 0 0",
            "rectangle eDP-1 0 0 0 0",
        ]
    );
}

#[test]
fn the_covered_screens_are_read() {
    let mut fake = Fake::start(&[("org.example.terminal", true)], true).expect("fake compositor");
    let connection = fake.connection.take().expect("connection");
    let mut shell = Shell::with_connection(connection).expect("manager bound");
    assert!(shell.read().expect("read").covered.is_empty());
    fake.changes
        .send(Change::Cover("eDP-1", true))
        .expect("sent");
    assert!(shell.wait());
    let covered = shell.read().expect("read").covered;
    assert!(covered.contains("eDP-1") && covered.len() == 1);
    fake.changes
        .send(Change::Cover("eDP-1", false))
        .expect("sent");
    assert!(shell.wait());
    assert!(shell.read().expect("read").covered.is_empty());
}
