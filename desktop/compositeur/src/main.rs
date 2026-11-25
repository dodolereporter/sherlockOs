//! Compositeur Wayland du desktop.
//!
//! `compositeur --imbrique [commande [arguments…]]` ouvre le compositeur dans une fenêtre de la
//! session en cours, puis lance la commande en lui donnant sa socket Wayland.

use std::{ffi::OsStr, process::Command};

use compositeur::{Error, State};
use smithay::reexports::calloop::EventLoop;
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Error> {
    // Niveau des journaux par RUST_LOG (par exemple `RUST_LOG=compositeur=debug`), info sinon.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args_os().skip(1);
    // Seul le backend fenêtré existe pour l'instant ; la session réelle vient au lot D-14.
    if args.next().as_deref() != Some(OsStr::new("--imbrique")) {
        return Err(Error::Usage);
    }
    let command: Vec<_> = args.collect();

    let mut event_loop = EventLoop::try_new()?;
    let mut state = State::new(&event_loop)?;
    let socket = state.listen(&event_loop)?;
    compositeur::winit::init(&event_loop, &mut state)?;
    tracing::info!(socket = ?socket, "compositeur prêt");

    if let Some((program, arguments)) = command.split_first() {
        // La socket passe par l'environnement de l'enfant seulement : le nôtre garde celui
        // de la session hôte.
        Command::new(program)
            .args(arguments)
            .env("WAYLAND_DISPLAY", &socket)
            .spawn()?;
    }

    event_loop.run(None, &mut state, State::refresh)?;
    Ok(())
}
