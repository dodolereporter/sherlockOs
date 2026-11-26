//! Workspaces: the model the compositor keeps, and `ext-workspace-v1` for the bar.
//!
//! Workspaces are shared by every screen (ADR 0021): one group holding every screen, and one
//! active workspace for all of them. There are `DEFAULT_COUNT` of them unless the session sets
//! another number, each window belongs to one, and only the windows of the active one are
//! shown (lot D-18w, `State::activate_workspace`).

use smithay::{
    output::Output,
    reexports::{
        wayland_protocols::ext::workspace::v1::server::{
            ext_workspace_group_handle_v1::{self, ExtWorkspaceGroupHandleV1, GroupCapabilities},
            ext_workspace_handle_v1::{self, ExtWorkspaceHandleV1, WorkspaceCapabilities},
            ext_workspace_manager_v1::{self, ExtWorkspaceManagerV1},
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
            protocol::wl_output::WlOutput,
        },
    },
};

use super::toplevels::bytes;
use crate::State;

/// A workspace: its stable id, which the protocols name it by, and the name shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub name: String,
}

/// The workspaces, in their order, and the active one.
#[derive(Debug)]
pub struct Workspaces {
    list: Vec<Workspace>,
    active: usize,
}

/// Number of workspaces of a session that sets none.
pub const DEFAULT_COUNT: usize = 3;
/// At most as many workspaces as there are shortcuts to reach them: Super + 1 to 9.
pub const MAX_COUNT: usize = 9;

impl Default for Workspaces {
    fn default() -> Self {
        Self {
            list: numbered(DEFAULT_COUNT),
            active: 0,
        }
    }
}

/// `count` workspaces, from one to `MAX_COUNT`, each with its number for id and name.
fn numbered(count: usize) -> Vec<Workspace> {
    (1..=count.clamp(1, MAX_COUNT))
        .map(|n| Workspace {
            id: n.to_string(),
            name: n.to_string(),
        })
        .collect()
}

impl Workspaces {
    /// Sets the number of workspaces. The active one stays if it still exists, the first one
    /// becomes active otherwise.
    pub(super) fn set_count(&mut self, count: usize) {
        self.list = numbered(count);
        if self.active >= self.list.len() {
            self.active = 0;
        }
    }

    pub fn list(&self) -> &[Workspace] {
        &self.list
    }

    /// Id of the active workspace.
    pub fn active(&self) -> &str {
        self.list.get(self.active).map_or("", |w| &w.id)
    }

    /// Index of the active workspace, from 0.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Index of the workspace `id`, from 0.
    pub fn index(&self, id: &str) -> Option<usize> {
        self.list.iter().position(|w| w.id == id)
    }

    /// Id of the workspace at `index`, from 0.
    pub fn id_at(&self, index: usize) -> Option<&str> {
        self.list.get(index).map(|w| w.id.as_str())
    }

    pub fn contains(&self, id: &str) -> bool {
        self.index(id).is_some()
    }

    /// Makes the workspace `id` active. `false` for an unknown id.
    pub(super) fn activate(&mut self, id: &str) -> bool {
        match self.index(id) {
            Some(i) => {
                self.active = i;
                true
            }
            None => false,
        }
    }
}

pub(super) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ExtWorkspaceManagerV1, ()>(1, ());
}

/// A bound manager and what its client knows: the shared group, the `wl_output` objects it
/// entered, and each workspace with its index and active state as last sent.
pub(super) struct Manager {
    resource: ExtWorkspaceManagerV1,
    /// Workspace this client activated since its last commit: the commit applies it.
    pending: Option<String>,
    group: Option<ExtWorkspaceGroupHandleV1>,
    entered: Vec<(WlOutput, Output)>,
    workspaces: Vec<(Workspace, ExtWorkspaceHandleV1, usize, bool)>,
}

impl Manager {
    pub(super) fn alive(&self) -> bool {
        self.resource.is_alive()
    }

