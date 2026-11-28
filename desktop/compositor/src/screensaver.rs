//! `org.freedesktop.ScreenSaver` and `org.freedesktop.PowerManagement.Inhibit` on the session
//! bus (lot D-43): what Chrome and Electron in X11, VLC and the older players ask to keep the
//! session awake during a video. `Inhibit` gives a cookie, `UnInhibit` gives it back; a program
//! that leaves the bus gives back all of its own. Only the program that took a cookie may give
//! it back. The count of cookies held goes to the loop (`inhibit.rs`).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use smithay::reexports::calloop::channel;
use zbus::message::Header;

use crate::inhibit::Event;

/// Paths of the service: Chrome asks the first one, some programs the second.
const SCREENSAVER_PATHS: [&str; 2] = ["/org/freedesktop/ScreenSaver", "/ScreenSaver"];
const POWER_PATH: &str = "/org/freedesktop/PowerManagement/Inhibit";
const NAMES: [&str; 3] = [
    "org.freedesktop.ScreenSaver",
    "org.freedesktop.PowerManagement",
    "org.freedesktop.PowerManagement.Inhibit",
];

/// The cookies given, with the unique bus name of who holds each.
#[derive(Default)]
pub struct Cookies {
    next: u32,
    held: HashMap<u32, String>,
}

impl Cookies {
    /// A new cookie for `holder`. Never 0, which some programs take for a failure.
    pub fn take(&mut self, holder: &str) -> u32 {
        self.next = self.next.wrapping_add(1).max(1);
        while self.held.contains_key(&self.next) {
            self.next = self.next.wrapping_add(1).max(1);
        }
        self.held.insert(self.next, holder.to_owned());
        self.next
    }

    /// `holder` gives `cookie` back. Another program's cookie stays.
    pub fn give_back(&mut self, holder: &str, cookie: u32) {
        if self.held.get(&cookie).is_some_and(|h| h == holder) {
            self.held.remove(&cookie);
        }
    }

    /// `holder` left the bus: its cookies go.
    pub fn forget(&mut self, holder: &str) {
        self.held.retain(|_, h| h != holder);
    }

    pub fn count(&self) -> usize {
        self.held.len()
    }
}

/// One object of the service, sharing the cookies with the others.
struct Service {
    cookies: Arc<Mutex<Cookies>>,
    sender: channel::Sender<Event>,
}

impl Service {
    fn change(&self, change: impl FnOnce(&mut Cookies)) {
        let mut cookies = self.cookies.lock().unwrap_or_else(PoisonError::into_inner);
        change(&mut cookies);
        let _ = self.sender.send(Event::Cookies(cookies.count()));
    }

    fn inhibit(&self, header: &Header<'_>, application: &str, reason: &str) -> u32 {
        let holder = header.sender().map(|s| s.to_string()).unwrap_or_default();
        tracing::info!(application, reason, holder, "idle inhibited");
        let mut cookie = 0;
        self.change(|cookies| cookie = cookies.take(&holder));
        cookie
    }

    fn uninhibit(&self, header: &Header<'_>, cookie: u32) {
        let holder = header.sender().map(|s| s.to_string()).unwrap_or_default();
        tracing::info!(cookie, holder, "idle inhibitor given back");
        self.change(|cookies| cookies.give_back(&holder, cookie));
    }
}

struct ScreenSaver(Service);

#[zbus::interface(name = "org.freedesktop.ScreenSaver")]
impl ScreenSaver {
    fn inhibit(
        &self,
        #[zbus(header)] header: Header<'_>,
        application_name: &str,
        reason_for_inhibit: &str,
    ) -> u32 {
        self.0
            .inhibit(&header, application_name, reason_for_inhibit)
    }

    fn un_inhibit(&self, #[zbus(header)] header: Header<'_>, cookie: u32) {
        self.0.uninhibit(&header, cookie);
    }
}

struct PowerManagement(Service);

#[zbus::interface(name = "org.freedesktop.PowerManagement.Inhibit")]
impl PowerManagement {
    fn inhibit(&self, #[zbus(header)] header: Header<'_>, application: &str, reason: &str) -> u32 {
        self.0.inhibit(&header, application, reason)
    }

    fn un_inhibit(&self, #[zbus(header)] header: Header<'_>, cookie: u32) {
        self.0.uninhibit(&header, cookie);
    }

    fn has_inhibit(&self) -> bool {
        let cookies = self
            .0
            .cookies
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        cookies.count() > 0
    }
}

/// Serves the objects on the session bus (`address`, or the environment's), then follows who
/// leaves the bus, until it closes.
pub(crate) fn serve(
    address: Option<String>,
    sender: &channel::Sender<Event>,
) -> Result<(), zbus::Error> {
    use zbus::blocking::{Connection, connection::Builder, fdo::DBusProxy};
    let cookies = Arc::new(Mutex::new(Cookies::default()));
    let service = || Service {
        cookies: cookies.clone(),
        sender: sender.clone(),
    };
    let connection: Connection = match address {
        Some(address) => Builder::address(address.as_str())?,
        None => Builder::session()?,
    }
    .serve_at(SCREENSAVER_PATHS[0], ScreenSaver(service()))?
    .serve_at(SCREENSAVER_PATHS[1], ScreenSaver(service()))?
    .serve_at(POWER_PATH, PowerManagement(service()))?
    .build()?;
    for name in NAMES {
        if let Err(err) = connection.request_name(name) {
            tracing::warn!(name, "screensaver service: {err}");
        }
    }
    let bus = DBusProxy::new(&connection)?;
    for change in bus.receive_name_owner_changed()? {
        let Ok(args) = change.args() else {
            continue;
        };
        // A unique name that no longer has an owner: the program left the bus.
        if args.new_owner().is_none() && args.name().starts_with(':') {
            let holder = args.name().to_string();
            let service = service();
            service.change(|cookies| cookies.forget(&holder));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_go_back_to_their_holder_only() {
        let mut cookies = Cookies::default();
        let a = cookies.take(":1.10");
        let b = cookies.take(":1.11");
        assert!(a != 0 && b != 0 && a != b);
        cookies.give_back(":1.11", a);
        assert_eq!(cookies.count(), 2, "another program's cookie stays");
        cookies.give_back(":1.10", a);
        assert_eq!(cookies.count(), 1);
        cookies.take(":1.11");
        cookies.forget(":1.11");
        assert_eq!(cookies.count(), 0);
    }
}
