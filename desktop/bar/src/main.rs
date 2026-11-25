//! Top bar: `cargo run -p bar`, in a Wayland session that supports layer-shell.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Log level from RUST_LOG (for example `RUST_LOG=bar=debug`), info otherwise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    match bar::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the bar stopped");
            ExitCode::FAILURE
        }
    }
}
