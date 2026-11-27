//! The compositor's workspaces and windows, from the shell protocols of ADR 0021: the standard
//! `ext-workspace-v1` for the workspaces, `sherlock_shell_v1` for the windows and the screens a
//! maximized window covers. Our compositor offers both on its shell socket only, next to the
//! session's (`shell_protocol::socket_from_env`): the bar opens a connection of its own there.
//!
//! Each protocol's events are kept pending, then settled at the `done` of that protocol's
//! manager: the bar only ever reads settled batches, never half of one.

use std::collections::BTreeSet;
use std::io::{self, ErrorKind};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use shell_protocol::client::sherlock_shell_manager_v1::{self as shell, SherlockShellManagerV1};
use shell_protocol::client::sherlock_shell_toplevel_v1::{
    self as toplevel, SherlockShellToplevelV1,
};
use wayland_client::globals::{BindError, GlobalError, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{
    ConnectError, Connection, Dispatch, DispatchError, EventQueue, Proxy, QueueHandle,
    event_created_child,
};
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_group_handle_v1::{self as group, ExtWorkspaceGroupHandleV1},
    ext_workspace_handle_v1::{self as workspace, ExtWorkspaceHandleV1},
    ext_workspace_manager_v1::{self as workspaces, ExtWorkspaceManagerV1},
};

use super::workspaces::{AppMenu, Desktop, Group, Placeholder, Workspace};
use super::{Source, SourceError};

/// `wl_output` version that gives the screen's name.
const OUTPUT_VERSION: u32 = 4;
/// `wl_output` version that has `release`.
const OUTPUT_RELEASE: u32 = 3;
/// Delay between two tries of a shell socket that does not exist yet.
pub const RETRY: Duration = Duration::from_secs(3);
/// `sherlock_shell_manager_v1` version with the global menu.
pub const APP_MENU_VERSION: u32 = 3;

