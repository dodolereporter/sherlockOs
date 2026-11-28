//! Dock of the desktop: a separate program, anchored at the bottom of the screen with
//! layer-shell (ADR 0005), written with iced 0.14 and software rendering (ADR 0014), like the
//! bar. See `docs/dock.md`.

pub mod anim;
pub mod app;
pub mod config;
pub mod drag;
pub mod entry;
pub mod hide;
pub mod icons;
pub mod items;
pub mod launch;
pub mod layout;
pub mod menu;
pub mod pins;
pub mod save;
pub mod shell;
pub mod stack;
pub mod trash;
mod view;
pub mod watch;
pub mod windows;

use std::path::PathBuf;

use dock_config::Position;
use iced::{Settings, Size};
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize, Margin};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, theme, typography};

use app::{Dock, Message, Sources};
use entry::Catalog;
use icons::IconLookup;
use layout::Geometry;

/// Name of the layer-shell surface, readable by the compositor.
const NAMESPACE: &str = "dock";

/// Surface settings for a dock on `geometry.position`, shown: anchored to that edge only, so
/// centered along it, against it; the bar is drawn 8 away from the edge, inside the surface
/// (`Geometry::gap`). The surface is deeper than the bar, for the magnified icons and the
/// label; the exclusive zone is the gap and the bar's thickness, so maximized windows stop just
/// past it. The keyboard comes on a click. A hiding dock changes the margin and the zone
/// (`hide`).
pub fn surface(geometry: Geometry, size: Size) -> LayerShellSettings {
    let anchor = match geometry.position {
        Position::Bottom => Anchor::Bottom,
        Position::Top => Anchor::Top,
        Position::Left => Anchor::Left,
        Position::Right => Anchor::Right,
    };
    LayerShellSettings {
        anchor,
        layer: Layer::Top,
        size: LayerSize::px(size.width as u32, size.height as u32),
        exclusive_zone: (geometry.bar_thickness() + geometry.gap).round() as i32,
        margin: margin(geometry.position, 0),
        keyboard_interactivity: KeyboardInteractivity::OnDemand,
        start_mode: StartMode::Active,
        ..LayerShellSettings::default()
    }
}

/// The margin that puts the surface `distance` away from the `position` edge.
pub fn margin(position: Position, distance: i32) -> Margin {
    let mut margin = Margin::default();
    match position {
        Position::Bottom => margin.bottom = distance,
        Position::Top => margin.top = distance,
        Position::Left => margin.left = distance,
        Position::Right => margin.right = distance,
    }
    margin
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

impl Sources {
    /// Reads the configuration, the installed applications and their icon themes.
    pub fn load() -> Self {
        let data_dirs = entry::data_dirs();
        let catalog = Catalog::load(&data_dirs);
        let config_home = dock_config::config_home();
        let config_path = dock_config::path();
        let config = config_path
            .as_deref()
            .and_then(|path| watch::load(path).ok())
            .unwrap_or_default();
        let pinned = config.pinned.clone().unwrap_or_else(|| {
            let mimeapps = config_home
                .as_deref()
                .map(config::mimeapps)
                .unwrap_or_default();
            config::defaults(&catalog, &mimeapps)
        });
        let gtk_theme = config_home.as_deref().and_then(icons::user_theme);
        let theme = config.icon_theme.clone().or_else(|| gtk_theme.clone());
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let lookup = IconLookup::new(&data_dirs, home.as_deref(), theme.as_deref());
        tracing::info!(?pinned, themes = ?lookup.themes(), "dock icons");
        Self {
            catalog,
            lookup,
            config,
            config_path,
            pinned,
            data_dirs,
            home,
            gtk_theme,
            control: std::sync::Arc::new(windows::Auto),
            applications: true,
        }
    }
}

/// Runs the dock until the session ends.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    // The dock has no text field: no need to start the clipboard thread.
    iced_exwlshell::disable_clipboard();
    // Icon files are rasterized by a worker thread, never by the one that draws the dock.
    sherlock_ui::raster::draw_in_background();
    // The theme of the file before the first frame; the subscription follows it afterwards.
    if let Some(path) = appearance_config::path() {
        color::set_active(appearance_config::theme_now(&path));
    }
    // iced_exwlshell reports there the screen of the surface.
    let (broadcast, screens) = iced_exwlshell::shell::channel();
    let dock = Dock::new(Sources::load()).with_screens(screens.clone());
    let layer = dock.layer_settings();
    // The daemon wants a boot function it may call again: the first dock is handed over once.
    let first = std::sync::Mutex::new(Some(dock));
    let boot = move || {
        let dock = first
            .lock()
            .ok()
            .and_then(|mut d| d.take())
            .unwrap_or_else(|| Dock::new(Sources::load()).with_screens(screens.clone()));
        (dock, iced::Task::none())
    };
    iced_exwlshell::daemon(boot, namespace, Dock::update, Dock::view)
        .theme(|_: &Dock, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Dock::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(settings())
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
