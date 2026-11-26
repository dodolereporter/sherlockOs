//! Notification daemon: `cargo run -p notifications`, in a Wayland session that supports
//! layer-shell, and whose session bus has no other notification daemon.

use std::process::ExitCode;

use notifications::Outcome;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    // Log level from RUST_LOG (for example `RUST_LOG=notifications=debug`), info otherwise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    tracing::info!(?renderer, "renderer");
    match notifications::run() {
        Ok(Outcome::Stopped) => ExitCode::SUCCESS,
        // Not an error of this program: the session already shows notifications.
        Ok(Outcome::Taken) => {
            tracing::error!(
                name = notifications::server::NAME,
                "another notification daemon owns the name on the session bus: stopping"
            );
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "the notification daemon stopped");
            ExitCode::FAILURE
        }
    }
}
