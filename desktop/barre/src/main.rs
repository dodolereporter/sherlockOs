//! Barre du haut : `cargo run -p barre`, dans une session Wayland qui gère layer-shell.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // Niveau des journaux par RUST_LOG (par exemple `RUST_LOG=barre=debug`), info sinon.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
    match barre::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "la barre s'est arrêtée");
            ExitCode::FAILURE
        }
    }
}
