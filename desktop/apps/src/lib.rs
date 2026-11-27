//! Applications view: every installed application in a grid of pages, after the Launchpad of
//! macOS, to start one or to find it again. A program started on demand, shown over the whole
//! screen with layer-shell (ADR 0005), written with iced 0.14 like the launcher, whose reading
//! of the applications, search and start it reuses. See `docs/apps.md`.

pub mod app;
pub mod catalog;
pub mod icons;
pub mod pager;

use std::path::PathBuf;
use std::sync::Arc;

use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, LayerSize, NewLayerShellSettings,
};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use launcher::apps::Apps;
use launcher::service::Inbox;
use launcher::startup;
use sherlock_ui::apps as look;
use sherlock_ui::{color, theme};

use app::{Message, View};
use catalog::Catalog;

/// Name of the layer-shell surface, readable by the compositor.
pub const NAMESPACE: &str = "sherlockos-apps";

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

/// The surface of a resident view, opened at each showing: the same as `surface`.
pub fn new_surface() -> NewLayerShellSettings {
    let screen = surface();
    NewLayerShellSettings {
        anchor: screen.anchor,
        layer: screen.layer,
        size: screen.size,
        exclusive_zone: Some(screen.exclusive_zone),
        keyboard_interactivity: screen.keyboard_interactivity,
        namespace: Some(String::from(NAMESPACE)),
        ..NewLayerShellSettings::default()
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
    let apps = startup::timed("desktop entries read", || {
        Apps::load(&dirs, &launcher::locales(), terminal)
    })
    .with_icons(icons)
    .with_icon_size(look::ICON_LOOKUP, look::ICON_SCALE);
    let catalog = Catalog::new(apps);
    startup::timed("icons looked up", || catalog.look_up_icons());
    tracing::info!(applications = catalog.len(), ?theme, "applications");
    catalog
}

/// Runs the view: until an application starts or it is closed, or for the whole session when
/// it is resident (`inbox`, lot P-02).
pub fn run(inbox: Option<Inbox>) -> Result<(), iced_exwlshell::Error> {
    startup::timed("theme read", launcher::apply_theme);
    let view = View::new(catalog(), Arc::new(launcher::launch::spawn));
    let layer = match inbox {
        Some(_) => LayerShellSettings {
            start_mode: StartMode::Background,
            ..LayerShellSettings::default()
        },
        None => surface(),
    };
    let reload: app::Reload = Arc::new(catalog);
    // Where the surfaces land: a resident view draws its icons for that screen at its start.
    let (broadcast, outputs) = iced_exwlshell::shell::channel();
    // The daemon wants a boot function it may call again: the view is handed over once.
    let first = std::sync::Mutex::new(Some(view));
    let boot = move || {
        let mut view = first
            .lock()
            .ok()
            .and_then(|mut v| v.take())
            .unwrap_or_else(|| View::new(catalog(), Arc::new(launcher::launch::spawn)));
        let task = match inbox.clone() {
            Some(inbox) => {
                view = view
                    .resident(inbox, reload.clone())
                    .with_outputs(outputs.clone());
                view.warm_up()
            }
            None => iced::Task::none(),
        };
        (view, task)
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
            layer_settings: layer,
            shell_broadcast: broadcast,
            ..ExWlSettings::default()
        })
        .run()
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
