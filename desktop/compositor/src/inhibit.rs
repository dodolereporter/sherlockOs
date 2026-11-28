//! What keeps the session awake while a video plays (lot D-43): the inactivity neither locks the
//! session nor (with lot D-38) dims or turns the screens off while
//!
//! - a **visible** surface holds an idle inhibitor (`zwp_idle_inhibit_manager_v1`: Firefox, mpv,
//!   Chrome in Wayland). A surface hidden, minimized or on another workspace does not count, as
//!   the protocol says;
//! - a program holds a cookie of `org.freedesktop.ScreenSaver` or
//!   `org.freedesktop.PowerManagement.Inhibit` on the session bus (Chrome and Electron in X11,
//!   VLC), until it gives it back or leaves the bus (`screensaver.rs`);
//! - logind blocks `idle` (`systemd-inhibit --what=idle`).
//!
//! The delay of inactivity starts again when the last one goes. None of them stops Super + L,
//! the lid, logind's `Lock` nor the Lock button: those lock without looking at inactivity.

use std::time::Duration;

use smithay::{
    delegate_idle_inhibit,
    desktop::{WindowSurfaceType, layer_map_for_output},
    reexports::{
        calloop::{LoopHandle, channel},
        wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    },
    utils::IsAlive,
    wayland::{
        compositor,
        idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState},
    },
};

use crate::{State, screensaver};

/// How often logind's `BlockInhibited` is read: logind announces no change of it.
const LOGIND_EVERY: Duration = Duration::from_secs(5);

/// What the bus threads tell the loop.
#[derive(Debug)]
pub enum Event {
    /// Cookies of the session bus services now held.
    Cookies(usize),
    /// logind blocks `idle`, or no longer.
    Logind(bool),
}

/// The inhibitors held by the state.
pub struct Inhibitors {
    _manager: IdleInhibitManagerState,
    /// Surfaces with an idle inhibitor.
    surfaces: Vec<WlSurface>,
    cookies: usize,
    logind: bool,
}

impl Inhibitors {
    pub(crate) fn new(dh: &DisplayHandle) -> Self {
        Self {
            _manager: IdleInhibitManagerState::new::<State>(dh),
            surfaces: Vec::new(),
            cookies: 0,
            logind: false,
        }
    }
}

impl State {
    /// The inactivity waits: a visible video, a cookie on the bus, or logind.
    pub fn idle_inhibited(&self) -> bool {
        let inhibitors = &self.inhibitors;
        inhibitors.logind
            || inhibitors.cookies > 0
            || inhibitors
                .surfaces
                .iter()
                .any(|s| s.alive() && self.surface_shown(s))
    }

    /// The surface is on the screen: in a window shown, or in an anchored surface (layer).
    fn surface_shown(&self, surface: &WlSurface) -> bool {
        let mut root = surface.clone();
        while let Some(parent) = compositor::get_parent(&root) {
            root = parent;
        }
        if let Some(window) = self.window_for(&root) {
            return self.is_visible(&window) && crate::window_data(&window).shown.get();
        }
        self.space.outputs().any(|output| {
            layer_map_for_output(output)
                .layer_for_surface(&root, WindowSurfaceType::TOPLEVEL)
                .is_some()
        })
    }

    /// Serves `org.freedesktop.ScreenSaver` and `org.freedesktop.PowerManagement.Inhibit` on
    /// the session bus (`session`, or the one of the environment), and reads logind's
    /// `BlockInhibited` on the system bus (`system`, or the machine's), on threads of their own.
    pub fn watch_inhibitors(
        &mut self,
        handle: &LoopHandle<'static, State>,
        session: Option<String>,
        system: Option<String>,
    ) -> Result<(), crate::Error> {
        let (sender, receiver) = channel::channel();
        handle
            .insert_source(receiver, |event, _, state| {
                if let channel::Event::Msg(event) = event {
                    state.inhibitor_event(event);
                }
            })
            .map_err(|err| err.error)?;
        let logind = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("screensaver".into())
            .spawn(move || {
                if let Err(err) = screensaver::serve(session, &sender) {
                    tracing::warn!("screensaver service: {err}: X11 videos do not keep awake");
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("screensaver service: no thread: {err}");
        }
        let spawned = std::thread::Builder::new()
            .name("logind-idle".into())
            .spawn(move || {
                if let Err(err) = watch_logind(system, &logind) {
                    tracing::warn!("logind: {err}: its idle inhibitors are not followed");
                }
            });
        if let Err(err) = spawned {
            tracing::warn!("logind: no thread: {err}");
        }
        Ok(())
    }

    fn inhibitor_event(&mut self, event: Event) {
        tracing::debug!(?event, "idle inhibitors");
        match event {
            Event::Cookies(count) => self.inhibitors.cookies = count,
            Event::Logind(blocked) => self.inhibitors.logind = blocked,
        }
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        tracing::debug!("idle inhibitor on a surface");
        self.inhibitors.surfaces.retain(IsAlive::alive);
        self.inhibitors.surfaces.push(surface);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        let surfaces = &mut self.inhibitors.surfaces;
        if let Some(i) = surfaces.iter().position(|s| *s == surface) {
            surfaces.remove(i);
        }
    }
}

delegate_idle_inhibit!(State);

/// Reads whether a program blocks `idle` through logind, every `LOGIND_EVERY`, and says so when
/// it changes.
fn watch_logind(
    address: Option<String>,
    sender: &channel::Sender<Event>,
) -> Result<(), zbus::Error> {
    use zbus::{
        blocking::{Connection, Proxy, connection::Builder, proxy},
        proxy::CacheProperties,
    };
    let connection = match address {
        Some(address) => Builder::address(address.as_str())?.build()?,
        None => Connection::system()?,
    };
    // Not cached: the cache would keep the first value, logind never says it changed.
    let manager: Proxy = proxy::Builder::new(&connection)
        .destination("org.freedesktop.login1")?
        .path("/org/freedesktop/login1")?
        .interface("org.freedesktop.login1.Manager")?
        .cache_properties(CacheProperties::No)
        .build()?;
    let mut last = None;
    loop {
        let blocked: String = manager.get_property("BlockInhibited")?;
        let idle = blocks_idle(&blocked);
        if last != Some(idle) {
            last = Some(idle);
            if sender.send(Event::Logind(idle)).is_err() {
                return Ok(());
            }
        }
        std::thread::sleep(LOGIND_EVERY);
    }
}

/// `BlockInhibited` is the list of what is blocked, separated by colons.
fn blocks_idle(blocked: &str) -> bool {
    blocked.split(':').any(|what| what == "idle")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logind_blocks_idle_when_listed() {
        assert!(blocks_idle("idle"));
        assert!(blocks_idle("shutdown:sleep:idle"));
        assert!(!blocks_idle("shutdown:sleep"));
        assert!(!blocks_idle("handle-lid-switch:idle-x"));
        assert!(!blocks_idle(""));
    }
}
