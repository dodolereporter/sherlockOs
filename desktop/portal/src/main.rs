//! `sherlock-portal`: the portal backend on the session bus, started by the bus when
//! `xdg-desktop-portal` needs it; `sherlock-portal --pick` is its picker (`picker.rs`).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use portal::{
    BUS_NAME, OBJECT_PATH, picker,
    screencast::{SHARES_PATH, ScreenCast, Shares, reap},
};
use tracing_subscriber::EnvFilter;

fn main() -> std::process::ExitCode {
    let pick = std::env::args().nth(1).as_deref() == Some("--pick");
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    if pick {
        let request = picker::read_request();
        // The picker's window comes with lot D-28d4: until then, every request is cancelled.
        tracing::warn!(app = request.app_id, "no picker yet: nothing shared");
        return std::process::ExitCode::SUCCESS;
    }
    match serve() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("portal: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// How often the shares are looked at, to end those whose source went away.
const REAP: std::time::Duration = std::time::Duration::from_secs(1);

fn serve() -> zbus::Result<()> {
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let screencast = ScreenCast {
        sessions: Arc::clone(&sessions),
    };
    let bus = zbus::blocking::connection::Builder::session()?
        .name(BUS_NAME)?
        .serve_at(OBJECT_PATH, screencast)?
        .serve_at(
            SHARES_PATH,
            Shares {
                sessions: Arc::clone(&sessions),
            },
        )?
        .build()?;
    tracing::info!("portal backend on the bus");
    // The shares whose source went away end their session, once a second.
    loop {
        std::thread::sleep(REAP);
        async_io::block_on(reap(bus.inner(), &sessions));
    }
}
