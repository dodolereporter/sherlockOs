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
pub mod source;
pub mod system;

use std::path::PathBuf;
use std::sync::Arc;

use iced::Settings;
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize, Margin};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, launcher, theme, typography};

use app::{Clipboard, Launcher, Message};
use apps::Apps;
use iced::futures::channel::oneshot;

/// Name of the layer-shell surface, readable by the compositor.
const NAMESPACE: &str = "launcher";

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

/// iced settings: sherlock-ui fonts and interface text size.
pub fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: typography::UI_SIZE.into(),
        ..Settings::default()
    }
}

/// Makes the theme of the appearance file the active one. The launcher lives a few seconds:
/// it reads the file once and does not follow it.
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
pub fn sources() -> (Vec<Box<dyn source::Source>>, oneshot::Receiver<usize>) {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let dirs = mime_apps::Dirs::from_env(&home);
    let terminal = mime_apps::pick_terminal(std::env::var("TERMINAL").ok().as_deref());
    let data: Vec<PathBuf> = dirs.data().cloned().collect();
    let theme = icon_theme::user_theme(&dirs.config_home);
    let icons =
        icon_theme::Lookup::new(&data, Some(&home), theme.as_deref()).context("Applications");
    let locales = locales();
    let apps = Apps::load(&dirs, &locales, terminal.clone()).with_icons(icons);
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
/// settings, the files, and « Demander à Sherlock » last, as on the board.
pub fn arrange(apps: Apps, files: files::Files, ask: ask::Ask) -> Vec<Box<dyn source::Source>> {
    vec![
        Box::new(calc::Calculator),
        Box::new(apps),
        Box::new(system::System::default()),
        Box::new(files),
        Box::new(ask),
    ]
}

/// The launcher for this session: its sources, `launch::spawn`, systemd, `wl-copy` when
/// installed.
fn launcher() -> (Launcher, oneshot::Receiver<usize>) {
    let (sources, walked) = sources();
    let launcher = Launcher::new(sources, Arc::new(launch::spawn))
        .with_session(Arc::new(system::Systemd::from_env()))
        .with_clipboard(Clipboard::detect());
    (launcher, walked)
}

/// Runs the launcher until a program starts or Escape closes it.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    apply_theme();
    let started = launcher();
    let layer = surface(started.0.height());
    // The daemon wants a boot function it may call again: the launcher is handed over once.
    let first = std::sync::Mutex::new(Some(started));
    let boot = move || {
        let (launcher, walked) = first
            .lock()
            .ok()
            .and_then(|mut l| l.take())
            .unwrap_or_else(launcher);
        // Once the files are walked, the query typed meanwhile is searched again.
        (launcher, iced::Task::perform(walked, |_| Message::Refresh))
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
