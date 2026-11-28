//! An application brings its window forward (lot Q-52): a click on its icon in the bar's
//! notification area, a notification, a second launch. Without it, a minimized window, one
//! behind others or on another workspace stayed where it was, whatever the application asked.
//!
//! - Wayland: `xdg-activation-v1`. A token is granted when it carries the serial of a recent
//!   press (button or key) that the compositor gave to the very client that asks for it: the
//!   bar, after a click on an icon, passes it to the application (`ProvideXdgActivationToken`,
//!   `XDG_ACTIVATION_TOKEN`), which activates its window with it.
//! - X11: `_NET_ACTIVE_WINDOW` (heard by `x11_activation`) and `WM_CHANGE_STATE` to
//!   `NormalState` (smithay's `unminimize_request`). A pager (source 2) is obeyed. An
//!   application (source 1) is obeyed when a token was granted a moment before (the click that
//!   made it ask: X11 applications cannot hand the token back), or when it already has the
//!   keyboard.
//!
//! Activated: out of its minimized state, its workspace shown, raised with the animation of
//! lot D-19d and given the keyboard (`State::activate`). Refused: no focus stolen, the window
//! asks for attention instead (the `attention` state of the shell protocol, which the dock
//! shows), until it gets the keyboard.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use smithay::{
    desktop::Window,
    input::Seat,
    reexports::wayland_server::{Resource, backend::ClientId, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Serial},
    wayland::xdg_activation::{
        XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
    },
    xwayland::X11Surface,
};

use crate::State;

/// How long a press lets its client grant a token, and a token or a grant stays usable.
pub const RECENT: Duration = Duration::from_secs(10);
/// Presses remembered: a burst of keys does not push the click that asks out of reach.
const PRESSES: usize = 32;
/// Grants kept for X11 at most: as many activations to come.
const GRANTS: usize = 4;

/// `_NET_ACTIVE_WINDOW`'s source indication: a pager, or the shell, acts for the user.
pub const SOURCE_PAGER: u32 = 2;

/// What the module keeps.
pub(crate) struct Activation {
    state: XdgActivationState,
    /// The last presses given to clients: serial, client, when.
    presses: VecDeque<(Serial, Option<ClientId>, Instant)>,
    /// When the last tokens were granted: an X11 application may use one each.
    grants: Grants,
    /// Windows that asked to come forward without the right: shown to the shell, until they
    /// get the keyboard.
    attention: Vec<Window>,
}

impl Activation {
    pub(crate) fn new(state: XdgActivationState) -> Self {
        Self {
            state,
            presses: VecDeque::new(),
            grants: Grants::default(),
            attention: Vec::new(),
        }
    }
}

/// Marks a token granted when it was created.
struct Granted;

/// Grants of the last tokens, for X11 applications, which cannot hand a token back: each one
/// lets one activation through within `RECENT`.
#[derive(Debug, Default)]
pub struct Grants(VecDeque<Instant>);

impl Grants {
    pub fn grant(&mut self, now: Instant) {
        self.0.push_back(now);
        while self.0.len() > GRANTS {
            self.0.pop_front();
        }
    }

    /// Uses the oldest grant still recent at `now`, if any.
    pub fn take(&mut self, now: Instant) -> bool {
        self.0
            .retain(|at| now.saturating_duration_since(*at) < RECENT);
        self.0.pop_front().is_some()
    }
}

impl State {
    /// A press (button or key) given with `serial` to the client of `surface`: it may ask for
    /// a token with it.
    pub(crate) fn note_press(&mut self, serial: Serial, surface: Option<&WlSurface>) {
        let client = surface.and_then(|s| s.client()).map(|c| c.id());
        let presses = &mut self.activation.presses;
        presses.push_back((serial, client, Instant::now()));
        while presses.len() > PRESSES {
            presses.pop_front();
        }
    }

