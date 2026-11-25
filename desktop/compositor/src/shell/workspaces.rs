//! Workspaces through `ext-workspace-v1`, for the bar.
//!
//! The compositor has one workspace per screen for now: one group per screen, holding one
//! active workspace named after its number. Switching between several workspaces is a lot of
//! its own; the protocol already carries it, and `activate` on the active workspace changes
//! nothing.

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

pub(super) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ExtWorkspaceManagerV1, ()>(1, ());
}

/// A bound manager and the groups its client knows.
pub(super) struct Manager {
    resource: ExtWorkspaceManagerV1,
    groups: Vec<Group>,
}

/// The group of a screen and its only workspace.
struct Group {
    output: Output,
    group: ExtWorkspaceGroupHandleV1,
    workspace: ExtWorkspaceHandleV1,
    /// The client's `wl_output` objects of the screen already announced.
    entered: Vec<WlOutput>,
}

impl Manager {
    pub(super) fn alive(&self) -> bool {
        self.resource.is_alive()
    }

    /// Announces the groups of new screens, removes those of screens gone, and announces the
    /// `wl_output` objects the client bound since. Then `done` if anything changed.
    pub(super) fn update(&mut self, dh: &DisplayHandle, outputs: &[Output]) {
        let Some(client) = self.resource.client() else {
            return;
        };
        let mut changed = false;
        self.groups.retain(|g| {
            let kept = outputs.contains(&g.output);
            if !kept {
                g.group.workspace_leave(&g.workspace);
                g.workspace.removed();
                g.group.removed();
                changed = true;
            }
            kept
        });
        for output in outputs {
            if self.groups.iter().any(|g| &g.output == output) {
                continue;
            }
            let Some(group) = self.announce(dh, &client, output) else {
                return;
            };
            self.groups.push(group);
            changed = true;
        }
        for group in &mut self.groups {
            group.entered.retain(Resource::is_alive);
            for wl_output in group.output.client_outputs(&client) {
                if !group.entered.contains(&wl_output) {
                    group.group.output_enter(&wl_output);
                    group.entered.push(wl_output);
                    changed = true;
                }
            }
        }
        if changed {
            self.resource.done();
        }
    }

    /// Creates the group of `output` and its workspace on the client's side.
    fn announce(&self, dh: &DisplayHandle, client: &Client, output: &Output) -> Option<Group> {
        let version = self.resource.version();
        let group = client
            .create_resource::<ExtWorkspaceGroupHandleV1, (), State>(dh, version, ())
            .ok()?;
        self.resource.workspace_group(&group);
        group.capabilities(GroupCapabilities::empty());
        let workspace = client
            .create_resource::<ExtWorkspaceHandleV1, (), State>(dh, version, ())
            .ok()?;
        self.resource.workspace(&workspace);
        // The first workspace of the screen: its number, at the first place of the row.
        workspace.name(String::from("1"));
        workspace.coordinates(bytes(&[0]));
        workspace.state(ext_workspace_handle_v1::State::Active);
        workspace.capabilities(WorkspaceCapabilities::Activate);
        group.workspace_enter(&workspace);
        Some(Group {
            output: output.clone(),
            group,
            workspace,
            entered: Vec::new(),
        })
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
            groups: Vec::new(),
        };
        // Right away, so that a roundtrip after the bind brings the workspaces.
        let outputs: Vec<_> = state.space.outputs().cloned().collect();
        manager.update(dh, &outputs);
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
        // `commit` applies nothing: the only workspace of each screen is always active.
        if let ext_workspace_manager_v1::Request::Stop = request {
            state.shell.workspaces.retain(|m| &m.resource != resource);
            resource.finished();
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

impl Dispatch<ExtWorkspaceHandleV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtWorkspaceHandleV1,
        _: ext_workspace_handle_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // `activate` targets the active workspace; the other requests are not among the
        // capabilities.
    }
}
