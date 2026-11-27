//! `wlr-output-management-unstable-v1`, server side (ADR 0025, lot D-26b): every screen as a
//! head with its modes, current mode, place, rotation, scale and names, sent again when they
//! change; a client sends a whole configuration and tests or applies it
//! (`displays.rs`). Control Room's « Écrans » page, `wlr-randr` and `kanshi` speak it.
//!
//! Every request is checked: a mode of another head, a property set twice, a head configured
//! twice or left out are protocol errors, as the protocol says; a configuration made before
//! the last change is cancelled.

use std::sync::{Arc, Mutex, MutexGuard};

use smithay::{
    output::{Mode, Output},
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
    },
    utils::{Point, Transform},
};
use wayland_protocols_wlr::output_management::v1::server::{
    zwlr_output_configuration_head_v1::{self as config_head, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self as configuration, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self as head, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self as manager, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self as mode, ZwlrOutputModeV1},
};

use crate::{
    State,
    displays::{Lit, Setting, Wanted},
};

/// Version 3: make, model and serial (2), `release` of heads and modes (3). Adaptive sync (4)
/// is not offered.
const VERSION: u32 = 3;

/// The managers bound, and the serial of the last configuration they were told about.
#[derive(Default)]
pub struct OutputManagement {
    serial: u32,
    clients: Vec<Bound>,
}

/// A manager bound by a client and what it was told.
struct Bound {
    manager: ZwlrOutputManagerV1,
    heads: Vec<SentHead>,
}

/// A head as it was last sent.
struct SentHead {
    output: Output,
    head: ZwlrOutputHeadV1,
    modes: Vec<(Mode, ZwlrOutputModeV1)>,
    lit: Option<Lit>,
    /// Nothing of its state was sent yet.
    fresh: bool,
}

/// What a configuration collects before `test` or `apply`.
#[derive(Default)]
pub struct Pending {
    serial: u32,
    used: bool,
    heads: Vec<(Output, Option<Arc<Mutex<HeadSetting>>>)>,
}

/// What a configuration head collects, and what was set (each once only).
pub struct HeadSetting {
    output: Output,
    setting: Setting,
    mode_set: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, ZwlrOutputManagerV1, ()>(VERSION, ());
}

impl State {
    /// Tells every manager what changed among the heads since its last `done`: new heads,
    /// changed ones, those gone; then a new serial. Called after every loop iteration.
    pub(crate) fn refresh_output_management(&mut self) {
        let heads: Vec<(Output, Vec<Mode>, Option<Lit>, crate::output::Details)> = self
            .heads()
            .into_iter()
            .map(|(s, lit)| {
                let mut modes = s.details.modes.clone();
                if modes.is_empty() {
                    modes.extend(s.output.current_mode());
                }
                (s.output.clone(), modes, lit, s.details.clone())
            })
            .collect();
        let dh = self.display_handle.clone();
        let management = &mut self.output_management;
        management.clients.retain(|b| b.manager.is_alive());
        let mut changed = false;
        for bound in &mut management.clients {
            let gone = |s: &SentHead| !heads.iter().any(|(o, ..)| *o == s.output);
            for sent in bound.heads.iter().filter(|s| gone(s)) {
                for (_, mode) in &sent.modes {
                    mode.finished();
                }
                sent.head.finished();
                changed = true;
            }
            bound.heads.retain(|s| !gone(s));
            for (output, modes, lit, details) in &heads {
                let index = bound.heads.iter().position(|s| s.output == *output);
                let sent = match index {
                    Some(i) => &mut bound.heads[i],
                    None => {
                        let Some(sent) = introduce(&dh, &bound.manager, output, details) else {
                            continue;
                        };
                        bound.heads.push(sent);
                        changed = true;
                        let Some(last) = bound.heads.last_mut() else {
                            continue;
                        };
                        last
                    }
                };
                changed |= update(&dh, sent, modes, *lit);
            }
        }
        if changed {
            management.serial = management.serial.wrapping_add(1);
            for bound in &management.clients {
                bound.manager.done(management.serial);
            }
        }
    }

    /// The configuration of `pending`, or `None` if a head is left out (a protocol error).
    fn wanted(&self, pending: &Pending) -> Option<Vec<Wanted>> {
        let heads = self.heads();
        let all = heads
            .iter()
            .all(|(s, _)| pending.heads.iter().any(|(o, _)| *o == s.output));
        all.then(|| {
            pending
                .heads
                .iter()
                .map(|(output, setting)| Wanted {
                    output: output.clone(),
                    on: setting.as_ref().map(|s| lock(s).setting.clone()),
                })
                .collect()
        })
    }
}

