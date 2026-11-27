//! Applications view: `cargo run -p apps`, in a Wayland session that supports layer-shell. Run
//! it again to close it.

use std::process::ExitCode;

use launcher::instance::{self, Claim};
use launcher::service::{self, Request};
use launcher::startup;
use tracing_subscriber::EnvFilter;

/// The option that starts the resident service.
const RESIDENT: &str = "--resident";

fn main() -> ExitCode {
    // Before anything else: the renderer is chosen through the environment (ADR 0018). It only
    // reads a few files of /sys, well under a millisecond.
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statements of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    startup::begin();
    let resident = std::env::args_os().nth(1).is_some_and(|a| a == RESIDENT);
    let dir = instance::runtime_dir();
    // A key or the dock's button while the service runs: handed over before anything that
    // costs, and done.
    if !resident
        && let Some(dir) = &dir
        && service::send(
            &service::socket(dir, service::APPLICATIONS),
            Request::Toggle,
        )
        .is_ok()
    {
        return ExitCode::SUCCESS;
    }
    // Log level from RUST_LOG (for example `RUST_LOG=apps=debug`), info otherwise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    startup::step("renderer chosen", startup::elapsed());
    tracing::info!(?renderer, "renderer");
    if resident {
        return serve(dir.as_deref());
    }
    // Held until the end of `main`: the kernel releases it with the process.
    let lock = match dir
        .as_deref()
        .map(|dir| instance::claim_named(dir, apps::LOCK))
    {
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
    let result = apps::run(None);
    if let Some(lock) = lock {
        instance::release(lock);
    }
    ended(result)
}

/// The resident service, for the whole session.
fn serve(dir: Option<&std::path::Path>) -> ExitCode {
    let Some(dir) = dir else {
        tracing::error!("no XDG_RUNTIME_DIR: no place for the service's socket");
        return ExitCode::FAILURE;
    };
    let listening = service::listen(dir, service::APPLICATIONS);
    let inbox = match listening.and_then(|s| s.map(|s| s.spawn()).transpose()) {
        Ok(Some(inbox)) => inbox,
        Ok(None) => {
            tracing::info!("the applications service already runs");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            tracing::error!(%error, "cannot listen as the applications service");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("applications service listening");
    ended(apps::run(Some(inbox)))
}

fn ended(result: Result<(), iced_exwlshell::Error>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the applications view stopped");
            ExitCode::FAILURE
        }
    }
}