    /// Sends what changed in the screens and the workspaces, then `done` if anything did.
    pub(super) fn update(&mut self, dh: &DisplayHandle, outputs: &[Output], model: &Workspaces) {
        let Some(client) = self.resource.client() else {
            return;
        };
        let version = self.resource.version();
        let mut changed = false;
        let group = match &self.group {
            Some(group) => group.clone(),
            None => {
                let Ok(group) =
                    client.create_resource::<ExtWorkspaceGroupHandleV1, (), State>(dh, version, ())
                else {
                    return;
                };
                self.resource.workspace_group(&group);
                group.capabilities(GroupCapabilities::empty());
                self.group = Some(group.clone());
                changed = true;
                group
            }
        };
        // Screens: the client's `wl_output` objects enter the group, including those bound
        // later, and leave it with their screen.
        self.entered.retain(|(wl_output, output)| {
            let kept = wl_output.is_alive() && outputs.contains(output);
            if !kept && wl_output.is_alive() {
                group.output_leave(wl_output);
                changed = true;
            }
            kept
        });
        for output in outputs {
            for wl_output in output.client_outputs(&client) {
                if !self.entered.iter().any(|e| e.0 == wl_output) {
                    group.output_enter(&wl_output);
                    self.entered.push((wl_output, output.clone()));
                    changed = true;
                }
            }
        }
        // Workspaces: removed, new, changed.
        self.workspaces.retain(|(workspace, handle, ..)| {
            let kept = model.contains(&workspace.id);
            if !kept {
                group.workspace_leave(handle);
                handle.removed();
                changed = true;
            }
            kept
        });
        for (index, workspace) in model.list().iter().enumerate() {
            let active = workspace.id == model.active();
            if let Some(known) = self.workspaces.iter_mut().find(|w| w.0.id == workspace.id) {
                if known.0.name != workspace.name {
                    known.1.name(workspace.name.clone());
                }
                if known.2 != index {
                    known.1.coordinates(bytes(&[index as u32]));
                }
                if known.3 != active {
                    known.1.state(state(active));
                }
                changed |= (&known.0.name, known.2, known.3) != (&workspace.name, index, active);
                *known = (workspace.clone(), known.1.clone(), index, active);
                continue;
            }
            let data = Handle {
                id: workspace.id.clone(),
                manager: self.resource.clone(),
            };
            let Ok(handle) =
                client.create_resource::<ExtWorkspaceHandleV1, Handle, State>(dh, version, data)
            else {
                return;
            };
            self.resource.workspace(&handle);
            handle.id(workspace.id.clone());
            handle.name(workspace.name.clone());
            handle.coordinates(bytes(&[index as u32]));
            handle.state(state(active));
            handle.capabilities(WorkspaceCapabilities::Activate);
            group.workspace_enter(&handle);
            self.workspaces
                .push((workspace.clone(), handle, index, active));
            changed = true;
        }
        if changed {
            self.resource.done();
        }
    }
}

/// What a workspace handle knows: its workspace, and the manager whose commit applies its
/// requests.
pub(super) struct Handle {
    id: String,
    manager: ExtWorkspaceManagerV1,
}

fn state(active: bool) -> ext_workspace_handle_v1::State {
    if active {
        ext_workspace_handle_v1::State::Active
    } else {
        ext_workspace_handle_v1::State::empty()
    }
}

impl GlobalDispatch<ExtWorkspaceManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<ExtWorkspaceManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let mut manager = Manager {
            resource: data_init.init(resource, ()),
            pending: None,
            group: None,
            entered: Vec::new(),
            workspaces: Vec::new(),
        };
        // Right away, so that a roundtrip after the bind brings the workspaces.
        let outputs: Vec<_> = state.space.outputs().cloned().collect();
        manager.update(dh, &outputs, &state.shell.workspace_model);
        state.shell.workspaces.push(manager);
    }

    fn can_view(client: Client, _: &()) -> bool {
        super::is_shell_client(&client)
    }
}

impl Dispatch<ExtWorkspaceManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &ExtWorkspaceManagerV1,
        request: ext_workspace_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            // The requests of this client since its last commit apply together: the last
            // activation wins.
            ext_workspace_manager_v1::Request::Commit => {
                let manager = state.shell.workspaces.iter_mut();
                let pending = manager
                    .filter(|m| &m.resource == resource)
                    .find_map(|m| m.pending.take());
                if let Some(id) = pending {
                    state.activate_workspace(&id);
                }
            }
            ext_workspace_manager_v1::Request::Stop => {
                state.shell.workspaces.retain(|m| &m.resource != resource);
                resource.finished();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtWorkspaceGroupHandleV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtWorkspaceGroupHandleV1,
        _: ext_workspace_group_handle_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // `create_workspace` is not among the capabilities: ignored, as the protocol allows.
    }
}

impl Dispatch<ExtWorkspaceHandleV1, Handle> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtWorkspaceHandleV1,
        request: ext_workspace_handle_v1::Request,
        handle: &Handle,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // Only `activate` is among the capabilities; it waits for the commit of its manager.
        if let ext_workspace_handle_v1::Request::Activate = request
            && let Some(manager) = state
                .shell
                .workspaces
                .iter_mut()
                .find(|m| m.resource == handle.manager)
        {
            manager.pending = Some(handle.id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_workspaces_by_default_from_one_to_nine() {
        let mut model = Workspaces::default();
        assert_eq!(model.list().len(), DEFAULT_COUNT);
        assert_eq!(model.active(), "1");
        assert!(model.contains("3") && !model.contains("4"));
        assert!(!model.activate("4"), "an unknown workspace was activated");
        assert!(model.activate("3"));
        assert_eq!((model.active(), model.active_index()), ("3", 2));
        assert_eq!((model.index("2"), model.id_at(1)), (Some(1), Some("2")));
        model.set_count(2);
        assert_eq!(model.active(), "1", "the first one becomes active");
        model.set_count(0);
        assert_eq!(model.list().len(), 1, "never without a workspace");
        model.set_count(12);
        assert_eq!(model.list().len(), MAX_COUNT);
    }
}
