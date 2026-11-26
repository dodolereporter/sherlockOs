//! The shell protocols client against a fake compositor with three workspaces and two screens:
//! a small `wayland-server` plays the compositor, which still has a single workspace (D-18w).

use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bar::app::{Bar, Message};
use bar::source::Source;
use bar::source::shell::{Auto, Shell, ShellError};
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
    managers: Vec<(ExtWorkspaceManagerV1, Vec<ExtWorkspaceHandleV1>)>,
    shells: Vec<SherlockShellManagerV1>,
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
        state.managers.push((manager, handles));
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
        for (manager, handles) in &state.managers {
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

/// Runs the fake compositor on a thread; returns a connection for each of `clients`.
struct Fake {
    connections: Vec<Connection>,
    requests: Arc<Mutex<Vec<String>>>,
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
            pending: None,
            requests: Arc::clone(&requests),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::SeqCst) {
                let _ = display.dispatch_clients(&mut state);
                let _ = display.flush_clients();
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        Ok(Self {
            connections,
            requests,
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

/// A bar on the screen `output`, showing `desktop`.
fn bar_on(output: &str, desktop: Desktop) -> Bar {
    let mut bar = Bar::default();
    let surface = Id::unique();
    let _ = bar.update(Message::Surface(surface));
    let _ = bar.update(Message::Output(surface, Some(output.to_owned())));
    let _ = bar.update(Message::Desktop(desktop));
    bar
}

#[test]
fn the_bar_follows_the_compositor_and_switches_workspaces() {
    let mut fake = Fake::start(true, 2).expect("fake compositor");
    let mut watcher = Shell::with_connection(fake.connections.remove(0)).expect("protocols bound");
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

    // What the click does, on a connection of its own: activate, then commit.
    let mut switcher = Shell::with_connection(fake.connections.remove(0)).expect("bound");
    assert!(switcher.activate("3").expect("asked"));
    assert!(!switcher.activate("9").expect("asked"), "unknown workspace");
    let requests = fake.requests.lock().map(|r| r.clone()).unwrap_or_default();
    assert_eq!(requests, ["activate 3", "commit"]);

    // The watcher sees workspace 3 active and the screen free again.
    for _ in 0..4 {
        let desktop = watcher.desktop();
        let switched = desktop.group(None).is_some_and(|g| g.workspaces[2].active);
        if switched && desktop.covered.is_empty() {
            break;
        }
        assert!(watcher.wait(), "a change came");
    }
    let desktop = watcher.desktop();
    let active: Vec<_> = desktop
        .group(None)
        .expect("group")
        .workspaces
        .iter()
        .map(|w| w.active)
        .collect();
    assert_eq!(active, [false, false, true]);
    let mut bar = bar_on("eDP-1", watcher.desktop());
    assert!(!bar.shape.attached(), "the bar floats again");
    let _ = bar.update(Message::Output(Id::unique(), Some(String::from("DP-1"))));
    assert!(!bar.shape.attached());
}

#[test]
fn without_the_shell_protocols_the_bar_keeps_its_placeholder() {
    let mut fake = Fake::start(false, 1).expect("fake compositor");
    let result = Shell::with_connection(fake.connections.remove(0));
    assert!(matches!(result, Err(ShellError::Unsupported(_))));

    let mut source = Auto::new(|| Err(ShellError::NoSession));
    let desktop = source.read().expect("placeholder");
    assert!(!desktop.live);
    assert_eq!(desktop, Desktop::placeholder(None, None));
    assert!(!source.wait(), "the placeholder never changes");
}
