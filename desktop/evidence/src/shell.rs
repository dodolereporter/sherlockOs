//! Evidence on Wayland (ADR 0029, lot E-02a): an `xdg_toplevel` window made by
//! `iced_exwlshell`, the runtime that will also give its menus real `xdg_popup`s. Under X11,
//! `main` keeps iced's own runtime on winit.

use iced::Settings;
use iced::Task;
use iced_exwlshell::actions::IcedXdgWindowSettings;
use iced_exwlshell::reexport::PixelSize;
use iced_exwlshell::settings::{ExWlSettings, LayerShellSettings, StartMode};
use sherlock_ui::{metrics, theme, typography};

use crate::app::{App, Config};

/// Desktop identifier of the application, as in `evidence.desktop`.
pub const APP_ID: &str = "evidence";

/// Oldest `xdg_wm_base` that `iced_exwlshell` binds: below, it cannot start.
const XDG_WM_BASE: u32 = 2;

/// Whether Evidence can run on `iced_exwlshell` here: a Wayland compositor answers, with an
/// `xdg_wm_base` recent enough. Otherwise (X11, or an old compositor such as WSLg's), iced's
/// own runtime on winit runs it.
pub fn on_wayland() -> bool {
    if std::env::var_os("WAYLAND_DISPLAY").is_none_or(|d| d.is_empty()) {
        return false;
    }
    let Ok(connection) = wayland_client::Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) =
        wayland_client::globals::registry_queue_init::<Globals>(&connection)
    else {
        return false;
    };
    let recent = globals.contents().with_list(|list| {
        list.iter()
            .any(|g| g.interface == "xdg_wm_base" && g.version >= XDG_WM_BASE)
    });
    if !recent {
        tracing::info!("xdg_wm_base too old for iced_exwlshell: Evidence runs on winit");
    }
    recent
}

/// The state of the probe's queue: it only reads the list of globals.
struct Globals;

impl
    wayland_client::Dispatch<
        wayland_client::protocol::wl_registry::WlRegistry,
        wayland_client::globals::GlobalListContents,
    > for Globals
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &wayland_client::globals::GlobalListContents,
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

/// The window: its first size and its app id. It draws its own title bar.
fn window() -> IcedXdgWindowSettings {
    let (width, height) = metrics::REFERENCE_WINDOW;
    IcedXdgWindowSettings {
        // Surface-local (logical) pixels: the runtime scales them to the output.
        size: PixelSize::try_px(width as u32, height as u32),
        client_side_decorations: true,
        app_id: Some(APP_ID.to_owned()),
        ..IcedXdgWindowSettings::default()
    }
}

/// Runs Evidence on Wayland until its window closes.
pub fn run(config: impl Fn() -> Config + 'static) -> Result<(), iced_exwlshell::Error> {
    let boot = move || {
        let (mut app, task) = App::new(config());
        let open = app.open_window(window());
        (app, Task::batch([open, task]))
    };
    iced_exwlshell::daemon(boot, namespace, App::update, App::view_window)
        .title(|app: &App, _| Some(app.title()))
        .theme(|_: &App, _| Some(theme::window_theme()))
        .style(surface_style)
        .subscription(App::subscription)
        .settings(Settings {
            fonts: typography::FONTS.map(std::borrow::Cow::Borrowed).to_vec(),
            default_font: typography::UI,
            default_text_size: typography::UI_SIZE.into(),
            ..Settings::default()
        })
        .wl_settings(ExWlSettings {
            // No layer surface: the window is the program's only surface.
            layer_settings: LayerShellSettings {
                start_mode: StartMode::Background,
                ..LayerShellSettings::default()
            },
            ..ExWlSettings::default()
        })
        .run()
}

/// Every surface starts transparent (lot Q-57): the window paints its own body, and a popup
/// keeps the room of its shadow clear around the menu. The theme's background, iced's default,
/// filled that room in black.
fn surface_style<State>(_: &State, _: &iced::Theme) -> iced::theme::Style {
    iced::theme::Style {
        background_color: sherlock_ui::color::TRANSPARENT.into(),
        text_color: sherlock_ui::color::palette().text_primary.into(),
    }
}

fn namespace() -> String {
    APP_ID.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lot Q-57: the surfaces start transparent, and the window paints its body in the color
    /// iced used to clear it with, so that nothing changes for the window itself.
    #[test]
    fn surfaces_start_transparent_and_the_window_paints_its_body() {
        let shown = theme::window_theme();
        let style = surface_style(&(), &shown);
        assert_eq!(style.background_color.a, 0.0);
        let body = theme::window(&shown).background;
        assert_eq!(
            body,
            Some(iced::Background::Color(shown.palette().background))
        );
    }
}
