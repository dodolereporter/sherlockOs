//! The picture behind the lock screen: read by `desktop_config::lock_background`, shared with
//! the login screen and with the compositor, which draws the same picture behind them as its
//! curtain (lot D-24e).

use std::ffi::OsString;

pub use desktop_config::lock_background::*;

/// `source` then `pictures`, logged: what the lock screen shows behind it.
pub fn load(var: impl Fn(&str) -> Option<OsString>, screen: (u32, u32)) -> Option<Pictures> {
    let source = source(var)?;
    pictures(&source, screen)
        .inspect_err(|err| tracing::warn!(?source, "lock background: {err}"))
        .ok()
}