/// Sends a new head and its names; its state follows in `update`.
fn introduce(
    dh: &DisplayHandle,
    manager: &ZwlrOutputManagerV1,
    output: &Output,
    details: &crate::output::Details,
) -> Option<SentHead> {
    let client = manager.client()?;
    let head = client
        .create_resource::<ZwlrOutputHeadV1, _, State>(dh, manager.version(), output.clone())
        .ok()?;
    manager.head(&head);
    head.name(output.name());
    let identity = &details.identity;
    head.description(format!(
        "{} {} ({})",
        identity.make,
        identity.model,
        output.name()
    ));
    let size = output.physical_properties().size;
    if size.w > 0 && size.h > 0 {
        head.physical_size(size.w, size.h);
    }
    if head.version() >= 2 {
        head.make(identity.make.clone());
        head.model(identity.model.clone());
        head.serial_number(identity.serial.clone());
    }
    Some(SentHead {
        output: output.clone(),
        head,
        modes: Vec::new(),
        lit: None,
        fresh: true,
    })
}

/// Sends what changed of a head: its modes, then whether it is on, and its current state.
fn update(dh: &DisplayHandle, sent: &mut SentHead, modes: &[Mode], lit: Option<Lit>) -> bool {
    let mut changed = false;
    if sent.modes.iter().map(|(m, _)| *m).ne(modes.iter().copied()) {
        for (_, mode) in sent.modes.drain(..) {
            mode.finished();
        }
        let Some(client) = sent.head.client() else {
            return false;
        };
        for (i, m) in modes.iter().enumerate() {
            let data = (sent.output.clone(), *m);
            let version = sent.head.version();
            let Ok(mode) = client.create_resource::<ZwlrOutputModeV1, _, State>(dh, version, data)
            else {
                continue;
            };
            sent.head.mode(&mode);
            mode.size(m.size.w, m.size.h);
            if m.refresh > 0 {
                mode.refresh(m.refresh);
            }
            if i == 0 {
                mode.preferred();
            }
            sent.modes.push((*m, mode));
        }
        // The current mode is sent again with the new objects.
        sent.lit = None;
        changed = true;
    }
    let fresh = std::mem::take(&mut sent.fresh);
    if lit == sent.lit && !fresh {
        return changed;
    }
    if lit.is_some() != sent.lit.is_some() || fresh {
        sent.head.enabled(i32::from(lit.is_some()));
    }
    if let Some(lit) = lit {
        let old = sent.lit;
        if old.map(|o| o.mode) != Some(lit.mode)
            && let Some((_, mode)) = sent.modes.iter().find(|(m, _)| *m == lit.mode)
        {
            sent.head.current_mode(mode);
        }
        if old.map(|o| o.position) != Some(lit.position) {
            sent.head.position(lit.position.x, lit.position.y);
        }
        if old.map(|o| o.transform) != Some(lit.transform) {
            sent.head.transform(lit.transform.into());
        }
        if old.map(|o| o.scale) != Some(lit.scale) {
            sent.head.scale(lit.scale);
        }
    }
    sent.lit = lit;
    true
}

impl GlobalDispatch<ZwlrOutputManagerV1, ()> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrOutputManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        let manager = data_init.init(resource, ());
        state.output_management.clients.push(Bound {
            manager: manager.clone(),
            heads: Vec::new(),
        });
        // Everything is new to it: its heads, then `done` with a new serial for all.
        state.refresh_output_management();
        if state
            .output_management
            .clients
            .iter()
            .all(|b| b.manager != manager || b.heads.is_empty())
        {
            manager.done(state.output_management.serial);
        }
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &ZwlrOutputManagerV1,
        request: manager::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            manager::Request::CreateConfiguration { id, serial } => {
                let pending = Pending {
                    serial,
                    ..Pending::default()
                };
                data_init.init(id, Mutex::new(pending));
            }
            manager::Request::Stop => {
                let clients = &mut state.output_management.clients;
                if let Some(i) = clients.iter().position(|b| b.manager == *resource) {
                    let bound = clients.remove(i);
                    for sent in bound.heads {
                        sent.head.finished();
                    }
                }
                resource.finished();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputHeadV1, Output> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrOutputHeadV1,
        _: head::Request,
        _: &Output,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        // `release` only: the object goes.
    }
}

