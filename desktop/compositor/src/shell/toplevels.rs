//! `sherlock_shell_v1`: the windows and the state of the screens, for the shell.

use shell_protocol::server::{
    sherlock_shell_manager_v1::{self, SherlockShellManagerV1},
    sherlock_shell_toplevel_v1::{self, SherlockShellToplevelV1},
};
use smithay::{
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
    utils::{Logical, Rectangle},
};

pub(super) use sherlock_shell_manager_v1::OutputState;
pub(super) use sherlock_shell_toplevel_v1::State as ToplevelState;

use crate::State;

/// Version of the global. Version 2 adds the `snapped` state of a screen.
const VERSION: u32 = 2;

pub(super) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, SherlockShellManagerV1, ()>(VERSION, ());
}

/// What a client knows of a window.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Snapshot {
    pub title: String,
    pub app_id: String,
    /// Values of `ToplevelState`.
    pub states: Vec<u32>,
    pub output: String,
    /// Id of its workspace.
    pub workspace: String,
    /// Relative to the screen `output`.
    pub geometry: Rectangle<i32, Logical>,
}

/// Every window, by id.
pub(super) type Windows = Vec<(u32, Snapshot)>;
/// Every screen by name, with the values of `OutputState`.
pub(super) type Outputs = Vec<(String, Vec<u32>)>;

/// A bound manager and what its client was told.
pub(super) struct Manager {
    resource: SherlockShellManagerV1,
    toplevels: Vec<Toplevel>,
    outputs: Outputs,
    /// The first `done` was sent.
    told: bool,
}

struct Toplevel {
    id: u32,
    resource: SherlockShellToplevelV1,
    sent: Snapshot,
}

impl Manager {
    pub(super) fn alive(&self) -> bool {
        self.resource.is_alive()
    }

    /// Sends what changed between what the client knows and `windows` and `outputs`, then
    /// `done` if anything did.
    pub(super) fn update(
        &mut self,
        dh: &DisplayHandle,
        windows: &[(u32, Snapshot)],
        outputs: &[(String, Vec<u32>)],
    ) {
        // A manager bound at version 1 does not know the `snapped` state of a screen.
        let outputs: Outputs = match self.resource.version() {
            1 => outputs
                .iter()
                .map(|(name, state)| {
                    let state = state.iter().copied();
                    let known = state.filter(|s| *s != OutputState::Snapped as u32);
                    (name.clone(), known.collect())
                })
                .collect(),
            _ => outputs.to_vec(),
        };
        let mut changed = false;
        self.toplevels.retain(|t| {
            let open = windows.iter().any(|w| w.0 == t.id);
            if !open {
                t.resource.closed();
                changed = true;
            }
            open
        });
        for (id, snapshot) in windows {
            if let Some(known) = self.toplevels.iter_mut().find(|t| t.id == *id) {
                if known.sent != *snapshot {
                    send(&known.resource, Some(&known.sent), snapshot);
                    known.sent = snapshot.clone();
                    changed = true;
                }
                continue;
            }
            let Some(client) = self.resource.client() else {
                return;
            };
            let Ok(resource) = client.create_resource::<SherlockShellToplevelV1, u32, State>(
                dh,
                self.resource.version(),
                *id,
            ) else {
                return;
            };
            self.resource.toplevel(&resource);
            resource.id(*id);
            send(&resource, None, snapshot);
            self.toplevels.push(Toplevel {
                id: *id,
                resource,
                sent: snapshot.clone(),
            });
            changed = true;
        }
        for (name, _) in &self.outputs {
            if !outputs.iter().any(|o| &o.0 == name) {
                self.resource.output_removed(name.clone());
                changed = true;
            }
        }
        for output in &outputs {
            if !self.outputs.contains(output) {
                self.resource.output(output.0.clone(), bytes(&output.1));
                changed = true;
            }
        }
        self.outputs = outputs;
        // Always a first `done`, even with no window and no screen.
        if changed || !self.told {
            self.resource.done();
            self.told = true;
        }
    }
}

/// Sends the properties of `new` that differ from `old`, or all of them.
fn send(resource: &SherlockShellToplevelV1, old: Option<&Snapshot>, new: &Snapshot) {
    if old.is_none_or(|o| o.title != new.title) {
        resource.title(new.title.clone());
    }
    if old.is_none_or(|o| o.app_id != new.app_id) {
        resource.app_id(new.app_id.clone());
    }
    if old.is_none_or(|o| o.states != new.states) {
        resource.state(bytes(&new.states));
    }
    if old.is_none_or(|o| o.output != new.output) {
        resource.output(new.output.clone());
    }
    if old.is_none_or(|o| o.workspace != new.workspace) {
        resource.workspace(new.workspace.clone());
    }
    if old.is_none_or(|o| o.geometry != new.geometry) {
        let g = new.geometry;
        resource.geometry(g.loc.x, g.loc.y, g.size.w, g.size.h);
    }
}

/// An array of 32-bit values, in the machine's byte order.
pub(super) fn bytes(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

impl GlobalDispatch<SherlockShellManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        _client: &Client,
        resource: New<SherlockShellManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let mut manager = Manager {
            resource: data_init.init(resource, ()),
            toplevels: Vec::new(),
            outputs: Vec::new(),
            told: false,
        };
        // Right away, so that a roundtrip after the bind brings the whole state.
        let (windows, outputs) = state.snapshot();
        manager.update(dh, &windows, &outputs);
        state.shell.managers.push(manager);
    }

    fn can_view(client: Client, _: &()) -> bool {
        super::is_shell_client(&client)
    }
}

impl Dispatch<SherlockShellManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &SherlockShellManagerV1,
        request: sherlock_shell_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let sherlock_shell_manager_v1::Request::Destroy = request {
            state.shell.managers.retain(|m| &m.resource != resource);
        }
    }
}

impl Dispatch<SherlockShellToplevelV1, u32> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &SherlockShellToplevelV1,
        request: sherlock_shell_toplevel_v1::Request,
        id: &u32,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use sherlock_shell_toplevel_v1::{Error, Request};
        if let Request::SetRectangle {
            output,
            x,
            y,
            width,
            height,
        } = request
        {
            if width < 0
                || height < 0
                || x.checked_add(width).is_none()
                || y.checked_add(height).is_none()
            {
                resource.post_error(
                    Error::InvalidRectangle,
                    "negative width or height, or a rectangle past the 32-bit range",
                );
                return;
            }
            state.shell.icons.retain(|i| i.0 != *id);
            if width > 0 && height > 0 && state.window_by_id(*id).is_some() {
                let rect = Rectangle::new((x, y).into(), (width, height).into());
                state.shell.icons.push((*id, output, rect));
            }
            return;
        }
        let Some(window) = state.window_by_id(*id) else {
            return;
        };
        match request {
            Request::Activate => state.activate(&window),
            Request::Minimize => state.minimize(&window),
            Request::Close => State::close(&window),
            Request::MoveToWorkspace { workspace } => state.move_to_workspace(&window, &workspace),
            _ => {}
        }
        tracing::debug!(id, "window request from the shell");
    }
}
