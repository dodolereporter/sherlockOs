//! `sherlockos-greeter`: the login screen. greetd starts it inside the compositor of the login
//! screen (`compositor --greeter -- sherlockos-greeter`); it ends once someone logged in, and
//! greetd then starts the session.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statements of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    // A panic aborts: the connection to greetd closes with the process, greetd drops the session
    // being created, the compositor stops with this program and greetd starts both again.
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("panic: {info}");
        std::process::abort();
    }));
    // Log level from RUST_LOG, info otherwise. The password is never logged.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    tracing::info!(?renderer, "renderer");
    match greeter::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the login screen stopped");
            ExitCode::FAILURE
        }
    }
}
