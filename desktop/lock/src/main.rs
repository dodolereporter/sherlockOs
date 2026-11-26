//! `sherlockos-lock`: the lock screen. The compositor starts it when the session locks (Super +
//! L); it can also be run by hand in a SherlockOS session, which it then locks.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    // Log level from RUST_LOG, info otherwise. The password is never logged.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    tracing::info!(?renderer, "renderer");
    match lock::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the lock screen stopped");
            ExitCode::FAILURE
        }
    }
}
