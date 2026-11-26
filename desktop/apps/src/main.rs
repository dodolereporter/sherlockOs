//! Applications view: `cargo run -p apps`, in a Wayland session that supports layer-shell. Run
//! it again to close it.

use std::process::ExitCode;

use launcher::instance::{self, Claim};
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    // Log level from RUST_LOG (for example `RUST_LOG=apps=debug`), info otherwise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    // Held until the end of `main`: the kernel releases it with the process.
    let lock = match instance::runtime_dir().map(|dir| instance::claim_named(&dir, apps::LOCK)) {
        Some(Ok(Claim::First(lock))) => Some(lock),
        Some(Ok(Claim::Second(closed))) => {
            tracing::info!(?closed, "the view was open: closed instead");
            return ExitCode::SUCCESS;
        }
        Some(Err(error)) => {
            tracing::warn!(%error, "cannot take the lock of the view: running anyway");
            None
        }
        None => {
            tracing::warn!("no XDG_RUNTIME_DIR: running without the lock of the view");
            None
        }
    };
    tracing::info!(?renderer, "renderer");
    let result = apps::run();
    if let Some(lock) = lock {
        instance::release(lock);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the applications view stopped");
            ExitCode::FAILURE
        }
    }
}
