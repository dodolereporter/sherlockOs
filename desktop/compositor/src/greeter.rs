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
//! - the compositor stops with the greeter (`Options::stops_with_command`): greetd starts the
//!   chosen session once its greeter has left, and starts the greeter again if it failed.

use smithay::desktop::Window;

use crate::State;

impl State {
    /// Makes this compositor the login screen's. Call it before `listen`, so that the shell
    /// socket is never opened.
    pub fn enter_greeter_mode(&mut self) {
        self.greeter = true;
        self.stop_shell_socket();
        tracing::info!("login screen: one program, fullscreen, no shortcut of the desktop");
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
