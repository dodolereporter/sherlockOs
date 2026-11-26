//! The shell protocols client against a fake compositor with three workspaces and two screens:
//! a small `wayland-server` plays the compositor, which still has a single workspace (D-18w).
//! The test drives it with commands, run on its thread between two dispatches.

use std::io::{self, ErrorKind};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use bar::app::{Bar, Message};
use bar::source::Source;
use bar::source::shell::{Auto, RETRY, Shell, ShellError};
use bar::source::workspaces::Desktop;
use iced::window::Id;
use shell_protocol::server::sherlock_shell_manager_v1::{self as shell, SherlockShellManagerV1};
use shell_protocol::server::sherlock_shell_toplevel_v1::{
    self as toplevel, SherlockShellToplevelV1,
};
use wayland_client::Connection;
use wayland_protocols::ext::workspace::v1::server::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1, GroupCapabilities},
    ext_workspace_handle_v1::{self as workspace, ExtWorkspaceHandleV1, WorkspaceCapabilities},
    ext_workspace_manager_v1::{self as workspaces, ExtWorkspaceManagerV1},
};
use wayland_server::backend::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::wl_output::{self, WlOutput};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
};

/// A window: title, `app_id`, workspace, active.
type Window = (&'static str, &'static str, &'static str, bool);

/// The fake compositor. A maximized window sits on workspace `1`, on `eDP-1`.
struct Compositor {
    workspaces: Vec<(&'static str, &'static str)>,
    active: &'static str,
    windows: Vec<Window>,
    /// The `wl_output` objects of every client.
    outputs: Vec<WlOutput>,
    /// Each client's manager, group and workspaces.
    managers: Vec<(
        ExtWorkspaceManagerV1,
        ExtWorkspaceGroupHandleV1,
        Vec<ExtWorkspaceHandleV1>,
    )>,
    shells: Vec<SherlockShellManagerV1>,
    /// Each client's windows, with their title.
    toplevels: Vec<(SherlockShellToplevelV1, &'static str)>,
    pending: Option<String>,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Compositor {
    fn covered(&self) -> Vec<u8> {
        let maximized = shell::OutputState::Maximized as u32;
        if self.active == "1" {
            maximized.to_ne_bytes().to_vec()
        } else {
            Vec::new()
        }
    }

    fn record(&self, request: String) {
        if let Ok(mut requests) = self.requests.lock() {
            requests.push(request);
        }
    }
}

fn active(yes: bool) -> workspace::State {
    if yes {
        workspace::State::Active
    } else {
        workspace::State::empty()
    }
}

impl GlobalDispatch<WlOutput, &'static str> for Compositor {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<WlOutput>,
        name: &&'static str,
        init: &mut DataInit<'_, Self>,
    ) {
        let output = init.init(resource, *name);
        output.name(name.to_string());
        output.done();
        state.outputs.push(output);
    }
}

impl Dispatch<WlOutput, &'static str> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &WlOutput,
        _: wl_output::Request,
        _: &&'static str,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl GlobalDispatch<ExtWorkspaceManagerV1, ()> for Compositor {
    fn bind(
        state: &mut Self,
        display: &DisplayHandle,
        client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let manager = init.init(resource, ());
        let Ok(group) =
            client.create_resource::<ExtWorkspaceGroupHandleV1, (), Self>(display, 1, ())
        else {
            return;
        };
        manager.workspace_group(&group);
        group.capabilities(GroupCapabilities::empty());
        // Workspaces shared by every screen (ADR 0021): one group holding both.
        let mine = |o: &&WlOutput| o.client().is_some_and(|c| c.id() == client.id());
        for output in state.outputs.iter().filter(mine) {
            group.output_enter(output);
        }
        let mut handles = Vec::new();
        for (index, (id, name)) in state.workspaces.iter().enumerate() {
            let Ok(handle) =
                client.create_resource::<ExtWorkspaceHandleV1, &'static str, Self>(display, 1, *id)
            else {
                return;
            };
            manager.workspace(&handle);
            handle.id(id.to_string());
            handle.name(name.to_string());
            handle.coordinates((index as u32).to_ne_bytes().to_vec());
            handle.state(active(*id == state.active));
            handle.capabilities(WorkspaceCapabilities::Activate);
            group.workspace_enter(&handle);
            handles.push(handle);
        }
        manager.done();
        state.managers.push((manager, group, handles));
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtWorkspaceManagerV1,
        request: workspaces::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if !matches!(request, workspaces::Request::Commit) {
            return;
        }
        state.record(String::from("commit"));
        let Some(id) = state.pending.take() else {
            return;
        };
        let Some(&(found, _)) = state.workspaces.iter().find(|w| w.0 == id) else {
            return;
        };
        state.active = found;
        for (manager, _, handles) in &state.managers {
            for handle in handles {
                handle.state(active(handle.data::<&'static str>() == Some(&found)));
            }
            manager.done();
        }
        // The maximized window stays behind on workspace 1: the screen is free.
        let covered = state.covered();
        for shell in &state.shells {
            shell.output(String::from("eDP-1"), covered.clone());
            shell.done();
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtWorkspaceGroupHandleV1,
        _: group::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ExtWorkspaceHandleV1, &'static str> for Compositor {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtWorkspaceHandleV1,
        request: workspace::Request,
        id: &&'static str,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let workspace::Request::Activate = request {
            state.record(format!("activate {id}"));
            state.pending = Some(id.to_string());
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
        for (i, (title, app_id, workspace, activated)) in state.windows.iter().enumerate() {
            let Ok(window) =
                client.create_resource::<SherlockShellToplevelV1, (), Self>(display, 1, ())
            else {
                return;
            };
            manager.toplevel(&window);
            window.id(i as u32 + 1);
            window.title(title.to_string());
            window.app_id(app_id.to_string());
            let mut states = (toplevel::State::Maximized as u32).to_ne_bytes().to_vec();
            if *activated {
                states.extend((toplevel::State::Activated as u32).to_ne_bytes());
            }
            window.state(states);
            window.output(String::from("eDP-1"));
            window.workspace(workspace.to_string());
            window.geometry(0, 36, 1440, 864);
            state.toplevels.push((window, title));
        }
        manager.output(String::from("eDP-1"), state.covered());
        manager.output(String::from("DP-1"), Vec::new());
        manager.done();
        state.shells.push(manager);
    }
}

impl Dispatch<SherlockShellManagerV1, ()> for Compositor {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &SherlockShellManagerV1,
        _: shell::Request,
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

/// Something the test makes the compositor do.
type Command = Box<dyn FnOnce(&mut Compositor, &DisplayHandle) + Send>;

/// Runs the fake compositor on a thread; returns a connection for each of `clients`.
struct Fake {
    connections: Vec<Connection>,
    requests: Arc<Mutex<Vec<String>>>,
    commands: Arc<Mutex<Vec<Command>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Fake {
    fn start(shell_globals: bool, clients: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let mut display = Display::<Compositor>::new()?;
        let mut handle = display.handle();
        handle.create_global::<Compositor, WlOutput, &'static str>(4, "eDP-1");
        handle.create_global::<Compositor, WlOutput, &'static str>(4, "DP-1");
        if shell_globals {
            handle.create_global::<Compositor, ExtWorkspaceManagerV1, ()>(1, ());
            handle.create_global::<Compositor, SherlockShellManagerV1, ()>(1, ());
        }
        let mut connections = Vec::new();
        for _ in 0..clients {
            let (ours, theirs) = UnixStream::pair()?;
            handle.insert_client(theirs, Arc::new(NoData))?;
            connections.push(Connection::from_socket(ours)?);
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut state = Compositor {
            workspaces: vec![("1", "1"), ("2", "2"), ("3", "Jeu")],
            active: "1",
            windows: vec![
                ("Notes — Éditeur", "org.example.editor", "1", true),
                ("", "org.example.browser", "3", false),
            ],
            outputs: Vec::new(),
            managers: Vec::new(),
            shells: Vec::new(),
            toplevels: Vec::new(),
            pending: None,
            requests: Arc::clone(&requests),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let commands: Arc<Mutex<Vec<Command>>> = Arc::default();
        let queued = Arc::clone(&commands);
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let now: Vec<_> = queued
                    .lock()
                    .map(|mut c| c.drain(..).collect())
                    .unwrap_or_default();
                for command in now {
                    command(&mut state, &display.handle());
                }
                let _ = display.dispatch_clients(&mut state);
                let _ = display.flush_clients();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Ok(Self {
            connections,
            requests,
            commands,
            stop,
            thread: Some(thread),
        })
    }
}

impl Fake {
    fn run(&self, command: impl FnOnce(&mut Compositor, &DisplayHandle) + Send + 'static) {
        if let Ok(mut commands) = self.commands.lock() {
            commands.push(Box::new(command));
        }
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

/// A bar on the screen `output`, showing `desktop`.
fn bar_on(output: &str, desktop: Desktop) -> Bar {
    let mut bar = Bar::default();
    let surface = Id::unique();
    let _ = bar.update(Message::Surface(surface));
    let _ = bar.update(Message::Output(surface, Some(output.to_owned()), None));
    let _ = bar.update(Message::Desktop(desktop));
    bar
}

/// `Shell::wait`, failing the test instead of hanging when nothing comes within 5 s.
fn wait(mut shell: Shell) -> (Shell, bool) {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let changed = shell.wait();
        let _ = sender.send((shell, changed));
    });
    match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(answer) => answer,
        Err(error) => panic!("the shell source did not answer in time: {error}"),
    }
}

/// Waits until `settled` holds of the desktop, for a few batches at most.
fn wait_until(mut shell: Shell, settled: impl Fn(&Desktop) -> bool) -> Shell {
    for _ in 0..4 {
        if settled(&shell.desktop()) {
            return shell;
        }
        let changed;
        (shell, changed) = wait(shell);
        assert!(changed, "the compositor is still there");
    }
    assert!(settled(&shell.desktop()), "{:?}", shell.desktop());
    shell
}

/// Ids and window counts of the workspaces shown.
fn counts(desktop: &Desktop) -> Vec<(String, Option<usize>)> {
    let group = desktop.group(None).map_or(&[][..], |g| &g.workspaces);
    group.iter().map(|w| (w.id.clone(), w.windows)).collect()
}

#[test]
fn the_bar_follows_the_compositor_and_switches_workspaces() {
    let mut fake = Fake::start(true, 1).expect("fake compositor");
    let watcher = Shell::with_connection(fake.connections.remove(0)).expect("protocols bound");
    let desktop = watcher.desktop();
    assert!(desktop.live);
    assert_eq!(desktop.app.as_deref(), Some("Notes — Éditeur"));
    let group = desktop.group(Some("DP-1")).expect("a group");
    assert_eq!(group.outputs, ["eDP-1", "DP-1"]);
    let seen: Vec<_> = group
        .workspaces
        .iter()
        .map(|w| (w.id.as_str(), w.name.as_deref(), w.active, w.windows))
        .collect();
    assert_eq!(
        seen,
        [
            ("1", None, true, Some(1)),
            ("2", None, false, Some(0)),
            ("3", Some("Jeu"), false, Some(1)),
        ]
    );

    // Each screen on its own: the maximized window is on eDP-1 only.
    assert!(bar_on("eDP-1", desktop.clone()).shape.attached());
    let bar = bar_on("DP-1", desktop.clone());
    assert!(!bar.shape.attached());
    // A click on the chip of an inactive workspace names it; on the active one, nothing.
    assert_eq!(bar.to_activate(2), Some("3"));
    assert_eq!(bar.to_activate(0), None);
    assert_eq!(bar.to_activate(7), None);

    // What the click does, on the watching connection: activate, then commit.
    let switcher = watcher.switcher();
    assert!(switcher.activate("3"));
    assert!(!switcher.activate("9"), "unknown workspace");

    // The watcher sees workspace 3 active and the screen free again.
    let switched = |d: &Desktop| d.group(None).is_some_and(|g| g.workspaces[2].active);
    let watcher = wait_until(watcher, |d| switched(d) && d.covered.is_empty());
    let requests = fake.requests.lock().map(|r| r.clone()).unwrap_or_default();
    assert_eq!(requests, ["activate 3", "commit"]);
    let desktop = watcher.desktop();
    // The editor keeps its active state on workspace 1, which is not shown: no name.
    assert_eq!(desktop.app, None);
    let mut bar = bar_on("eDP-1", desktop);
    assert!(!bar.shape.attached(), "the bar floats again");
    let _ = bar.update(Message::Output(
        Id::unique(),
        Some(String::from("DP-1")),
        None,
    ));
    assert!(!bar.shape.attached());
}

#[test]
fn only_finished_batches_are_read() {
    let mut fake = Fake::start(true, 1).expect("fake compositor");
    let watcher = Shell::with_connection(fake.connections.remove(0)).expect("protocols bound");

    // A fourth workspace arrives without its `done`, while sherlock_shell_v1 ends a batch.
    fake.run(|state, display| {
        for (manager, group, handles) in &mut state.managers {
            let Some(client) = manager.client() else {
                continue;
            };
            let Ok(handle) = client
                .create_resource::<ExtWorkspaceHandleV1, &'static str, Compositor>(display, 1, "4")
            else {
                continue;
            };
            manager.workspace(&handle);
            handle.id(String::from("4"));
            handle.name(String::from("Musique"));
            handle.coordinates(3u32.to_ne_bytes().to_vec());
            handle.state(active(false));
            group.workspace_enter(&handle);
            handles.push(handle);
        }
        for shell in &state.shells {
            shell.done();
        }
    });
    let (watcher, changed) = wait(watcher);
    assert!(changed);
    assert_eq!(
        counts(&watcher.desktop()).len(),
        3,
        "half a batch is not read"
    );
    fake.run(|state, _| state.managers.iter().for_each(|m| m.0.done()));
    let watcher = wait_until(watcher, |d| counts(d).len() == 4);
    let names = watcher
        .desktop()
        .group(None)
        .map(|g| g.workspaces[3].name.clone());
    assert_eq!(names, Some(Some(String::from("Musique"))));

    // The other way round: a terminal opens and takes the focus, without its `done`, while
    // ext-workspace-v1 ends a batch.
    fake.run(|state, display| {
        let maximized = (toplevel::State::Maximized as u32).to_ne_bytes().to_vec();
        let activated = (toplevel::State::Activated as u32).to_ne_bytes().to_vec();
        for (editor, _) in &state.toplevels {
            editor.state(maximized.clone());
        }
        for shell in &state.shells {
            let Some(client) = shell.client() else {
                continue;
            };
            let Ok(window) =
                client.create_resource::<SherlockShellToplevelV1, (), Compositor>(display, 1, ())
            else {
                continue;
            };
            shell.toplevel(&window);
            window.id(3);
            window.title(String::from("Terminal"));
            window.workspace(String::from("1"));
            window.state(activated.clone());
            state.toplevels.push((window, "Terminal"));
        }
        state.managers.iter().for_each(|m| m.0.done());
    });
    let (watcher, changed) = wait(watcher);
    assert!(changed);
    let desktop = watcher.desktop();
    assert_eq!(
        desktop.app.as_deref(),
        Some("Notes — Éditeur"),
        "half a batch is not read"
    );
    assert_eq!(counts(&desktop)[0].1, Some(1));
    fake.run(|state, _| state.shells.iter().for_each(|s| s.done()));
    let watcher = wait_until(watcher, |d| d.app.as_deref() == Some("Terminal"));
    assert_eq!(counts(&watcher.desktop())[0].1, Some(2));

    // The terminal closes: one window less, and no active window left.
    fake.run(|state, _| {
        for (window, title) in &state.toplevels {
            if *title == "Terminal" {
                window.closed();
            }
        }
        state.shells.iter().for_each(|s| s.done());
    });
    let watcher = wait_until(watcher, |d| counts(d)[0].1 == Some(1));
    assert_eq!(watcher.desktop().app, None);

    // The fourth workspace goes.
    fake.run(|state, _| {
        for (manager, group, handles) in &state.managers {
            for handle in handles
                .iter()
                .filter(|h| h.data::<&'static str>() == Some(&"4"))
            {
                group.workspace_leave(handle);
                handle.removed();
            }
            manager.done();
        }
    });
    let watcher = wait_until(watcher, |d| counts(d).len() == 3);

    // The compositor goes: the source says it will not change any more.
    drop(fake);
    let (_, changed) = wait(watcher);
    assert!(!changed);
}

#[test]
fn without_the_shell_protocols_the_bar_keeps_its_placeholder() {
    let mut fake = Fake::start(false, 1).expect("fake compositor");
    let result = Shell::with_connection(fake.connections.remove(0));
    assert!(matches!(result, Err(ShellError::Unsupported(_))));

    let mut source = Auto::new(|| Err(ShellError::NoSession), RETRY);
    let desktop = source.read().expect("placeholder");
    assert!(!desktop.live);
    assert_eq!(desktop, Desktop::placeholder(None, None));
    assert!(!source.wait(), "the placeholder never changes");
}

#[test]
fn a_shell_socket_that_comes_later_is_picked_up() {
    let mut fake = Fake::start(true, 1).expect("fake compositor");
    let mut connection = Some(fake.connections.remove(0));
    let mut tries = 0;
    // The socket is missing at the first try, there at the second.
    let open = move || {
        tries += 1;
        match connection.take() {
            Some(later) if tries == 1 => {
                connection = Some(later);
                Err(missing())
            }
            Some(connection) => Shell::with_connection(connection),
            None => Err(missing()),
        }
    };
    let mut source = Auto::new(open, Duration::from_millis(10));
    assert!(!source.read().expect("placeholder").live, "no socket yet");
    assert!(source.wait(), "it tries again");
    assert!(
        source.read().expect("desktop").live,
        "the compositor's workspaces"
    );
}

fn missing() -> ShellError {
    ShellError::Socket {
        path: PathBuf::from("/run/user/1000/wayland-1-shell"),
        source: io::Error::from(ErrorKind::NotFound),
    }
}