    /// Whether `serial` is a recent press given to `client`.
    fn pressed(&self, serial: Serial, client: Option<&ClientId>) -> bool {
        client.is_some()
            && self
                .activation
                .presses
                .iter()
                .any(|(s, c, at)| *s == serial && c.as_ref() == client && at.elapsed() < RECENT)
    }

    /// An application asks to bring `window` forward: activated when `allowed`, else it asks
    /// for attention, and the keyboard stays where it is.
    pub fn request_activation(&mut self, window: &Window, allowed: bool) {
        if allowed {
            tracing::debug!(title = State::title(window), "activation granted");
            self.activate(window);
        } else if self.focused_window().as_ref() != Some(window) {
            tracing::debug!(
                title = State::title(window),
                "activation refused: attention"
            );
            if !self.activation.attention.contains(window) {
                self.activation.attention.push(window.clone());
            }
        }
    }

    /// Whether `window` asks for attention.
    pub fn needs_attention(&self, window: &Window) -> bool {
        self.activation.attention.contains(window)
    }

    /// `window` got the keyboard, or went: it no longer asks for attention.
    pub(crate) fn attention_done(&mut self, window: Option<&Window>) {
        self.activation
            .attention
            .retain(|w| w.alive() && Some(w) != window);
    }

    /// An X11 application asks to bring `surface` forward (`_NET_ACTIVE_WINDOW` with `source`,
    /// or `WM_CHANGE_STATE` to `NormalState`).
    pub(crate) fn x11_activation(&mut self, surface: &X11Surface, source: u32) {
        let Some(window) = self.x11_window(surface) else {
            return;
        };
        let focused = self.focused_window();
        // The application that has the keyboard may bring another of its windows forward.
        let same_application = focused
            .as_ref()
            .and_then(Window::x11_surface)
            .and_then(X11Surface::pid)
            .is_some_and(|pid| surface.pid() == Some(pid));
        let allowed = source == SOURCE_PAGER
            || same_application
            || self.activation.grants.take(Instant::now());
        self.request_activation(&window, allowed);
    }

    /// The X11 window of id `id`, shown or hidden.
    pub(crate) fn x11_surface_by_id(&self, id: u32) -> Option<X11Surface> {
        self.space
            .elements()
            .filter_map(Window::x11_surface)
            .find(|s| s.window_id() == id)
            .cloned()
    }
}

impl XdgActivationHandler for State {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.activation.state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        let granted = data.serial.as_ref().is_some_and(|(serial, seat)| {
            Seat::<State>::from_resource(seat).as_ref() == Some(&self.seat)
                && self.pressed(*serial, data.client_id.as_ref())
        });
        if granted {
            data.user_data.insert_if_missing_threadsafe(|| Granted);
            self.activation.grants.grant(Instant::now());
        }
        // Old tokens go: a client cannot fill the compositor with them.
        self.activation
            .state
            .retain_tokens(|_, d| d.timestamp.elapsed() < RECENT);
        true
    }

    fn request_activation(
        &mut self,
        token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        let granted = token_data.user_data.get::<Granted>().is_some()
            && token_data.timestamp.elapsed() < RECENT;
        // One use per token.
        self.activation.state.remove_token(&token);
        if let Some(window) = self.window_for(&surface) {
            State::request_activation(self, &window, granted);
        }
    }
}

smithay::delegate_xdg_activation!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_grant_lets_one_x11_activation_through_while_recent() {
        let mut grants = Grants::default();
        let now = Instant::now();
        assert!(!grants.take(now), "nothing granted");
        grants.grant(now);
        assert!(grants.take(now + Duration::from_secs(1)));
        assert!(!grants.take(now + Duration::from_secs(1)), "used once");
        grants.grant(now);
        assert!(!grants.take(now + RECENT), "too old");
        for _ in 0..10 {
            grants.grant(now);
        }
        let taken = (0..10).filter(|_| grants.take(now)).count();
        assert_eq!(taken, GRANTS);
    }
}
