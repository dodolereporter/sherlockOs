//! What the Sherlock menu starts (lot D-36d): Control Room, for « Réglages » and « À propos de
//! cet ordinateur ». Started without a shell, with the renderer variables the bar was given
//! (`sherlock_ui::gpu::for_child`), and waited for on a thread of its own: no zombie is left
//! when it ends.

use std::fmt;
use std::io;
use std::process::Command;
use std::sync::Arc;

/// The settings application.
pub const CONTROL_ROOM: &str = "control-room";
/// Its network page, for a network the bar cannot join (lot D-26n).
pub const NETWORK_PAGE: [&str; 2] = ["--page", "network"];

/// Starts a program by its name, found in `PATH`.
pub trait Start: Send + Sync {
    fn start(&self, program: &str) -> io::Result<()>;

    /// Starts `program` with `args`; by default, as `start` (the tests' starters).
    fn start_with(&self, program: &str, _args: &[&str]) -> io::Result<()> {
        self.start(program)
    }
}

/// The real thing: a child process, waited for on a thread.
pub struct Spawn;

impl Start for Spawn {
    fn start(&self, program: &str) -> io::Result<()> {
        self.start_with(program, &[])
    }

    fn start_with(&self, program: &str, args: &[&str]) -> io::Result<()> {
        let mut command = Command::new(program);
        command.args(args);
        sherlock_ui::gpu::for_child(&mut command);
        let mut child = command.spawn()?;
        let name = program.to_owned();
        std::thread::Builder::new()
            .name(String::from("bar-child"))
            .spawn(move || match child.wait() {
                Ok(status) => tracing::debug!(program = name, %status, "program ended"),
                Err(error) => tracing::warn!(program = name, %error, "cannot wait for the program"),
            })?;
        Ok(())
    }
}

/// Starts nothing: what a bar made by `Default` has, as in the tests. `Bar::new` takes `Spawn`.
pub struct Nothing;

impl Start for Nothing {
    fn start(&self, program: &str) -> io::Result<()> {
        Err(io::Error::other(format!(
            "{program}: no program is started here"
        )))
    }
}

/// What starts the programs of the menu: `Spawn`, or a test's.
#[derive(Clone)]
pub struct Programs(pub Arc<dyn Start>);

impl fmt::Debug for Programs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Programs")
    }
}

impl Default for Programs {
    fn default() -> Self {
        Self(Arc::new(Nothing))
    }
}

impl Programs {
    /// Starts `program`, logging when it cannot.
    pub fn start(&self, program: &str) {
        self.start_with(program, &[]);
    }

    /// Starts `program` with `args`, logging when it cannot.
    pub fn start_with(&self, program: &str, args: &[&str]) {
        match self.0.start_with(program, args) {
            Ok(()) => tracing::info!(program, "program started from the Sherlock menu"),
            Err(error) => tracing::warn!(program, %error, "cannot start the program"),
        }
    }
}
