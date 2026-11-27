//! Wayland compositor of the desktop.
//!
//! `compositor [command [arguments…]]`, from a text console or the login screen, is the real
//! session: the screens, keyboard, mouse and touchpad of the machine (lot D-14). It refuses to
//! start inside another graphical session.
//! `compositor --nested [command [arguments…]]` opens the compositor in a window of the
//! current session instead. Either way it starts XWayland, then launches the command, giving it
//! its Wayland socket (`WAYLAND_DISPLAY`) and its X11 display (`DISPLAY`).
//! `--measure` logs the frame rate and frame render time every second, then a summary at the
//! end. The session then stops with the command.
//! `--headless` runs it without any screen: one virtual screen of 1440 × 900 per scale, nothing
//! drawn, for real applications checked without a window (lot D-36).
//! `--outputs N` splits the window into N screens side by side. `--scale S` sets the scale of
//! the screens (`1`, `1.25` or `1,25`…); given several times, one per screen in order.
//! `--workspaces N` sets the number of workspaces, from 1 to 9 (3 by default). `--snap-gap PX`
//! sets the margin around snapped windows, from 0 to 64 (8 by default). `--minimize-effect`
//! chooses what minimizing a window shows: `genie` (by default), `scale` or `none`.
//! `--window-animations on|off` turns the opening and closing of the windows on (by default)
//! or off. `--greeter` makes it the compositor of the login screen, which greetd starts as
//! `compositor --greeter -- sherlockos-greeter` (lot D-24, ADR 0026): one program shown
//! fullscreen, no shortcut of the desktop, and the compositor stops with it.

use std::{
    ffi::{OsStr, OsString},
    process::Command,
};

use compositor::{Error, State, mesa, options::Options, stats::FrameStats, udev};
use smithay::reexports::calloop::{
    EventLoop, LoopSignal,
    channel::{self, Event, Sender},
};
use tracing_subscriber::EnvFilter;

fn main() -> Result<(), Error> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    options.check_environment(|key| std::env::var_os(key))?;
    if !options.nested {
        // The real session renders with Mesa only, and so do the programs it launches: NVIDIA's
        // EGL library is never loaded (it comes first on Debian), so that the card stays asleep.
        // SAFETY: no thread exists yet: only the arguments and the environment have been read.
        unsafe { mesa::restrict_egl_to_mesa() };
    }

    // Log level from RUST_LOG (for example `RUST_LOG=compositor=debug`), info otherwise.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let mut event_loop = EventLoop::try_new()?;
    let mut state = State::new(&event_loop)?;
    // Everything the compositor draws follows the theme of the appearance file.
    compositor::theme::follow(&event_loop.handle(), &mut state);
    if options.measure {
        state.frame_stats = Some(FrameStats::default());
    }
    if let Some(count) = options.workspaces {
        state.set_workspace_count(count);
    }
    if let Some(gap) = options.snap_gap {
        state.set_snap_gap(gap);
    }
    if let Some(effect) = options.minimize_effect {
        state.set_minimize_effect(effect);
    }
    if let Some(on) = options.window_animations {
        state.set_window_animations(on);
    }
    if options.greeter {
        state.enter_greeter_mode();
    }
    let socket = state.listen(&event_loop)?;
    // The real session's devices are released before the seat when leaving, whatever happens.
    let session = if options.headless {
        compositor::headless::init(&event_loop, &mut state, &options.scales)?;
        None
    } else if options.nested {
        compositor::winit::init(&event_loop, &mut state, &options.scales)?;
        None
    } else {
        Some(udev::init(&event_loop, &mut state, &options)?)
    };
    tracing::info!(socket = ?socket, "compositor ready");

    // When measuring, the session stops with the command, which is the measured scene; the
    // login screen stops with its greeter.
    let signal = options
        .stops_with_command()
        .then(|| state.loop_signal.clone());
    // The login screen fades its curtain to black once its greeter has left, then stops
    // (lot D-24e).
    let left = if options.greeter {
        let (sender, ended) = channel::channel::<()>();
        event_loop
            .handle()
            .insert_source(ended, |event, _, state| {
                if let Event::Msg(()) = event {
                    state.greeter_left();
                }
            })
            .map_err(|err| Error::Session(format!("greeter: {}", err.error)))?;
        Some(sender)
    } else {
        None
    };
    // XWayland starts with the compositor, and the command once it is ready: it may be an X11
    // application. Without Xwayland on the machine, only Wayland applications show up.
    let launch = {
        let (command, socket) = (options.command.clone(), socket.clone());
        let (signal, left) = (signal.clone(), left.clone());
        move |_: &mut State, display: Option<u32>| {
            if let Err(err) = spawn(&command, &socket, display, signal, left) {
                tracing::warn!("{err}");
            }
        }
    };
    let xwayland = if options.xwayland() {
        state.start_xwayland(&event_loop, launch)
    } else {
        Err(Error::NoXwayland)
    };
    if let Err(err) = xwayland {
        tracing::warn!("no X11 applications: {err}");
        spawn(&options.command, &socket, None, signal, left)?;
    }

    // Each iteration ends with the cleanup, and is measured from the end of the previous one:
    // its wait for an event included (lot P-01).
    event_loop.run(None, &mut state, |state| {
        state.refresh();
        state.perf.end_iteration();
    })?;
    state.shutdown();
    if let Some(session) = session {
        session.close(&mut state);
    }
    tracing::info!("event loop, whole session: {}", state.perf.session());
    if let Some(stats) = &state.frame_stats {
        tracing::info!("summary, all frames: {}", stats.all);
        tracing::info!("summary, animated frames: {}", stats.animated);
    }
    Ok(())
}

/// Launches the command, if there is one, with the compositor's Wayland socket and its X11
/// display. Both go through the child's environment only: ours keeps the host session's. Without
/// an X11 display, the child does not inherit the host's, where its X11 windows would open
/// outside the compositor. Once it ends, `left` hears it (the login screen), or `signal` stops
/// the loop.
fn spawn(
    command: &[OsString],
    socket: &OsStr,
    display: Option<u32>,
    signal: Option<LoopSignal>,
    left: Option<Sender<()>>,
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
        let heard = left.is_some_and(|left| left.send(()).is_ok());
        if let (false, Some(signal)) = (heard, signal) {
            signal.stop();
        }
    });
    Ok(())
}
