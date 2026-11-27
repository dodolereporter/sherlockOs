//! The compositor of the login screen (`--greeter`, lot D-24, ADR 0026). greetd starts
//! `compositor --greeter -- sherlockos-greeter` as its `greeter` user; the compositor shows that
//! one program and nothing of the desktop:
//!
//! - its windows cover their whole screen (fullscreen), and its anchored surfaces show as
//!   usual;
//! - no shortcut of the desktop: neither Mission Control, the launcher, the applications view,
//!   the workspaces, the snapping keys, the screenshots, nor the touchpad gestures. Every key
//!   goes to the greeter, except the console switch, Ctrl + Alt + Backspace and the volume and
//!   brightness keys;
//! - no shell socket (there is no bar nor dock), and nothing locks: no lock program, no lock on
//!   inactivity, on logind's demand nor on the lid;
//! - the screens still go off after the inactivity, after the veil (lot D-38), and the input
//!   still has its watchdog;
//! - the compositor stops with the greeter (`Options::stops_with_command`): greetd starts the
//!   chosen session once its greeter has left, and starts the greeter again if it failed;
//! - the background is the curtain, the picture of the lock, from the first frame; once the
//!   greeter has left, it fades to black in `CURTAIN_BLACK`, then the compositor stops: the
//!   session's compositor comes back from black to the same picture (`curtain.rs`, lot D-24e).

use sherlock_ui::lock::CURTAIN_BLACK;
use smithay::{
    desktop::Window,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::State;

impl State {
    /// Makes this compositor the login screen's. Call it before `listen`, so that the shell
    /// socket is never opened.
    pub fn enter_greeter_mode(&mut self) {
        self.greeter = true;
        self.stop_shell_socket();
        // The greeter is the one client, trusted: it may anchor surfaces (lot D-46).
        self.share_layer_shell();
        // Nothing to lock: the inactivity only turns the screens off, and its veil comes before
        // that alone.
        self.set_idle_lock(None);
        tracing::info!("login screen: one program, fullscreen, no shortcut of the desktop");
    }

    /// The greeter has left: the curtain fades to black, then the compositor stops.
    pub fn greeter_left(&mut self) {
        tracing::info!("the greeter has left: fading to black, then stopping");
        self.close_curtain();
        let stop = self.loop_signal.clone();
        let timer = Timer::from_duration(CURTAIN_BLACK);
        let inserted = self.loop_handle.insert_source(timer, move |_, _, _| {
            stop.stop();
            TimeoutAction::Drop
        });
        if inserted.is_err() {
            self.loop_signal.stop();
        }
    }

    /// This compositor shows the login screen.
    pub fn is_greeter(&self) -> bool {
        self.greeter
    }

    /// A new window of the login screen covers its screen from its first configure on: called
    /// before that configure.
    pub(crate) fn greeter_window(&mut self, window: &Window) {
        if self.greeter {
            self.rearrange(window, |arrangement, _| arrangement.fullscreen = true);
        }
    }
}
