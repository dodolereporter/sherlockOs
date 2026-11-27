//! `sherlockos-autostart`: starts the applications of the autostart folders, once, at the start
//! of the session (lot D-44, `docs/autostart.md`). It stops once the last one is started.

use std::{
    process::{Command, ExitCode, Stdio},
    time::Instant,
};

use autostart::{Context, plan};
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Log level from RUST_LOG, info otherwise. The session copies it to its log.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    let context = Context::from_env();
    let start = Instant::now();
    for (id, decision) in plan(&context) {
        let launch = match decision {
            Ok(launch) => launch,
            Err(skip) => {
                tracing::info!(id, ?skip, "autostart: left out");
                continue;
            }
        };
        if let Some(wait) = launch.delay.checked_sub(start.elapsed()) {
            std::thread::sleep(wait);
        }
        let mut command = Command::new(&launch.program);
        command.args(&launch.args).stdin(Stdio::null());
        if let Some(dir) = &launch.working_dir {
            command.current_dir(dir);
        }
        // Nothing goes through a shell. The child does not inherit a choice of renderer.
        sherlock_ui::gpu::for_child(&mut command);
        match command.spawn() {
            Ok(child) => tracing::info!(id, pid = child.id(), "autostart: started"),
            Err(err) => tracing::warn!(id, program = ?launch.program, "autostart: {err}"),
        }
    }
    ExitCode::SUCCESS
}
