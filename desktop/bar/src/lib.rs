//! Top bar of the desktop: a separate program, anchored at the top of the screen with
//! layer-shell (ADR 0005), written with iced 0.14 and software rendering (ADR 0014). See
//! `docs/bar.md`.

pub mod app;
pub mod bus;
pub mod calendar;
pub mod dbusmenu;
pub mod global_menu;
pub mod history;
pub mod live;
pub mod menu;
pub mod names;
pub mod panel;
pub mod quick;
pub mod registrar;
pub mod shape;
pub mod source;
pub mod system;
pub mod unfold;
pub mod user;

use iced::Settings;
use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize, Margin};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{color, metrics, theme, typography};

use app::{Bar, Message};

/// Name of the layer-shell surface, readable by the compositor.
const NAMESPACE: &str = "bar";

/// Surface settings: anchored to the top and both sides, 36 high, margins of the design
/// board, exclusive zone of its height. The keyboard only comes when clicking into the bar.
/// The bar starts floating; `shape` moves its margins.
pub fn layer_settings() -> LayerShellSettings {
    let side = metrics::TOP_BAR_MARGIN_SIDE as i32;
    LayerShellSettings {
        anchor: Anchor::Top | Anchor::Left | Anchor::Right,
        layer: Layer::Top,
        size: LayerSize::fill_width(metrics::TOP_BAR_HEIGHT as u32),
        exclusive_zone: shape::exclusive_zone(),
        margin: Margin {
            top: metrics::TOP_BAR_MARGIN_TOP as i32,
            right: side,
            bottom: 0,
            left: side,
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

/// Runs the bar until the session ends.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    // The bar has no text field: no need to start the clipboard thread.
    iced_exwlshell::disable_clipboard();
    // The theme of the file before the first frame; the subscription follows it afterwards.
    if let Some(path) = appearance_config::path() {
        color::set_active(appearance_config::theme_now(&path));
    }
    // On our compositor, the bar answers the global menu registrations (lot D-36).
    source::shell::on_app_menus(start_registrar);
    // iced_exwlshell reports there the screen each surface is on.
    let (broadcast, shell_events) = iced_exwlshell::shell::channel();
    let boot = move || Bar::new(shell_events.clone());
    let result = iced_exwlshell::daemon(boot, namespace, Bar::update, Bar::view)
        .theme(|_: &Bar, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Bar::subscription)
        .on_new_shell(|info: ShellInfo| {
            (info.shell == ShellType::LayerShell).then_some(Message::Surface(info.window))
        })
        .settings(settings())
        .wl_settings(ExWlSettings {
            layer_settings: layer_settings(),
            shell_broadcast: broadcast,
            ..ExWlSettings::default()
        })
        .run();
    // The monitoring commands would also die with the process (PR_SET_PDEATHSIG); stopping
    // them here makes a normal exit or a dispatch error clean right away.
    source::shutdown();
    result
}

/// Serves `com.canonical.AppMenu.Registrar` on the session bus, the registrations going to the
/// compositor. Without a session bus, or with another registrar on it, nothing is served.
fn start_registrar() {
    let sink: registrar::Sink = std::sync::Arc::new(source::shell::app_menu);
    let served = zbus::block_on(async {
        let bus = bus::session().await?;
        registrar::serve(&bus, sink).await
    });
    if let Err(error) = served {
        tracing::warn!(%error, "no global menu registrar");
    }
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
