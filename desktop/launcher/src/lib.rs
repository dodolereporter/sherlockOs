//! Application launcher: a search field that finds the installed applications and starts them
//! from the keyboard. A program started on demand, shown above everything with layer-shell
//! (ADR 0005), written with iced 0.14 like the bar and the dock. See `docs/launcher.md`.

pub mod app;
pub mod apps;
pub mod ask;
pub mod calc;
pub mod files;
pub mod instance;
pub mod launch;
pub mod search;
pub mod service;
pub mod source;
pub mod startup;
pub mod system;

use std::path::PathBuf;
use std::sync::Arc;

use iced::Settings;
use iced::futures::Stream;
use iced::futures::channel::mpsc;
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, LayerSize, Margin, NewLayerShellSettings,
};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, launcher, theme, typography};

use app::{Clipboard, Launcher, Message};
use apps::Apps;
use iced::futures::channel::oneshot;

/// Name of the layer-shell surface, readable by the compositor.
pub const NAMESPACE: &str = "launcher";

/// Surface settings for a panel `height` high: anchored to the top only, so centered, at the
/// board's distance from the screen top whatever the bar reserves (exclusive zone -1), above
/// the windows and the bar, with the keyboard all to itself.
pub fn surface(height: f32) -> LayerShellSettings {
    LayerShellSettings {
        anchor: Anchor::Top,
        layer: Layer::Overlay,
        size: LayerSize::px(launcher::WIDTH as u32, height.ceil() as u32),
        exclusive_zone: -1,
        margin: Margin {
            top: launcher::TOP as i32,
            ..Margin::default()
        },
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        start_mode: StartMode::Active,
        ..LayerShellSettings::default()
    }
}

/// The surface of a resident launcher when the renderer has alpha (lot P-02): the whole screen,
/// so that a click beside the panel closes it and goes no further, as Spotlight does. The
/// panel sits at the same place as on its own surface.
pub fn screen_surface() -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Overlay,
        size: LayerSize::FILL,
        exclusive_zone: Some(-1),
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        namespace: Some(String::from(NAMESPACE)),
        ..NewLayerShellSettings::default()
    }
}

/// The surface of a resident launcher in software, whose surfaces are opaque: the panel alone,
/// as `surface` places it.
pub fn panel_surface(height: f32) -> NewLayerShellSettings {
    let panel = surface(height);
    NewLayerShellSettings {
        anchor: panel.anchor,
        layer: panel.layer,
        size: panel.size,
        exclusive_zone: Some(panel.exclusive_zone),
        margin: Some(panel.margin),
        keyboard_interactivity: panel.keyboard_interactivity,
        namespace: Some(String::from(NAMESPACE)),
        ..NewLayerShellSettings::default()
    }
}

/// The surface drawn once as a service starts, to ready the renderer: one pixel in a corner,
/// under the windows, without input nor keyboard. `namespace`: the program's.
pub fn warm_surface(namespace: &str) -> NewLayerShellSettings {
    NewLayerShellSettings {
        anchor: Anchor::Top | Anchor::Left,
        layer: Layer::Background,
        size: LayerSize::px(1, 1),
        exclusive_zone: Some(0),
        keyboard_interactivity: KeyboardInteractivity::None,
        events_transparent: true,
        namespace: Some(format!("{namespace}-warm-up")),
        ..NewLayerShellSettings::default()
    }
}

/// The theme of the appearance file each time it changes, for a launcher that stays.
pub fn appearance() -> impl Stream<Item = color::Theme> {
    let (sender, receiver) = mpsc::unbounded();
    if let Some(path) = appearance_config::path() {
        let followed = appearance_config::follow(path, move |event| match event {
            appearance_config::Event::Theme(theme) => sender.unbounded_send(theme).is_ok(),
            _ => true,
        });
        if let Err(error) = followed {
            tracing::warn!(%error, "cannot follow the theme");
        }
    }
    receiver
}

/// iced settings: sherlock-ui fonts and interface text size.
pub fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: typography::UI_SIZE.into(),
        ..Settings::default()
    }
}

/// Makes the theme of the appearance file the active one, before the first frame; the
/// subscription follows it then (`appearance`).
pub fn apply_theme() {
    let Some(path) = appearance_config::path() else {
        return;
    };
    match appearance_config::AppearanceConfig::load(&path) {
        Ok((config, _)) => {
            let now = appearance_config::TimeOfDay::at(appearance_config::epoch_now());
            if let Ok(now) = now {
                color::set_active(config.theme_at(now).0);
            }
        }
        Err(error) => tracing::warn!(%error, path = %path.display(), "cannot read the theme"),
    }
}

