//! Launcher: `cargo run -p launcher`, in a Wayland session that supports layer-shell. Run it
//! again to close it. `launcher --resident` stays for the whole session, hidden; `launcher`
//! then shows or hides it at once (lot P-02, `docs/launcher.md`).

use std::process::ExitCode;

use launcher::instance::{self, Claim};
use launcher::service::{self, Request};
use launcher::startup;
use tracing_subscriber::EnvFilter;

/// The option that starts the resident service.
const RESIDENT: &str = "--resident";

fn main() -> ExitCode {
    startup::begin();
    let resident = std::env::args_os().nth(1).is_some_and(|a| a == RESIDENT);
    let dir = instance::runtime_dir();
    // A press while the service runs: handed over before anything that costs, and done.
    if !resident
        && let Some(dir) = &dir
        && service::send(&service::socket(dir, service::LAUNCHER), Request::Toggle).is_ok()
    {
        return ExitCode::SUCCESS;
    }
    // The renderer is chosen through the environment, before any thread exists (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: no other thread exists yet: the request above used none.
    unsafe { renderer.apply() };
    // Log level from RUST_LOG (for example `RUST_LOG=launcher=debug`), info otherwise.
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
    let lock = match dir.as_deref().map(instance::claim) {
        Some(Ok(Claim::First(lock))) => Some(lock),
        Some(Ok(Claim::Second(closed))) => {
            tracing::info!(?closed, "a launcher was open: closed instead");
            return ExitCode::SUCCESS;
        }
        Some(Err(error)) => {
            tracing::warn!(%error, "cannot take the launcher lock: running anyway");
            None
        }
        None => {
            tracing::warn!("no XDG_RUNTIME_DIR: running without the launcher lock");
            None
        }
    };
    let result = launcher::run(launcher::Mode::OnDemand);
    // A normal close (Escape, a start) leaves no process id behind.
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
    let inbox = match service::listen(dir, service::LAUNCHER)
        .and_then(|s| s.map(|s| s.spawn()).transpose())
    {
        Ok(Some(inbox)) => inbox,
        Ok(None) => {
            tracing::info!("the launcher service already runs");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            tracing::error!(%error, "cannot listen as the launcher service");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!("launcher service listening");
    ended(launcher::run(launcher::Mode::Resident(inbox)))
}

fn ended(result: Result<(), iced_exwlshell::Error>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the launcher stopped");
            ExitCode::FAILURE
        }
    }
}
