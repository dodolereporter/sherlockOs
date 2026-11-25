//! Compositeur Wayland du desktop.
//!
//! `compositeur --imbrique [--mesurer] [commande [arguments…]]` ouvre le compositeur dans une
//! fenêtre de la session en cours, démarre XWayland, puis lance la commande en lui donnant sa
//! socket Wayland (`WAYLAND_DISPLAY`) et son affichage X11 (`DISPLAY`).
//! `--mesurer` journalise chaque seconde la cadence et le temps de rendu des images, puis un
//! bilan à la fin. La session s'arrête alors avec la commande.

use std::{
    ffi::{OsStr, OsString},
    process::Command,
};

use compositeur::{Error, State, stats::FrameStats};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
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
    let mut command: Vec<_> = args.collect();
    let measure = command.first().map(|a| a.as_os_str()) == Some(OsStr::new("--mesurer"));
    if measure {
        command.remove(0);
    }

    let mut event_loop = EventLoop::try_new()?;
    let mut state = State::new(&event_loop)?;
    if measure {
        state.frame_stats = Some(FrameStats::default());
    }
    let socket = state.listen(&event_loop)?;
    compositeur::winit::init(&event_loop, &mut state)?;
    tracing::info!(socket = ?socket, "compositeur prêt");

    // En mesure, la session s'arrête avec la commande, qui est la scène mesurée.
    let signal = measure.then(|| state.loop_signal.clone());
    // XWayland démarre avec le compositeur, et la commande une fois qu'il est prêt : elle peut
    // être une application X11. Sans Xwayland sur le poste, seules les applications Wayland
    // s'affichent.
    let launch = {
        let (command, socket, signal) = (command.clone(), socket.clone(), signal.clone());
        move |_: &mut State, display: Option<u32>| {
            if let Err(err) = spawn(&command, &socket, display, signal) {
                tracing::warn!("{err}");
            }
        }
    };
    if let Err(err) = state.start_xwayland(&event_loop, launch) {
        tracing::warn!("pas d'applications X11 : {err}");
        spawn(&command, &socket, None, signal)?;
    }

    event_loop.run(None, &mut state, State::refresh)?;
    state.stop_xwayland();
    if let Some(stats) = &state.frame_stats {
        tracing::info!("bilan, toutes les images : {}", stats.all);
        tracing::info!("bilan, images animées : {}", stats.animated);
    }
    Ok(())
}

/// Lance la commande, s'il y en a une, avec la socket Wayland du compositeur et son affichage
/// X11. Les deux passent par l'environnement de l'enfant seulement : le nôtre garde celui de la
/// session hôte. Sans affichage X11, l'enfant n'hérite pas de celui de l'hôte, où ses fenêtres
/// X11 s'ouvriraient hors du compositeur.
fn spawn(
    command: &[OsString],
    socket: &OsStr,
    display: Option<u32>,
    signal: Option<LoopSignal>,
) -> Result<(), Error> {
    let Some((program, arguments)) = command.split_first() else {
        return Ok(());
    };
    let mut child = Command::new(program);
    child.args(arguments).env("WAYLAND_DISPLAY", socket);
    match display {
        Some(n) => child.env("DISPLAY", format!(":{n}")),
        None => child.env_remove("DISPLAY"),
    };
    let mut child = child.spawn()?;
    // Un fil attend la fin de la commande, pour qu'elle ne reste pas zombie.
    std::thread::spawn(move || {
        match child.wait() {
            Ok(status) => tracing::info!(%status, "la commande lancée s'est terminée"),
            Err(err) => tracing::warn!("attente de la commande lancée : {err}"),
        }
        if let Some(signal) = signal {
            signal.stop();
        }
    });
    Ok(())
}