#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("no Wayland session in the environment (WAYLAND_DISPLAY, XDG_RUNTIME_DIR)")]
    NoSession,
    #[error("no shell socket at {path}: {source}")]
    Socket { path: PathBuf, source: io::Error },
    #[error("cannot open the connection: {0}")]
    Connect(#[from] ConnectError),
    #[error("cannot list the compositor's globals: {0}")]
    Globals(#[from] GlobalError),
    #[error("the compositor does not offer the shell protocols: {0}")]
    Unsupported(#[from] BindError),
    #[error("lost the compositor: {0}")]
    Dispatch(#[from] DispatchError),
}

impl ShellError {
    /// The shell socket is not there, or nobody listens on it yet: it may come later.
    pub fn may_come(&self) -> bool {
        matches!(self, Self::Socket { source, .. }
            if matches!(source.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused))
    }
}

/// A connection to the shell socket of the session.
pub fn connect() -> Result<Connection, ShellError> {
    let path = shell_protocol::socket_from_env().ok_or(ShellError::NoSession)?;
    let stream =
        UnixStream::connect(&path).map_err(|source| ShellError::Socket { path, source })?;
    Ok(Connection::from_socket(stream)?)
}

/// The workspaces of the last settled batch, by id, for `Switcher`.
type Handles = Arc<Mutex<Vec<(String, ExtWorkspaceHandleV1)>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The workspaces and windows of the compositor, kept up to date.
pub struct Shell {
    queue: EventQueue<Objects>,
    objects: Objects,
    switcher: Switcher,
}

impl Shell {
    /// Binds both protocols on `connection` and reads the current state.
    pub fn with_connection(connection: Connection) -> Result<Self, ShellError> {
        let (globals, mut queue) = registry_queue_init::<Objects>(&connection)?;
        let handle = queue.handle();
        let mut objects = Objects::default();
        // The screens first: the workspace groups name them by these objects.
        for global in globals.contents().clone_list() {
            if global.interface == WlOutput::interface().name {
                objects.bind_output(globals.registry(), global.name, global.version, &handle);
            }
        }
        let manager = globals.bind(&handle, 1..=1, ())?;
        let shell: SherlockShellManagerV1 = globals.bind(&handle, 1..=APP_MENU_VERSION, ())?;
        queue.roundtrip(&mut objects)?;
        // The first batches are read right away: `wait` waits for the next ones.
        objects.changed = false;
        let switcher = Switcher {
            connection,
            manager,
            shell,
            handles: Arc::clone(&objects.handles),
        };
        Ok(Self {
            queue,
            objects,
            switcher,
        })
    }

    pub fn desktop(&self) -> Desktop {
        self.objects.desktop()
    }

    /// Switches workspaces on this connection, from any thread.
    pub fn switcher(&self) -> Switcher {
        self.switcher.clone()
    }

    /// Waits for the next settled batch. `false` once the compositor is gone.
    pub fn wait(&mut self) -> bool {
        loop {
            if let Err(error) = self.queue.blocking_dispatch(&mut self.objects) {
                tracing::warn!(%error, "lost the compositor's shell protocols");
                return false;
            }
            if std::mem::take(&mut self.objects.changed) {
                return true;
            }
        }
    }
}

/// Sends `activate` then `commit` on the watching connection. Requests can go from any thread
/// while the watching one waits for events, and nothing waits for the compositor: a click never
/// blocks, even on a compositor that hangs.
#[derive(Clone)]
pub struct Switcher {
    connection: Connection,
    manager: ExtWorkspaceManagerV1,
    shell: SherlockShellManagerV1,
    handles: Handles,
}

impl Switcher {
    /// Asks for the workspace `id`. `false` for an unknown id or a lost connection.
    pub fn activate(&self, id: &str) -> bool {
        let handles = lock(&self.handles);
        let Some((_, handle)) = handles.iter().find(|(known, _)| known == id) else {
            return false;
        };
        handle.activate();
        self.manager.commit();
        match self.connection.flush() {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(id, %error, "cannot switch workspaces");
                false
            }
        }
    }

    /// The compositor knows the global menu: `register_app_menu` can go.
    pub fn has_app_menus(&self) -> bool {
        self.shell.version() >= APP_MENU_VERSION
    }

    /// Passes a change of the registrar on to the compositor, without waiting.
    pub fn app_menu(&self, change: crate::registrar::Change) {
        if !self.has_app_menus() {
            return;
        }
        use crate::registrar::Change;
        match change {
            Change::Register {
                window,
                pid,
                service,
                path,
            } => self.shell.register_app_menu(window, pid, service, path),
            Change::Unregister { service, path } => self.shell.unregister_app_menu(service, path),
        }
        if let Err(error) = self.connection.flush() {
            tracing::warn!(%error, "cannot pass a window menu on to the compositor");
        }
    }
}

/// The switcher of the source the bar watches, once connected.
static SWITCHER: Mutex<Option<Switcher>> = Mutex::new(None);

/// What to do once connected to a compositor that knows the global menu: `bar::run` starts
/// the registrar there. Tests leave it empty, and never take a name on a bus.
static ON_APP_MENUS: Mutex<Option<Box<dyn FnOnce() + Send>>> = Mutex::new(None);

/// Runs `start` once the bar is connected to a compositor that takes the global menus.
pub fn on_app_menus(start: impl FnOnce() + Send + 'static) {
    *lock(&ON_APP_MENUS) = Some(Box::new(start));
}

/// Passes a change of the registrar on to the compositor, through the bar's connection.
pub fn app_menu(change: crate::registrar::Change) {
    match lock(&SWITCHER).as_ref() {
        Some(switcher) => switcher.app_menu(change),
        None => tracing::warn!("not connected to the compositor: window menu dropped"),
    }
}

/// Switches to the workspace `id` through the bar's connection.
pub fn switch_to(id: &str) {
    match lock(&SWITCHER).as_ref() {
        Some(switcher) if switcher.activate(id) => tracing::debug!(id, "workspace asked"),
        Some(_) => tracing::warn!(id, "no such workspace"),
        None => tracing::warn!(id, "not connected to the compositor's workspaces"),
    }
}

/// How `Auto` reaches the compositor.
type Open = Box<dyn FnMut() -> Result<Shell, ShellError> + Send>;

/// The source of the bar: the compositor's shell protocols, or the placeholder when they are
/// not offered or when its variables are set. It connects on the source's thread, at the first
/// read. When the shell socket is not there yet, it tries again every `retry`, quietly: an
/// unchanged placeholder sends nothing, and so asks for no frame.
pub struct Auto {
    open: Open,
    retry: Duration,
    source: Option<Result<Shell, Placeholder>>,
    /// The shell socket may still come.
    retrying: bool,
}

impl Auto {
    pub fn new(
        open: impl FnMut() -> Result<Shell, ShellError> + Send + 'static,
        retry: Duration,
    ) -> Self {
        Self {
            open: Box::new(open),
            retry,
            source: None,
            retrying: false,
        }
    }

    fn connected(&mut self, shell: Shell) {
        let switcher = shell.switcher();
        let app_menus = switcher.has_app_menus();
        *lock(&SWITCHER) = Some(switcher);
        self.source = Some(Ok(shell));
        if app_menus && let Some(start) = lock(&ON_APP_MENUS).take() {
            start();
        }
    }
}

impl Default for Auto {
    fn default() -> Self {
        Self::new(|| Shell::with_connection(connect()?), RETRY)
    }
}

impl Source for Auto {
    type State = Desktop;

    fn read(&mut self) -> Result<Desktop, SourceError> {
        if self.source.is_none() {
            if Placeholder::asked() {
                tracing::info!("workspaces from the placeholder variables");
                self.source = Some(Err(Placeholder));
            } else {
                match (self.open)() {
                    Ok(shell) => self.connected(shell),
                    Err(error) => {
                        self.retrying = error.may_come();
                        let retrying = self.retrying;
                        tracing::info!(%error, retrying, "placeholder workspaces, no active application");
                        self.source = Some(Err(Placeholder));
                    }
                }
            }
        }
        match &mut self.source {
            Some(Ok(shell)) => Ok(shell.desktop()),
            _ => Placeholder.read(),
        }
    }

    fn wait(&mut self) -> bool {
        match &mut self.source {
            Some(Ok(shell)) => shell.wait(),
            Some(Err(_)) if self.retrying => {
                std::thread::sleep(self.retry);
                match (self.open)() {
                    Ok(shell) => {
                        tracing::info!("workspaces and windows from the compositor");
                        self.connected(shell);
                    }
                    Err(error) if error.may_come() => {}
                    Err(error) => {
                        tracing::info!(%error, "placeholder workspaces for good");
                        self.retrying = false;
                    }
                }
                true
            }
            _ => false,
        }
    }
}

/// The protocol objects of the connection: what they said since the last `done` of their
/// protocol (pending), and the batches settled at those `done` events.
#[derive(Default)]
struct Objects {
    /// Each screen: its registry name, object and name.
    outputs: Vec<(u32, WlOutput, String)>,
    groups: Vec<GroupObject>,
    workspaces: Vec<WorkspaceObject>,
    windows: Vec<Window>,
    covered: BTreeSet<String>,
    /// Settled by `ext-workspace-v1`: the groups, window counts left out.
    settled_groups: Vec<Group>,
    /// Settled by `ext-workspace-v1`, for `Switcher`.
    handles: Handles,
    /// Settled by `sherlock_shell_v1`.
    settled_windows: Vec<Settled>,
    settled_covered: BTreeSet<String>,
    /// A batch was settled since the last `wait`.
    changed: bool,
}

struct GroupObject {
    handle: ExtWorkspaceGroupHandleV1,
    outputs: Vec<WlOutput>,
    workspaces: Vec<ExtWorkspaceHandleV1>,
}

struct WorkspaceObject {
    handle: ExtWorkspaceHandleV1,
    id: String,
    name: String,
    coordinates: Vec<u32>,
    active: bool,
}

struct Window {
    handle: SherlockShellToplevelV1,
    title: String,
    app_id: String,
    workspace: String,
    activated: bool,
    menu: Option<AppMenu>,
}

/// A window of a settled batch.
struct Settled {
    workspace: String,
    /// Its title, or its `app_id` without one.
    name: String,
    activated: bool,
    menu: Option<AppMenu>,
}

impl Objects {
    fn bind_output(
        &mut self,
        registry: &WlRegistry,
        name: u32,
        version: u32,
        qh: &QueueHandle<Self>,
    ) {
        let output = registry.bind(name, version.min(OUTPUT_VERSION), qh, ());
        self.outputs.push((name, output, String::new()));
    }

    /// The `done` of `ext-workspace-v1`.
    fn settle_workspaces(&mut self) {
        let output_name =
            |o: &WlOutput| self.outputs.iter().find(|e| &e.1 == o).map(|e| e.2.clone());
        let mut handles = Vec::new();
        self.settled_groups = self
            .groups
            .iter()
            .map(|g| {
                let mut list: Vec<_> = self
                    .workspaces
                    .iter()
                    .filter(|w| g.workspaces.contains(&w.handle))
                    .collect();
                list.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));
                handles.extend(list.iter().map(|w| (w.id.clone(), w.handle.clone())));
                let workspaces = list.iter().enumerate().map(|(i, w)| Workspace {
                    id: w.id.clone(),
                    name: Some(w.name.clone())
                        .filter(|n| !n.is_empty() && *n != (i + 1).to_string()),
                    active: w.active,
                    windows: None,
                });
                Group {
                    outputs: g.outputs.iter().filter_map(output_name).collect(),
                    workspaces: workspaces.collect(),
                }
            })
            .collect();
        *lock(&self.handles) = handles;
        self.changed = true;
    }

    /// The `done` of `sherlock_shell_v1`.
    fn settle_windows(&mut self) {
        self.settled_windows = self
            .windows
            .iter()
            .map(|w| {
                let name = if w.title.trim().is_empty() {
                    &w.app_id
                } else {
                    &w.title
                };
                Settled {
                    workspace: w.workspace.clone(),
                    name: name.trim().to_owned(),
                    activated: w.activated,
                    menu: w.menu.clone(),
                }
            })
            .collect();
        self.settled_covered = self.covered.clone();
        self.changed = true;
    }

    fn desktop(&self) -> Desktop {
        let mut groups = self.settled_groups.clone();
        let mut active = BTreeSet::new();
        for workspace in groups.iter_mut().flat_map(|g| &mut g.workspaces) {
            let on = self
                .settled_windows
                .iter()
                .filter(|w| w.workspace == workspace.id);
            workspace.windows = Some(on.count());
            if workspace.active {
                active.insert(workspace.id.as_str());
            }
        }
        // A window keeps its states on a workspace that is not shown: only the active
        // workspaces' active window names the application.
        let window = self
            .settled_windows
            .iter()
            .find(|w| w.activated && active.contains(w.workspace.as_str()));
        let app = window.map(|w| w.name.clone()).filter(|n| !n.is_empty());
        Desktop {
            groups,
            menu: window.and_then(|w| w.menu.clone()),
            app,
            covered: self.settled_covered.clone(),
            live: true,
        }
    }
}