/// Locales to try for application names, most specific first: `fr_FR`, then `fr`.
pub fn locales() -> Vec<String> {
    let raw = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let base = raw.split(['.', '@']).next().unwrap_or_default();
    match base.split_once('_') {
        _ if base.is_empty() || base == "C" || base == "POSIX" => Vec::new(),
        Some((language, _)) => vec![base.to_owned(), language.to_owned()],
        None => vec![base.to_owned()],
    }
}

/// The current desktop names, lower case, for the desktop-specific `mimeapps.list`.
fn desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .filter(|d| !d.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The sources of results, in the order of their sections, and what tells when the files
/// are walked.
pub fn sources() -> app::Sources {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let dirs = mime_apps::Dirs::from_env(&home);
    let terminal = mime_apps::pick_terminal(std::env::var("TERMINAL").ok().as_deref());
    let data: Vec<PathBuf> = dirs.data().cloned().collect();
    let theme = icon_theme::user_theme(&dirs.config_home);
    let icons =
        icon_theme::Lookup::new(&data, Some(&home), theme.as_deref()).context("Applications");
    let locales = locales();
    let apps = startup::timed("desktop entries read", || {
        Apps::load(&dirs, &locales, terminal.clone())
    })
    .with_icons(icons);
    tracing::info!(applications = apps.len(), ?theme, "launcher sources");
    let (catalog_dirs, catalog_terminal) = (dirs.clone(), terminal.clone());
    let (files, walked) = files::Files::start(home, move || {
        mime_apps::Catalog::load_with_terminal(
            catalog_dirs,
            &desktops(),
            &locales,
            catalog_terminal,
        )
    });
    let ask = ask::Ask::new(terminal, ask::ASK_OPTION);
    (arrange(apps, files, ask), walked)
}

/// The sources in the order of their sections: the calculator, whose single result is the
/// most precise answer, the applications, then the system commands where the board has its
/// settings, the files, and « Demander à Sherlock » last, as on the board. System commands
/// found by a keyword only come after the files.
pub fn arrange(apps: Apps, files: files::Files, ask: ask::Ask) -> Vec<Box<dyn source::Source>> {
    use system::{Placement, System};
    vec![
        Box::new(calc::Calculator),
        Box::new(apps),
        Box::new(System::new(Placement::ByName)),
        Box::new(files),
        Box::new(System::new(Placement::ByKeyword)),
        Box::new(ask),
    ]
}

/// The launcher for this session: its sources, `launch::spawn_with`, systemd, `wl-copy` when
/// installed.
fn launcher() -> (Launcher, oneshot::Receiver<usize>) {
    let (sources, walked) = sources();
    let launcher = Launcher::new(sources, Arc::new(launch::spawn_with))
        .with_session(Arc::new(system::Systemd::from_env()))
        .with_clipboard(Clipboard::detect());
    (launcher, walked)
}

/// How the launcher lives.
#[derive(Debug)]
pub enum Mode {
    /// Started by a key, it ends when it closes.
    OnDemand,
    /// Started with the session, hidden, and shown by the requests of its clients (lot P-02).
    Resident(service::Inbox),
}

/// Runs the launcher: until a program starts or Escape closes it, or for the whole session when
/// it is resident.
pub fn run(mode: Mode) -> Result<(), iced_exwlshell::Error> {
    startup::timed("theme read", apply_theme);
    let resident = match mode {
        Mode::OnDemand => None,
        Mode::Resident(inbox) => Some(inbox),
    };
    let started = launcher();
    let layer = match resident {
        Some(_) => LayerShellSettings {
            start_mode: StartMode::Background,
            ..LayerShellSettings::default()
        },
        None => surface(started.0.height()),
    };
    let reload: app::Reload = Arc::new(sources);
    // The daemon wants a boot function it may call again: the launcher is handed over once.
    let first = std::sync::Mutex::new(Some(started));
    let boot = move || {
        let (mut launcher, walked) = first
            .lock()
            .ok()
            .and_then(|mut l| l.take())
            .unwrap_or_else(launcher);
        let mut tasks = Vec::new();
        if let Some(inbox) = resident.clone() {
            launcher = launcher.resident(inbox, reload.clone());
            tasks.push(launcher.warm_up());
        }
        // Once the files are walked, the query typed meanwhile is searched again.
        tasks.push(iced::Task::perform(walked, |_| Message::Refresh));
        (launcher, iced::Task::batch(tasks))
    };
    iced_exwlshell::daemon(boot, namespace, Launcher::update, Launcher::view)
        .theme(|_: &Launcher, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Launcher::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(settings())
        .wl_settings(ExWlSettings {
            layer_settings: layer,
            ..ExWlSettings::default()
        })
        .run()
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
