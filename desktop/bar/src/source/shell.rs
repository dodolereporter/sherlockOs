//! The compositor's workspaces and windows, from the shell protocols of ADR 0021: the standard
//! `ext-workspace-v1` for the workspaces, `sherlock_shell_v1` for the windows and the screens a
//! maximized window covers. Our compositor offers both on its shell socket only, next to the
//! session's (`shell_protocol::socket_from_env`): the bar opens a connection of its own there.
//!
//! Events are applied as they come; a `done` of either manager ends a batch, after which the
//! state is read.

use std::collections::BTreeSet;
use std::io;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

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

use super::workspaces::{Desktop, Group, Placeholder, Workspace};
use super::{Source, SourceError};

/// `wl_output` version that gives the screen's name.
const OUTPUT_VERSION: u32 = 4;

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

/// A connection to the shell socket of the session.
pub fn connect() -> Result<Connection, ShellError> {
    let path = shell_protocol::socket_from_env().ok_or(ShellError::NoSession)?;
    let stream =
        UnixStream::connect(&path).map_err(|source| ShellError::Socket { path, source })?;
    Ok(Connection::from_socket(stream)?)
}

/// The workspaces and windows of the compositor, kept up to date.
pub struct Shell {
    queue: EventQueue<Objects>,
    objects: Objects,
    manager: ExtWorkspaceManagerV1,
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
        let _: SherlockShellManagerV1 = globals.bind(&handle, 1..=1, ())?;
        queue.roundtrip(&mut objects)?;
        Ok(Self {
            queue,
            objects,
            manager,
        })
    }

    pub fn desktop(&self) -> Desktop {
        self.objects.desktop()
    }

    /// Waits for the next batch of changes. `false` once the compositor is gone.
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

    /// Makes the workspace `id` active: `activate`, then `commit`. `false` for an unknown id.
    pub fn activate(&mut self, id: &str) -> Result<bool, ShellError> {
        let Some(found) = self.objects.workspaces.iter().find(|w| w.id == id) else {
            return Ok(false);
        };
        found.handle.activate();
        self.manager.commit();
        // The compositor has the requests before the connection goes.
        self.queue.roundtrip(&mut self.objects)?;
        Ok(true)
    }
}

/// Switches to the workspace `id`, on a thread and a connection of its own: the watching
/// connection stays on its thread, blocked waiting for events.
pub fn switch_to(id: String) {
    let spawned = std::thread::Builder::new()
        .name(String::from("workspace-switch"))
        .spawn(move || {
            match connect()
                .and_then(Shell::with_connection)
                .and_then(|mut s| s.activate(&id))
            {
                Ok(true) => tracing::debug!(id, "workspace asked"),
                Ok(false) => tracing::warn!(id, "no such workspace"),
                Err(error) => tracing::warn!(id, %error, "cannot switch workspaces"),
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start the workspace switch");
    }
}

/// The source of the bar: the compositor's shell protocols, or the placeholder when they are
/// not offered or when its variables are set. It connects on the source's thread, at the first
/// read, with `open`.
pub struct Auto {
    open: fn() -> Result<Shell, ShellError>,
    source: Option<Result<Shell, Placeholder>>,
}

impl Auto {
    pub fn new(open: fn() -> Result<Shell, ShellError>) -> Self {
        Self { open, source: None }
    }
}

impl Default for Auto {
    fn default() -> Self {
        Self::new(|| Shell::with_connection(connect()?))
    }
}

impl Source for Auto {
    type State = Desktop;

    fn read(&mut self) -> Result<Desktop, SourceError> {
        let open = self.open;
        let source = self.source.get_or_insert_with(|| {
            if Placeholder::asked() {
                tracing::info!("workspaces from the placeholder variables");
                return Err(Placeholder);
            }
            open()
                .inspect_err(|error| {
                    tracing::info!(%error, "placeholder workspaces, no active application");
                })
                .map_err(|_| Placeholder)
        });
        match source {
            Ok(shell) => Ok(shell.desktop()),
            Err(placeholder) => placeholder.read(),
        }
    }

    fn wait(&mut self) -> bool {
        match &mut self.source {
            Some(Ok(shell)) => shell.wait(),
            _ => false,
        }
    }
}

/// The protocol objects of the connection and what they said.
#[derive(Default)]
struct Objects {
    /// Each screen: its registry name, object and name.
    outputs: Vec<(u32, WlOutput, String)>,
    groups: Vec<GroupObject>,
    workspaces: Vec<WorkspaceObject>,
    windows: Vec<Window>,
    covered: BTreeSet<String>,
    /// A batch ended since the last `wait`.
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

    fn desktop(&self) -> Desktop {
        let output_name =
            |o: &WlOutput| self.outputs.iter().find(|e| &e.1 == o).map(|e| e.2.clone());
        let groups = self.groups.iter().map(|g| {
            let mut list: Vec<_> = self
                .workspaces
                .iter()
                .filter(|w| g.workspaces.contains(&w.handle))
                .collect();
            list.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));
            let workspaces = list.iter().enumerate().map(|(i, w)| Workspace {
                id: w.id.clone(),
                name: Some(w.name.clone()).filter(|n| !n.is_empty() && *n != (i + 1).to_string()),
                active: w.active,
                windows: Some(self.windows.iter().filter(|t| t.workspace == w.id).count()),
            });
            Group {
                outputs: g.outputs.iter().filter_map(output_name).collect(),
                workspaces: workspaces.collect(),
            }
        });
        let active = self.windows.iter().find(|w| w.activated);
        let app = active.map(|w| {
            if w.title.trim().is_empty() {
                &w.app_id
            } else {
                &w.title
            }
        });
        Desktop {
            groups: groups.collect(),
            app: app.map(|a| a.trim().to_owned()).filter(|a| !a.is_empty()),
            covered: self.covered.clone(),
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
            wl_registry::Event::GlobalRemove { name } => state.outputs.retain(|o| o.0 != name),
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
            workspaces::Event::Done => state.changed = true,
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
            shell::Event::Done => state.changed = true,
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
