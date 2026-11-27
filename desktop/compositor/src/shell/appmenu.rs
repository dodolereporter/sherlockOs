//! The global menu of each window (lot D-36): where its application exports it with
//! `com.canonical.dbusmenu` on the session bus, given to the bar by the `app_menu` event of
//! `sherlock_shell_v1`. Two ways in:
//!
//! - **`org_kde_kwin_appmenu_manager`**, KDE's protocol, offered to every client: a Qt
//!   application with KDE's platform theme sets the address of each surface itself.
//! - **The registrations the shell passes on** (`register_app_menu`): the bar serves
//!   `com.canonical.AppMenu.Registrar`, which Qt's generic theme and qt6ct call with the X11
//!   window id, or under Wayland with an id the compositor does not know. The registration names
//!   the process of the bus connection, which the bus daemon gives and the application cannot
//!   choose: the menu goes to the X11 window of that id and process, else to the newest window
//!   of a Wayland client of that process that has none yet, else to the next one that shows up.

use std::{cell::RefCell, sync::Mutex};

use smithay::{
    desktop::Window,
    reexports::wayland_server::{
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
        protocol::wl_surface::WlSurface,
    },
    wayland::{compositor::with_states, seat::WaylandFocus},
};
use wayland_protocols_plasma::appmenu::server::{
    org_kde_kwin_appmenu::{self, OrgKdeKwinAppmenu},
    org_kde_kwin_appmenu_manager::{self, OrgKdeKwinAppmenuManager},
};

use crate::State;

/// Version of `org_kde_kwin_appmenu_manager`: 2 adds `release`.
const VERSION: u32 = 2;

/// Registrations kept while no window takes them: a client that registers without ever
/// showing a window cannot make the list grow for ever.
const MAX_PENDING: usize = 64;

pub(super) fn create_global(dh: &DisplayHandle) {
    dh.create_global::<State, OrgKdeKwinAppmenuManager, ()>(VERSION, ());
}

/// Where a menu is exported: bus name and object path. Both empty for no menu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct MenuAddress {
    pub service: String,
    pub path: String,
}

impl MenuAddress {
    fn is_empty(&self) -> bool {
        self.service.is_empty() || self.path.is_empty()
    }
}

/// A `RegisterWindow` passed on by the shell.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Registration {
    /// Window id given to `RegisterWindow`.
    pub window: u32,
    /// Process of the bus connection that called it.
    pub pid: u32,
    pub address: MenuAddress,
}

/// Registrations that no window took yet.
#[derive(Default)]
pub(super) struct AppMenus {
    pending: Vec<Registration>,
}

/// The address a client gave to a surface with `org_kde_kwin_appmenu`.
#[derive(Default)]
struct KdeAddress(Mutex<MenuAddress>);

/// The registration a window took.
#[derive(Default)]
struct Registered(RefCell<Option<Registration>>);

fn set_kde_address(surface: &WlSurface, address: MenuAddress) {
    with_states(surface, |states| {
        let data = states
            .data_map
            .get_or_insert_threadsafe(KdeAddress::default);
        *data
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = address;
    });
}

fn kde_address(surface: &WlSurface) -> MenuAddress {
    with_states(surface, |states| {
        let data = states.data_map.get::<KdeAddress>();
        let address = data.map(|d| d.0.lock().map(|a| a.clone()));
        address.and_then(Result::ok).unwrap_or_default()
    })
}

fn registered(window: &Window) -> &Registered {
    window.user_data().get_or_insert(Registered::default)
}

impl State {
    /// The menu of `window`: the address its client set, else the registration it took.
    pub(super) fn app_menu(window: &Window) -> MenuAddress {
        if window.x11_surface().is_none()
            && let Some(surface) = window.wl_surface()
        {
            let address = kde_address(&surface);
            if !address.is_empty() {
                return address;
            }
        }
        let taken = registered(window).0.borrow();
        taken
            .as_ref()
            .map(|r| r.address.clone())
            .unwrap_or_default()
    }

    /// Process of the client of a Wayland window, or of an X11 window as it says.
    fn window_pid(&self, window: &Window) -> Option<u32> {
        if let Some(surface) = window.x11_surface() {
            return surface.pid();
        }
        let client = window.wl_surface()?.client()?;
        let credentials = client.get_credentials(&self.display_handle).ok()?;
        u32::try_from(credentials.pid).ok()
    }

    /// Keeps a registration until a window takes it.
    pub(super) fn register_app_menu(&mut self, registration: Registration) {
        tracing::debug!(?registration, "menu registered");
        let pending = &mut self.shell.app_menus.pending;
        pending.retain(|r| r.window != registration.window || r.address != registration.address);
        if pending.len() == MAX_PENDING {
            pending.remove(0);
        }
        pending.push(registration);
        self.assign_app_menus();
    }

    /// Withdraws the menu that `service` exported at `path`.
    pub(super) fn unregister_app_menu(&mut self, service: &str, path: &str) {
        let matches = |r: &Registration| r.address.service == service && r.address.path == path;
        self.shell.app_menus.pending.retain(|r| !matches(r));
        for shown in self.space.elements() {
            let mut taken = registered(shown).0.borrow_mut();
            if taken.as_ref().is_some_and(matches) {
                *taken = None;
            }
        }
    }

    /// Gives the pending registrations to their windows, when they are there.
    pub(super) fn assign_app_menus(&mut self) {
        if self.shell.app_menus.pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.shell.app_menus.pending);
        let windows: Vec<_> = self
            .space
            .elements()
            .filter(|w| crate::window_data(w).shown.get())
            .map(|w| (w.clone(), self.window_pid(w)))
            .collect();
        for registration in pending {
            let pid = Some(registration.pid);
            let x11 = windows.iter().find(|(w, window_pid)| {
                let id = w.x11_surface().map(|s| s.window_id());
                id == Some(registration.window) && (window_pid.is_none() || *window_pid == pid)
            });
            // Under Wayland the id means nothing here: the newest window of the process
            // without a menu.
            let wayland = || {
                let free = windows.iter().filter(|(w, window_pid)| {
                    w.x11_surface().is_none() && *window_pid == pid && State::app_menu(w).is_empty()
                });
                free.max_by_key(|(w, _)| super::window_id(w))
            };
            match x11.or_else(wayland) {
                Some((window, _)) => {
                    tracing::debug!(id = super::window_id(window), "menu given to a window");
                    *registered(window).0.borrow_mut() = Some(registration);
                }
                None => self.shell.app_menus.pending.push(registration),
            }
        }
    }
}

impl GlobalDispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<OrgKdeKwinAppmenuManager>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<OrgKdeKwinAppmenuManager, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &OrgKdeKwinAppmenuManager,
        request: org_kde_kwin_appmenu_manager::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let org_kde_kwin_appmenu_manager::Request::Create { id, surface } = request {
            data_init.init(id, surface);
        }
    }
}

impl Dispatch<OrgKdeKwinAppmenu, WlSurface> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &OrgKdeKwinAppmenu,
        request: org_kde_kwin_appmenu::Request,
        surface: &WlSurface,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if !surface.is_alive() {
            return;
        }
        match request {
            org_kde_kwin_appmenu::Request::SetAddress {
                service_name,
                object_path,
            } => {
                let address = MenuAddress {
                    service: service_name,
                    path: object_path,
                };
                tracing::debug!(?address, "menu address of a surface");
                set_kde_address(surface, address);
            }
            org_kde_kwin_appmenu::Request::Release => {
                set_kde_address(surface, MenuAddress::default());
            }
            _ => {}
        }
    }
}
