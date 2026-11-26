//! Applications view: every installed application in a grid of pages, after the Launchpad of
//! macOS, to start one or to find it again. A program started on demand, shown over the whole
//! screen with layer-shell (ADR 0005), written with iced 0.14 like the launcher, whose reading
//! of the applications, search and start it reuses. See `docs/apps.md`.

pub mod app;
pub mod catalog;
pub mod pager;

use std::path::PathBuf;
use std::sync::Arc;

use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use launcher::apps::Apps;
use sherlock_ui::apps as look;
use sherlock_ui::{color, theme};

use app::{Message, View};
use catalog::Catalog;

/// Name of the layer-shell surface, readable by the compositor.
const NAMESPACE: &str = "sherlockos-apps";

/// Name of the lock file that keeps one view at a time (`launcher::instance`).
pub const LOCK: &str = "sherlockos-apps.lock";

/// Surface settings: the whole screen, over the windows, the bar and the dock, whatever they
/// reserve (exclusive zone -1), with the keyboard all to itself.
pub fn surface() -> LayerShellSettings {
    LayerShellSettings {
        anchor: Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right,
        layer: Layer::Overlay,
        size: LayerSize::FILL,
        exclusive_zone: -1,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        start_mode: StartMode::Active,
        ..LayerShellSettings::default()
    }
}

/// The installed applications of this session, as the launcher reads them, with icons looked
/// up for the grid's size.
pub fn catalog() -> Catalog {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let dirs = mime_apps::Dirs::from_env(&home);
    let terminal = mime_apps::pick_terminal(std::env::var("TERMINAL").ok().as_deref());
    let data: Vec<PathBuf> = dirs.data().cloned().collect();
    let theme = icon_theme::user_theme(&dirs.config_home);
    let icons =
        icon_theme::Lookup::new(&data, Some(&home), theme.as_deref()).context("Applications");
    let apps = Apps::load(&dirs, &launcher::locales(), terminal)
        .with_icons(icons)
        .with_icon_size(look::ICON_LOOKUP, look::ICON_SCALE);
    let catalog = Catalog::new(apps);
    tracing::info!(applications = catalog.len(), ?theme, "applications");
    catalog
}

/// Runs the view until an application starts or it is closed.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    launcher::apply_theme();
    let view = View::new(catalog(), Arc::new(launcher::launch::spawn));
    // The daemon wants a boot function it may call again: the view is handed over once.
    let first = std::sync::Mutex::new(Some(view));
    let boot = move || {
        let view = first
            .lock()
            .ok()
            .and_then(|mut v| v.take())
            .unwrap_or_else(|| View::new(catalog(), Arc::new(launcher::launch::spawn)));
        (view, iced::Task::none())
    };
    iced_exwlshell::daemon(boot, namespace, View::update, View::view)
        .theme(|_: &View, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(View::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(launcher::settings())
        .wl_settings(ExWlSettings {
            layer_settings: surface(),
            ..ExWlSettings::default()
        })
        .run()
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
