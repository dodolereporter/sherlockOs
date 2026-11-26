//! The lock screen of SherlockOS (lot D-25a, ADR 0023): `sherlockos-lock`, started by the
//! compositor when the session locks. It takes the lock through `ext-session-lock-v1`, shows one
//! surface per screen, and ends the lock only once PAM accepted the password of the session's
//! user. The compositor keeps the session locked whatever happens to this program. See
//! `docs/lock.md`.

pub mod app;
pub mod machine;
pub mod pam;

use std::sync::{Arc, Mutex};

use iced::Settings;
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::ShellType;
use sherlock_ui::{color, theme, typography};

use crate::app::{Lock, Message};

/// The fonts of the design system, Inter by default.
pub fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: typography::UI_SIZE.into(),
        ..Settings::default()
    }
}

/// What keeps the lock screen from running.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no user for this session in the password database")]
    NoUser,
    #[error(transparent)]
    Shell(#[from] iced_exwlshell::Error),
}

/// Takes the lock and shows the screen until the password is accepted.
pub fn run() -> Result<(), Error> {
    let user = pam::session_user().ok_or(Error::NoUser)?;
    // No clipboard: no second Wayland connection either. The compositor hands this program a
    // single connection (`WAYLAND_SOCKET`), the only one allowed to lock.
    iced_exwlshell::disable_clipboard();
    let first = Mutex::new(Some(Lock::new(
        user.clone(),
        Arc::new(pam::Pam),
        Arc::new(machine::Systemctl),
    )));
    // The daemon wants a boot function it may call again: the state is handed over once. The
    // lock is asked for at once.
    let boot = move || {
        let state = first.lock().ok().and_then(|mut s| s.take());
        let state = state.unwrap_or_else(|| {
            Lock::new(
                user.clone(),
                Arc::new(pam::Pam),
                Arc::new(machine::Systemctl),
            )
        });
        (state, iced::Task::done(Message::Lock))
    };
    iced_exwlshell::daemon(boot, namespace, Lock::update, Lock::view)
        .theme(|_: &Lock, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::palette().system_screen.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Lock::subscription)
        .on_new_shell(|info| {
            matches!(info.shell, ShellType::SessionLock).then_some(Message::Surface(info.window))
        })
        .settings(settings())
        .wl_settings(ExWlSettings {
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..LayerShellSettings::default()
            },
            ..ExWlSettings::default()
        })
        .run()?;
    Ok(())
}

fn namespace() -> String {
    String::from("sherlockos-lock")
}
