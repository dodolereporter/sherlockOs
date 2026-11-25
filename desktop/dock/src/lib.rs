//! Dock of the desktop: a separate program, anchored at the bottom of the screen with
//! layer-shell (ADR 0005), written with iced 0.14 and software rendering (ADR 0014), like the
//! bar. See `docs/dock.md`.

pub mod app;
pub mod config;
pub mod entry;
pub mod icons;
pub mod launch;
pub mod layout;
pub mod menu;
pub mod windows;

use std::path::PathBuf;

use iced::{Settings, Size};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize, Margin};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, dock, metrics, theme, typography};

use app::{Dock, Item, Message};
use config::Config;
use entry::Catalog;
use icons::IconLookup;

/// Name of the layer-shell surface, readable by the compositor.
const NAMESPACE: &str = "dock";

/// Surface settings: anchored to the bottom only, so centered, 8 above the screen edge. The
/// surface is taller than the bar, for the magnified icons and the label; the exclusive zone is
/// the bar's height, so maximized windows stop just above it. The keyboard comes on a click.
pub fn layer_settings(size: Size) -> LayerShellSettings {
    LayerShellSettings {
        anchor: Anchor::Bottom,
        layer: Layer::Top,
        size: LayerSize::px(size.width as u32, size.height as u32),
        exclusive_zone: metrics::DOCK_HEIGHT as i32,
        margin: Margin {
            top: 0,
            right: 0,
            bottom: dock::MARGIN_BOTTOM as i32,
            left: 0,
        },
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
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

/// What the dock starts with: its icons, read from the configuration and the system.
pub struct Setup {
    pub items: Vec<Item>,
    pub pinned: Vec<String>,
    pub config: Config,
    pub config_path: Option<PathBuf>,
}

impl Setup {
    /// Reads the configuration, the installed applications and their icons.
    pub fn load() -> Self {
        let data_dirs = entry::data_dirs();
        let catalog = Catalog::load(&data_dirs);
        let config_home = config::config_home();
        let config_path = config::path();
        let config = match config_path.as_deref().map(Config::load) {
            Some(Ok(config)) => config,
            Some(Err(error)) => {
                tracing::warn!(%error, "cannot read the configuration, defaults used");
                Config::default()
            }
            None => Config::default(),
        };
        let pinned = config.pinned.clone().unwrap_or_else(|| {
            let mimeapps = config_home
                .as_deref()
                .map(config::mimeapps)
                .unwrap_or_default();
            config::defaults(&catalog, &mimeapps)
        });
        let theme = config
            .icon_theme
            .clone()
            .or_else(|| config_home.as_deref().and_then(icons::gtk_theme));
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let lookup = IconLookup::new(&data_dirs, home.as_deref(), theme.as_deref());
        tracing::info!(?pinned, themes = ?lookup.themes(), "dock icons");
        let items = app::items(&catalog, &pinned, &lookup);
        Self {
            items,
            pinned,
            config,
            config_path,
        }
    }
}

/// Runs the dock until the session ends.
pub fn run(setup: Setup) -> Result<(), iced_exwlshell::Error> {
    // The dock has no text field: no need to start the clipboard thread.
    iced_exwlshell::disable_clipboard();
    let size = layout::surface_size(setup.items.len());
    let boot = move || {
        let dock = Dock::new(
            setup.items.clone(),
            setup.pinned.clone(),
            setup.config.clone(),
            setup.config_path.clone(),
        );
        (dock, iced::Task::none())
    };
    iced_exwlshell::daemon(boot, namespace, Dock::update, Dock::view)
        .theme(|_: &Dock, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::TEXT_PRIMARY.into(),
        })
        .subscription(Dock::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(settings())
        .wl_settings(ExWlSettings {
            layer_settings: layer_settings(size),
            ..ExWlSettings::default()
        })
        .run()
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
