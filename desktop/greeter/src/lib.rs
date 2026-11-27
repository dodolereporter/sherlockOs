//! The login screen of SherlockOS (lot D-24, ADR 0026), a greeter of greetd: greetd runs PAM and
//! starts the session, this program asks who logs in and relays the conversation. It runs as
//! greetd's user, in the compositor started with `--greeter`. See `docs/greeter.md`.
//!
//! - `greetd`: the client of greetd's IPC;
//! - `login`: a whole login, from the user name to the session started;
//! - `users`, `sessions`, `memory`: who may log in, what can be started, who logged in last;
//! - `app`: the screen.

pub mod app;
pub mod greetd;
pub mod login;
pub mod memory;
pub mod sessions;
pub mod users;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::ShellType;
use lock::app::Mascot;
use sherlock_ui::{color, theme};
use system_services::secret::Secret;

use crate::app::{Greeter, Login, Message, Remember};
use crate::login::Report;
use crate::memory::Memory;
use crate::sessions::Session;

/// greetd, reached once through `GREETD_SOCK` and kept for the next attempts.
#[derive(Default)]
pub struct Greetd(Mutex<Option<greetd::Greetd>>);

impl Login for Greetd {
    fn log_in(&self, user: &str, password: Secret, session: &Session) -> Result<Report, String> {
        let mut held = self.0.lock().map_err(|_| "greetd: poisoned".to_owned())?;
        let greetd = match held.as_mut() {
            Some(greetd) => greetd,
            None => held.insert(greetd::Greetd::connect().map_err(|e| e.to_string())?),
        };
        let report = login::log_in(greetd, user, password, &session.launch());
        // A broken connection is opened again at the next attempt.
        if report.is_err() {
            *held = None;
        }
        report.map_err(|e| e.to_string())
    }
}

/// The memory, written to its file.
pub struct MemoryFile(pub PathBuf);

impl Remember for MemoryFile {
    fn save(&self, memory: &Memory) {
        if let Err(err) = memory::save(&self.0, memory) {
            tracing::warn!(path = %self.0.display(), "cannot remember the last user: {err}");
        }
    }
}

/// The model of Sherlock, read on a thread of its own (the still picture until it is ready).
fn start() -> iced::Task<Message> {
    lock::in_background(
        "mascot",
        || {
            let path = sherlock_3d::locate::find()?;
            sherlock_3d::Model::open(&path)
                .inspect_err(|err| tracing::warn!(path = %path.display(), "mascot: {err}"))
                .ok()
        },
        |model| Some(Message::Mascot(model.map(|m| Mascot(Arc::new(m))))),
    )
}

/// Every screen gets a surface over everything, which takes the keyboard.
pub fn layer_settings() -> LayerShellSettings {
    LayerShellSettings {
        anchor: Anchor::all(),
        layer: Layer::Overlay,
        exclusive_zone: -1,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        start_mode: StartMode::AllScreens,
        ..LayerShellSettings::default()
    }
}

/// Shows the screen until someone logs in: the program ends, and greetd starts the session.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    color::set_active(color::Theme::Dark);
    iced_exwlshell::disable_clipboard();
    let users = users::list(&users::Sources::default());
    let sessions = sessions::list(&sessions::folders());
    let path = memory::path();
    let memory = memory::load(&path);
    tracing::info!(
        users = users.len(),
        sessions = sessions.len(),
        "login screen"
    );
    let make = move || {
        Greeter::new(
            users.clone(),
            sessions.clone(),
            memory.clone(),
            Arc::new(Greetd::default()),
            Arc::new(MemoryFile(path.clone())),
            Arc::new(lock::machine::Systemctl),
        )
    };
    iced_exwlshell::daemon(
        move || (make(), start()),
        namespace,
        Greeter::update,
        Greeter::view,
    )
    .theme(|_: &Greeter, _| Some(theme::theme()))
    // Transparent: the compositor shows the picture of the lock behind from its first frame
    // (lot D-24d), and this program's own fades in over it.
    .style(|_, _| iced::theme::Style {
        background_color: iced::Color::TRANSPARENT,
        text_color: color::palette().text_primary.into(),
    })
    .subscription(Greeter::subscription)
    .on_new_shell(|info| {
        matches!(info.shell, ShellType::LayerShell).then_some(Message::Surface(info.window))
    })
    .settings(lock::settings())
    .wl_settings(ExWlSettings {
        layer_settings: layer_settings(),
        ..ExWlSettings::default()
    })
    .run()
}

fn namespace() -> String {
    String::from("sherlockos-greeter")
}
