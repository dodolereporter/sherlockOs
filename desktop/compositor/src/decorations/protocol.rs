//! Who gets a decoration from the compositor. Two protocols ask for one:
//!
//! - `zxdg_decoration_manager_v1` (xdg-decoration), which Qt 6 and winit use. A new decoration,
//!   or a client that gives up its preference, gets `server_side`. A client that asks for
//!   `client_side` keeps its own decoration: Chromium draws its tabs in it, and
//!   would get a bar of ours on top of its own.
//! - `org_kde_kwin_server_decoration_manager`, KDE's older protocol, which GTK 3 and the Qt 5 of
//!   KDE use: `Server` by default, and the mode the client asks for afterwards.
//!
//! A client that binds neither, such as GTK 4 and libadwaita, draws its own decoration and gets
//! none.

use std::sync::atomic::{AtomicBool, Ordering};

use smithay::{
    delegate_kde_decoration, delegate_xdg_decoration,
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode,
        wayland_protocols_misc::server_decoration::server::{
            org_kde_kwin_server_decoration::{Mode as KdeMode, OrgKdeKwinServerDecoration},
            org_kde_kwin_server_decoration_manager::Mode as DefaultMode,
        },
        wayland_server::{DisplayHandle, WEnum, protocol::wl_surface::WlSurface},
    },
    wayland::{
        compositor::with_states,
        shell::{
            kde::decoration::{KdeDecorationHandler, KdeDecorationState},
            xdg::{
                ToplevelSurface,
                decoration::{XdgDecorationHandler, XdgDecorationState},
            },
        },
    },
};

use crate::State;

/// The globals of both protocols.
pub(super) fn globals(dh: &DisplayHandle) -> (XdgDecorationState, KdeDecorationState) {
    (
        XdgDecorationState::new::<State>(dh),
        KdeDecorationState::new::<State>(dh, DefaultMode::Server),
    )
}

/// The mode a surface has through KDE's protocol: `true` for `Server`.
struct KdeServerSide(AtomicBool);

/// Whether `surface` has a decoration of the compositor through KDE's protocol.
pub(super) fn kde_server_side(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        let mode = states.data_map.get::<KdeServerSide>();
        mode.is_some_and(|mode| mode.0.load(Ordering::Relaxed))
    })
}

fn set_kde_server_side(surface: &WlSurface, on: bool) {
    with_states(surface, |states| {
        let mode = || KdeServerSide(AtomicBool::new(false));
        states.data_map.insert_if_missing_threadsafe(mode);
        if let Some(mode) = states.data_map.get::<KdeServerSide>() {
            mode.0.store(on, Ordering::Relaxed);
        }
    });
}

impl State {
    /// Gives `toplevel` the decoration `mode`, with its next configure.
    fn set_decoration_mode(&mut self, toplevel: &ToplevelSurface, mode: Mode) {
        tracing::debug!(?mode, "xdg-decoration");
        toplevel.with_pending_state(|s| s.decoration_mode = Some(mode));
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
        self.decoration_changed(self.window_for(toplevel.wl_surface()));
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.set_decoration_mode(&toplevel, Mode::ServerSide);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: Mode) {
        self.set_decoration_mode(&toplevel, mode);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.set_decoration_mode(&toplevel, Mode::ServerSide);
    }
}

impl KdeDecorationHandler for State {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.decorations.kde
    }

    fn new_decoration(&mut self, surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration) {
        decoration.mode(KdeMode::Server);
        set_kde_server_side(surface, true);
        self.decoration_changed(self.window_for(surface));
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<KdeMode>,
    ) {
        let WEnum::Value(mode) = mode else {
            return;
        };
        tracing::debug!(?mode, "KDE server decoration");
        decoration.mode(mode);
        set_kde_server_side(surface, mode == KdeMode::Server);
        self.decoration_changed(self.window_for(surface));
    }

    fn release(&mut self, _decoration: &OrgKdeKwinServerDecoration, surface: &WlSurface) {
        set_kde_server_side(surface, false);
        self.decoration_changed(self.window_for(surface));
    }
}

delegate_xdg_decoration!(State);
delegate_kde_decoration!(State);
