//! Clipboard history panel: `cargo run -p clipboard-panel`, in our compositor's session (it
//! needs the shell socket). Run it again to close it: the compositor starts it on Super + V.

use std::process::ExitCode;

use launcher::instance::{self, Claim};
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statements of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    // Held until the end of `main`: the kernel releases it with the process.
    let lock = match instance::runtime_dir()
        .map(|dir| instance::claim_named(&dir, clipboard_panel::LOCK))
    {
        Some(Ok(Claim::First(lock))) => Some(lock),
        Some(Ok(Claim::Second(closed))) => {
            tracing::info!(?closed, "a clipboard panel was open: closed instead");
            return ExitCode::SUCCESS;
        }
        Some(Err(error)) => {
            tracing::warn!(%error, "cannot take the panel lock: running anyway");
            None
        }
        None => None,
    };
    let result = clipboard_panel::run();
    if let Some(lock) = lock {
        instance::release(lock);
    }
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the clipboard panel stopped");
            ExitCode::FAILURE
        }
    }
}
