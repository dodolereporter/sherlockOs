//! The lock screen of SherlockOS (lot D-25a, ADR 0023): `sherlockos-lock`, started by the
//! compositor when the session locks. It takes the lock through `ext-session-lock-v1`, shows one
//! surface per screen, and ends the lock only once PAM accepted the password of the session's
//! user. The compositor keeps the session locked whatever happens to this program. See
//! `docs/lock.md`.

pub mod app;
pub mod background;
pub mod form;
pub mod machine;
pub mod pam;

use std::sync::{Arc, Mutex};

use iced::Settings;
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::ShellType;
use sherlock_ui::{color, theme, typography};

use crate::app::{Lock, Mascot, Message};

/// Runs `work` on a thread of its own once the runtime runs the task (never in the tests, which
/// only call `update`), and hands its result to the program as a message.
pub(crate) fn in_background<T: Send + 'static>(
    name: &'static str,
    work: impl FnOnce() -> Option<T> + Send + 'static,
    message: impl Fn(Option<T>) -> Option<Message> + Send + 'static,
) -> iced::Task<Message> {
    let answer = async move {
        let (sender, receiver) = iced::futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _ = sender.send(work());
            });
        if let Err(err) = spawned {
            tracing::warn!("{name}: {err}");
            return None;
        }
        receiver.await.ok().flatten()
    };
    iced::Task::perform(answer, message).and_then(iced::Task::done)
}

/// What starts with the screen: the lock and the model of Sherlock, read on a thread of its own
/// (the still picture until it is ready). The background is read once the size of the screens
/// is known (`app`).
fn start(rescue: bool) -> iced::Task<Message> {
    if rescue {
        return iced::Task::done(Message::Lock);
    }
    let mascot = in_background(
        "mascot",
        || {
            let path = sherlock_3d::locate::find()?;
            sherlock_3d::Model::open(&path)
                .inspect_err(|err| tracing::warn!(path = %path.display(), "mascot: {err}"))
                .ok()
        },
        |model| Some(Message::Mascot(model.map(|m| Mascot(Arc::new(m))))),
    );
    iced::Task::batch([iced::Task::done(Message::Lock), mascot])
}

/// The fonts of the design system, Inter by default.
pub fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: typography::UI_SIZE.into(),
        ..Settings::default()
    }
}

/// The program was started with `--rescue` (its rescue mode, `app::Lock::rescue`).
pub fn rescue_asked() -> bool {
    std::env::args_os().skip(1).any(|arg| arg == "--rescue")
}

/// What keeps the lock screen from running.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no user for this session in the password database")]
    NoUser,
    #[error(transparent)]
    Shell(#[from] iced_exwlshell::Error),
}

/// Takes the lock and shows the screen until the password is accepted; `rescue`: the rescue
/// mode (`app::Lock::rescue`).
pub fn run(rescue: bool) -> Result<(), Error> {
    let user = pam::session_user().ok_or(Error::NoUser)?;
    // The theme of the session, light or dark, from its first frame.
    if let Some(path) = appearance_config::path() {
        color::set_active(appearance_config::theme_now(&path));
    }
    // No clipboard: no second Wayland connection either. The compositor hands this program a
    // single connection (`WAYLAND_SOCKET`), the only one allowed to lock.
    iced_exwlshell::disable_clipboard();
    let make = move |user: String| {
        let lock = Lock::new(user, Arc::new(pam::Pam), Arc::new(machine::Systemctl));
        if rescue { lock.rescue() } else { lock }
    };
    let first = Mutex::new(Some(make(user.clone())));
    // The daemon wants a boot function it may call again: the state is handed over once. The
    // lock is asked for at once.
    let boot = move || {
        let state = first.lock().ok().and_then(|mut s| s.take());
        let state = state.unwrap_or_else(|| make(user.clone()));
        (state, start(rescue))
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