impl Dispatch<ZwlrOutputModeV1, (Output, Mode)> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ZwlrOutputModeV1,
        _: mode::Request,
        _: &(Output, Mode),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, Mutex<Pending>> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &ZwlrOutputConfigurationV1,
        request: configuration::Request,
        data: &Mutex<Pending>,
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        let mut pending = lock(data);
        let used = || {
            resource.post_error(
                configuration::Error::AlreadyUsed,
                "configuration already used",
            );
        };
        let twice = || {
            let code = configuration::Error::AlreadyConfiguredHead;
            resource.post_error(code, "head configured twice");
        };
        match request {
            configuration::Request::EnableHead { id, head } => {
                let Some(output) = head.data::<Output>().cloned() else {
                    return;
                };
                if pending.used {
                    return used();
                }
                if pending.heads.iter().any(|(o, _)| *o == output) {
                    return twice();
                }
                let setting = Arc::new(Mutex::new(HeadSetting {
                    output: output.clone(),
                    setting: Setting::default(),
                    mode_set: false,
                }));
                data_init.init(id, setting.clone());
                pending.heads.push((output, Some(setting)));
            }
            configuration::Request::DisableHead { head } => {
                let Some(output) = head.data::<Output>().cloned() else {
                    return;
                };
                if pending.used {
                    return used();
                }
                if pending.heads.iter().any(|(o, _)| *o == output) {
                    return twice();
                }
                pending.heads.push((output, None));
            }
            configuration::Request::Apply | configuration::Request::Test => {
                if pending.used {
                    return used();
                }
                pending.used = true;
                if pending.serial != state.output_management.serial {
                    resource.cancelled();
                    return;
                }
                let Some(wanted) = state.wanted(&pending) else {
                    let code = configuration::Error::UnconfiguredHead;
                    return resource.post_error(code, "a head is left out");
                };
                let apply = matches!(request, configuration::Request::Apply);
                let result = match apply {
                    true => state.apply_screens(&wanted),
                    false => state.check_screens(&wanted).map(|_| ()),
                };
                match result {
                    Ok(()) => resource.succeeded(),
                    Err(err) => {
                        tracing::info!(apply, "screen configuration refused: {err}");
                        resource.failed();
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, Arc<Mutex<HeadSetting>>> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        resource: &ZwlrOutputConfigurationHeadV1,
        request: config_head::Request,
        data: &Arc<Mutex<HeadSetting>>,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let mut head = lock(data);
        let set_twice = || {
            resource.post_error(config_head::Error::AlreadySet, "property already set");
        };
        match request {
            config_head::Request::SetMode { mode } => {
                let Some((output, m)) = mode.data::<(Output, Mode)>().cloned() else {
                    return;
                };
                if output != head.output {
                    let code = config_head::Error::InvalidMode;
                    return resource.post_error(code, "mode of another head");
                }
                if head.mode_set {
                    return set_twice();
                }
                head.mode_set = true;
                head.setting.mode = Some(m);
            }
            config_head::Request::SetCustomMode {
                width,
                height,
                refresh,
            } => {
                let valid = desktop_config::displays::Mode {
                    width,
                    height,
                    refresh,
                }
                .is_valid();
                if !valid {
                    let code = config_head::Error::InvalidCustomMode;
                    return resource.post_error(code, "invalid custom mode");
                }
                if head.mode_set {
                    return set_twice();
                }
                head.mode_set = true;
                head.setting.mode = Some(Mode {
                    size: (width, height).into(),
                    refresh,
                });
            }
            config_head::Request::SetPosition { x, y } => {
                if head.setting.position.is_some() {
                    return set_twice();
                }
                head.setting.position = Some(Point::from((x, y)));
            }
            config_head::Request::SetTransform { transform } => {
                let WEnum::Value(transform) = transform else {
                    let code = config_head::Error::InvalidTransform;
                    return resource.post_error(code, "unknown transform");
                };
                if head.setting.transform.is_some() {
                    return set_twice();
                }
                head.setting.transform = Some(Transform::from(transform));
            }
            config_head::Request::SetScale { scale } => {
                if !(scale.is_finite() && scale > 0.0) {
                    let code = config_head::Error::InvalidScale;
                    return resource.post_error(code, "scale negative or zero");
                }
                if head.setting.scale.is_some() {
                    return set_twice();
                }
                head.setting.scale = Some(scale);
            }
            _ => {}
        }
    }
}
