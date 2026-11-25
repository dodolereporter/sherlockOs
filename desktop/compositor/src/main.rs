//! Wayland compositor of the desktop.
//!
//! `compositor --nested [--measure] [command [arguments…]]` opens the compositor in a window of
//! the current session, starts XWayland, then launches the command, giving it its Wayland
//! socket (`WAYLAND_DISPLAY`) and its X11 display (`DISPLAY`).
//! `--measure` logs the frame rate and frame render time every second, then a summary at the
//! end. The session then stops with the command.
//! `--outputs N` splits the window into N screens side by side. `--scale S` sets the scale of
//! the screens (`1`, `1.25` or `1,25`…); given several times, one per screen in order.

use std::{
    ffi::{OsStr, OsString},
    process::Command,
};

use compositor::{Error, State, stats::FrameStats};
use smithay::reexports::calloop::{EventLoop, LoopSignal};
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Error> {
    // Log level from RUST_LOG (for example `RUST_LOG=compositor=debug`), info otherwise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let mut args = std::env::args_os().skip(1);
    // Only the windowed backend exists for now; the real session comes with lot D-14.
    if args.next().as_deref() != Some(OsStr::new("--nested")) {
        return Err(Error::Usage);
    }
    let mut command: Vec<_> = args.collect();
    let (mut measure, mut screens, mut scales) = (false, 1, Vec::new());
    loop {
        match command.first().and_then(|a| a.to_str()) {
            Some("--measure") => measure = true,
            Some("--outputs") => screens = option(&mut command, 1..=4)?,
            Some("--scale") => scales.push(option(&mut command, 0.5..=4.0)?),
            _ => break,
        }
        command.remove(0);
    }
    // Missing scales repeat the last one given, or 1.
    let scales: Vec<f64> = (0..screens)
        .map(|i| scales.get(i).or(scales.last()).copied().unwrap_or(1.0))
        .collect();

    let mut event_loop = EventLoop::try_new()?;
    let mut state = State::new(&event_loop)?;
    if measure {
        state.frame_stats = Some(FrameStats::default());
    }
    let socket = state.listen(&event_loop)?;
    compositor::winit::init(&event_loop, &mut state, &scales)?;
    tracing::info!(socket = ?socket, "compositor ready");

    // When measuring, the session stops with the command, which is the measured scene.
    let signal = measure.then(|| state.loop_signal.clone());
    // XWayland starts with the compositor, and the command once it is ready: it may be an X11
    // application. Without Xwayland on the machine, only Wayland applications show up.
    let launch = {
        let (command, socket, signal) = (command.clone(), socket.clone(), signal.clone());
        move |_: &mut State, display: Option<u32>| {
            if let Err(err) = spawn(&command, &socket, display, signal) {
                tracing::warn!("{err}");
            }
        }
    };
    if let Err(err) = state.start_xwayland(&event_loop, launch) {
        tracing::warn!("no X11 applications: {err}");
        spawn(&command, &socket, None, signal)?;
    }

    event_loop.run(None, &mut state, State::refresh)?;
    state.shutdown();
    if let Some(stats) = &state.frame_stats {
        tracing::info!("summary, all frames: {}", stats.all);
        tracing::info!("summary, animated frames: {}", stats.animated);
    }
    Ok(())
}

/// Value of the option in front of `args`, which is removed with it, the option itself being
/// left for the caller. A decimal comma is read as a point.
fn option<T: std::str::FromStr + PartialOrd>(
    args: &mut Vec<OsString>,
    range: std::ops::RangeInclusive<T>,
) -> Result<T, Error> {
    if args.len() < 2 {
        return Err(Error::Usage);
    }
    let value = args.remove(1);
    let value = value.to_str().map(|v| v.replace(',', "."));
    value
        .and_then(|v| v.parse().ok())
        .filter(|v| range.contains(v))
        .ok_or(Error::Usage)
}

/// Launches the command, if there is one, with the compositor's Wayland socket and its X11
/// display. Both go through the child's environment only: ours keeps the host session's. Without
/// an X11 display, the child does not inherit the host's, where its X11 windows would open
/// outside the compositor.
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
    // A thread waits for the command to end, so that it does not stay a zombie.
    std::thread::spawn(move || {
        match child.wait() {
            Ok(status) => tracing::info!(%status, "the launched command has ended"),
            Err(err) => tracing::warn!("waiting for the launched command: {err}"),
        }
        if let Some(signal) = signal {
            signal.stop();
        }
    });
    Ok(())
}