/// Whether an array of 32-bit values, in the machine's byte order, holds `value`.
fn holds(array: &[u8], value: u32) -> bool {
    array
        .as_chunks::<4>()
        .0
        .iter()
        .any(|c| u32::from_ne_bytes(*c) == value)
}

impl Dispatch<WlRegistry, GlobalListContents> for Objects {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == WlOutput::interface().name => {
                state.bind_output(registry, name, version, qh);
            }
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(index) = state.outputs.iter().position(|o| o.0 == name) {
                    let (_, output, _) = state.outputs.remove(index);
                    if output.version() >= OUTPUT_RELEASE {
                        output.release();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for Objects {
    fn event(
        state: &mut Self,
        proxy: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(output) = state.outputs.iter_mut().find(|o| &o.1 == proxy)
        {
            output.2 = name;
        }
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for Objects {
    fn event(
        state: &mut Self,
        _: &ExtWorkspaceManagerV1,
        event: workspaces::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            workspaces::Event::WorkspaceGroup { workspace_group } => {
                state.groups.push(GroupObject {
                    handle: workspace_group,
                    outputs: Vec::new(),
                    workspaces: Vec::new(),
                })
            }
            workspaces::Event::Workspace { workspace } => state.workspaces.push(WorkspaceObject {
                handle: workspace,
                id: String::new(),
                name: String::new(),
                coordinates: Vec::new(),
                active: false,
            }),
            workspaces::Event::Done => state.settle_workspaces(),
            _ => {}
        }
    }

    event_created_child!(Objects, ExtWorkspaceManagerV1, [
        workspaces::EVT_WORKSPACE_GROUP_OPCODE => (ExtWorkspaceGroupHandleV1, ()),
        workspaces::EVT_WORKSPACE_OPCODE => (ExtWorkspaceHandleV1, ()),
    ]);
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for Objects {
    fn event(
        state: &mut Self,
        proxy: &ExtWorkspaceGroupHandleV1,
        event: group::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.groups.iter().position(|g| &g.handle == proxy) else {
            return;
        };
        let group = &mut state.groups[index];
        match event {
            group::Event::OutputEnter { output } => group.outputs.push(output),
            group::Event::OutputLeave { output } => group.outputs.retain(|o| o != &output),
            group::Event::WorkspaceEnter { workspace } => group.workspaces.push(workspace),
            group::Event::WorkspaceLeave { workspace } => {
                group.workspaces.retain(|w| w != &workspace)
            }
            group::Event::Removed => state.groups.remove(index).handle.destroy(),
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceHandleV1, ()> for Objects {
    fn event(
        state: &mut Self,
        proxy: &ExtWorkspaceHandleV1,
        event: workspace::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.workspaces.iter().position(|w| &w.handle == proxy) else {
            return;
        };
        let workspace = &mut state.workspaces[index];
        match event {
            workspace::Event::Id { id } => workspace.id = id,
            workspace::Event::Name { name } => workspace.name = name,
            workspace::Event::Coordinates { coordinates } => {
                let (values, _) = coordinates.as_chunks::<4>();
                workspace.coordinates = values.iter().map(|c| u32::from_ne_bytes(*c)).collect();
            }
            workspace::Event::State { state: flags } => {
                let active = workspace::State::Active;
                workspace.active = flags.into_result().is_ok_and(|f| f.contains(active));
            }
            workspace::Event::Removed => state.workspaces.remove(index).handle.destroy(),
            _ => {}
        }
    }
}

impl Dispatch<SherlockShellManagerV1, ()> for Objects {
    fn event(
        state: &mut Self,
        _: &SherlockShellManagerV1,
        event: shell::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            shell::Event::Toplevel { toplevel } => state.windows.push(Window {
                handle: toplevel,
                title: String::new(),
                app_id: String::new(),
                workspace: String::new(),
                activated: false,
                menu: None,
            }),
            shell::Event::Output { name, state: flags } => {
                let maximized = holds(&flags, shell::OutputState::Maximized as u32);
                if maximized || holds(&flags, shell::OutputState::Fullscreen as u32) {
                    state.covered.insert(name);
                } else {
                    state.covered.remove(&name);
                }
            }
            shell::Event::OutputRemoved { name } => {
                state.covered.remove(&name);
            }
            shell::Event::Done => state.settle_windows(),
            _ => {}
        }
    }

    event_created_child!(Objects, SherlockShellManagerV1, [
        shell::EVT_TOPLEVEL_OPCODE => (SherlockShellToplevelV1, ()),
    ]);
}

impl Dispatch<SherlockShellToplevelV1, ()> for Objects {
    fn event(
        state: &mut Self,
        proxy: &SherlockShellToplevelV1,
        event: toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(index) = state.windows.iter().position(|w| &w.handle == proxy) else {
            return;
        };
        let window = &mut state.windows[index];
        match event {
            toplevel::Event::Title { title } => window.title = title,
            toplevel::Event::AppId { app_id } => window.app_id = app_id,
            toplevel::Event::Workspace { workspace } => window.workspace = workspace,
            toplevel::Event::AppMenu { service, path } => {
                let some = !service.is_empty() && !path.is_empty();
                window.menu = some.then_some(AppMenu { service, path });
            }
            toplevel::Event::State { state: flags } => {
                window.activated = holds(&flags, toplevel::State::Activated as u32);
            }
            toplevel::Event::Closed => state.windows.remove(index).handle.destroy(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_array_is_read_in_the_machine_order() {
        let bytes: Vec<u8> = [1u32, 5].iter().flat_map(|v| v.to_ne_bytes()).collect();
        assert!(holds(&bytes, 5) && holds(&bytes, 1));
        assert!(!holds(&bytes, 2) && !holds(&[], 1));
    }
}
