//! Application launcher: a search field that finds the installed applications and starts them
//! from the keyboard. A program started on demand, shown above everything with layer-shell
//! (ADR 0005), written with iced 0.14 like the bar and the dock. See `docs/launcher.md`.

pub mod app;
pub mod apps;
pub mod instance;
pub mod launch;
pub mod search;
pub mod source;

use std::path::PathBuf;
use std::sync::Arc;

use iced::Settings;
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize, Margin};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, launcher, theme, typography};

use app::{Launcher, Message};
use apps::Apps;

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

/// The sources of results, in the order of their sections.
pub fn sources() -> Vec<Box<dyn source::Source>> {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let dirs = mime_apps::Dirs::from_env(&home);
    let terminal = mime_apps::pick_terminal(std::env::var("TERMINAL").ok().as_deref());
    let apps = Apps::load(&dirs, &locales(), terminal);
    tracing::info!(applications = apps.len(), "launcher sources");
    vec![Box::new(apps)]
}

/// Runs the launcher until a program starts or Escape closes it.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    apply_theme();
    let launcher = Launcher::new(sources(), Arc::new(launch::spawn));
    let layer = surface(launcher.height());
    // The daemon wants a boot function it may call again: the launcher is handed over once.
    let first = std::sync::Mutex::new(Some(launcher));
    let boot = move || {
        let launcher = first
            .lock()
            .ok()
            .and_then(|mut l| l.take())
            .unwrap_or_else(|| Launcher::new(sources(), Arc::new(launch::spawn)));
        (launcher, iced::Task::none())
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
