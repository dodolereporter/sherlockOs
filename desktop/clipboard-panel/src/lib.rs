//! Clipboard history panel, Super + V (lot D-42): what was copied, in Wayland and X11
//! applications, from the compositor's history through the shell's clipboard protocol. A
//! program started on demand by the compositor, shown above everything with layer-shell (ADR
//! 0005), written with iced 0.14 like the launcher. See `docs/clipboard-panel.md`.

pub mod app;
pub mod history;
pub mod items;

use iced_exwlshell::reexport::{Anchor, KeyboardInteractivity, Layer, LayerSize};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use iced_exwlshell::shell::{ShellInfo, ShellType};
use sherlock_ui::{clipboard, color, theme};

use app::{Message, Panel};

/// Name of the layer-shell surface, readable by the compositor.
pub const NAMESPACE: &str = "clipboard";

/// The name of the lock file of the single instance, in `$XDG_RUNTIME_DIR`.
pub const LOCK: &str = "clipboard.lock";

/// Height of the surface at the opening: the panel with all its rows. It then follows the rows
/// found and the room of the screen (`Panel::fit`).
pub fn initial_height() -> u32 {
    clipboard::panel_height(clipboard::VISIBLE_ROWS).ceil() as u32
}

/// The surface: the panel alone, in the middle of the screen (anchored nowhere), above the
/// windows and the bar, with the keyboard all to itself.
pub fn surface() -> LayerShellSettings {
    LayerShellSettings {
        anchor: Anchor::empty(),
        layer: Layer::Overlay,
        size: LayerSize::px(clipboard::WIDTH as u32, initial_height()),
        exclusive_zone: -1,
        keyboard_interactivity: KeyboardInteractivity::Exclusive,
        start_mode: StartMode::Active,
        ..LayerShellSettings::default()
    }
}

/// Runs the panel until an item is put back or Escape closes it.
pub fn run() -> Result<(), iced_exwlshell::Error> {
    launcher::apply_theme();
    iced_exwlshell::daemon(boot, namespace, Panel::update, Panel::view)
        .theme(|_: &Panel, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Panel::subscription)
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

fn boot() -> (Panel, iced::Task<Message>) {
    (Panel::default(), iced::Task::none())
}

fn namespace() -> String {
    String::from(NAMESPACE)
}
