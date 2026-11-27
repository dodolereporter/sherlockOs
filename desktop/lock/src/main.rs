//! `sherlockos-lock`: the lock screen. The compositor starts it when the session locks (Super +
//! L); it can also be run by hand in a SherlockOS session, which it then locks.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    // First of all, before the password can be typed: no other process of the user may trace
    // this program or read its memory, and a panic writes no core dump (lot D-25k). Logged once
    // the log is set up.
    let private = lock::keep_memory_private();
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    // `--rescue`: the rescue mode, which the compositor starts once the lock screen failed
    // three times; in software, whatever the machine (lot D-25c).
    let renderer = sherlock_ui::gpu::Choice::detect().or_rescue(lock::rescue_asked());
    // SAFETY: first statements of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    let rescue = renderer == sherlock_ui::gpu::Choice::Software(sherlock_ui::gpu::Reason::Rescue);
    // A panic aborts, without unwinding through the toolkit: a clean exit of the toolkit would
    // end the lock (see `app.rs`). The compositor keeps the session locked and starts the
    // program again.
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
    if let Err(err) = private {
        tracing::warn!("the memory of the lock screen could not be made private: {err}");
    }
    tracing::info!(?renderer, "renderer");
    match lock::run(rescue) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the lock screen stopped");
            ExitCode::FAILURE
        }
    }
}
