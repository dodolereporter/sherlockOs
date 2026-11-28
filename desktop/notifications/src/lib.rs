//! Notification daemon of the desktop: the `org.freedesktop.Notifications` server on the
//! session bus, and its bubbles, shown top right below the bar with layer-shell (ADR 0005),
//! written with iced 0.14 like the bar and the dock. See `docs/notifications.md`.

pub mod app;
pub mod core;
pub mod history;
pub mod layout;
pub mod markup;
pub mod model;
pub mod picture;
pub mod server;

use std::sync::Mutex;

use iced::Settings;
use iced_exwlshell::reexport::{
    Anchor, KeyboardInteractivity, Layer, LayerSize, Margin, NewLayerShellSettings,
};
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use sherlock_ui::notifications as ui;
use sherlock_ui::{color, theme, typography};

use app::{Inbox, Notifications};

/// Name of the layer-shell surface, readable by the compositor.
pub const NAMESPACE: &str = "notifications";

/// How the daemon ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The session ended.
    Stopped,
    /// Another daemon owns the name: this one never started.
    Taken,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("session bus: {0}")]
    Bus(#[from] zbus::Error),
    #[error("layer-shell: {0}")]
    Shell(#[from] iced_exwlshell::Error),
}

/// The surface of the stack, `height` high: anchored top right, `ui::BELOW_BAR` under the area
/// the bar reserves (an exclusive zone of 0 keeps it out of other surfaces' zones), up to the
/// screen edge so that a bubble slides out there. Above the windows; never the keyboard.
pub fn surface(height: u32) -> NewLayerShellSettings {
    NewLayerShellSettings {
        size: LayerSize::px(layout::SURFACE_WIDTH as u32, height.max(1)),
        layer: Layer::Top,
        anchor: Anchor::Top | Anchor::Right,
        exclusive_zone: Some(0),
        margin: Some(Margin {
            // The room of « × » above the first bubble comes out of the gap under the bar.
            top: (ui::BELOW_BAR - ui::CLOSE_OUTSET) as i32,
            ..Margin::default()
        }),
        keyboard_interactivity: KeyboardInteractivity::None,
        namespace: Some(String::from(NAMESPACE)),
        ..NewLayerShellSettings::default()
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

/// Takes the name on the session bus, then shows the bubbles until the session ends.
pub fn run() -> Result<Outcome, Error> {
    let (sink, inbox) = Inbox::new();
    let dnd = notifications_config::path().map(notifications_config::Follower::new);
    let core = core::Core::new(sink, dnd);
    let builder = zbus::blocking::connection::Builder::session()?;
    // Kept open for as long as the daemon runs.
    let Some(_bus) = server::start(builder, &core)? else {
        return Ok(Outcome::Taken);
    };
    tracing::info!(name = server::NAME, "serving");
    // No text field: no need to start the clipboard thread.
    iced_exwlshell::disable_clipboard();
    // iced_exwlshell reports there the screen the stack is on.
    let (broadcast, shell_events) = iced_exwlshell::shell::channel();
    let fresh = move || {
        Notifications::new(core.clone(), inbox.clone()).with_shell_events(shell_events.clone())
    };
    // The daemon wants a boot function it may call again: the state is handed over once.
    let first = Mutex::new(Some(fresh()));
    let boot = move || {
        let state = first.lock().ok().and_then(|mut s| s.take());
        (state.unwrap_or_else(&fresh), iced::Task::none())
    };
    iced_exwlshell::daemon(boot, namespace, Notifications::update, Notifications::view)
        .theme(|_: &Notifications, _| Some(theme::theme()))
        .style(|_, _| iced::theme::Style {
            background_color: color::TRANSPARENT.into(),
            text_color: color::palette().text_primary.into(),
        })
        .subscription(Notifications::subscription)
        .settings(settings())
        .wl_settings(ExWlSettings {
            // On the shell socket, the only one that offers anchored surfaces (lot D-46).
            with_connection: Some(shell_protocol::connect.into()),
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..LayerShellSettings::default()
            },
            shell_broadcast: broadcast,
            ..ExWlSettings::default()
        })
        .run()?;
    Ok(Outcome::Stopped)
}

fn namespace() -> String {
    String::from(NAMESPACE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sherlock_ui::metrics;

    #[test]
    fn the_stack_sits_below_the_bar_against_the_right_edge() {
        let settings = surface(200);
        assert_eq!(settings.anchor, Anchor::Top | Anchor::Right);
        // 0, not -1: the compositor places it below the zone the bar reserves.
        assert_eq!(settings.exclusive_zone, Some(0));
        let margin = settings.margin.unwrap_or_default();
        assert_eq!(
            (margin.top, margin.right),
            ((ui::BELOW_BAR - ui::CLOSE_OUTSET) as i32, 0)
        );
        assert_eq!(settings.keyboard_interactivity, KeyboardInteractivity::None);
        assert_eq!(settings.size, LayerSize::px(340, 200));
        // On the reference screen: the bar reserves its margin and height, the first bubble
        // starts where the board puts the notification centre, 12 from the right edge.
        let reserved = metrics::TOP_BAR_MARGIN_TOP + metrics::TOP_BAR_HEIGHT;
        let first = reserved + margin.top as f32 + ui::CLOSE_OUTSET + layout::stack(&[90.0])[0];
        assert_eq!(first, 54.0);
        let right = metrics::REFERENCE_SCREEN.0 - margin.right as f32 - layout::SURFACE_WIDTH
            + ui::CLOSE_OUTSET;
        assert_eq!(
            right + ui::WIDTH,
            metrics::REFERENCE_SCREEN.0 - ui::SCREEN_EDGE
        );
        assert_eq!(surface(0).size, LayerSize::px(340, 1));
    }
}
