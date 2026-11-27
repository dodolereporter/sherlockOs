//! Control Room: `cargo run -p control-room`.

use control_room::app::{App, Config};
use sherlock_ui::control_room as cr;
use sherlock_ui::{theme, typography};

/// Desktop identifier of the application, as in `control-room.desktop`.
const APP_ID: &str = "control-room";

fn main() -> iced::Result {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!(?renderer, "renderer");
    let Some(config) = Config::from_env() else {
        tracing::error!("no home folder: the settings files cannot be found");
        return Ok(());
    };
    // The theme of the file before the first frame; the subscription follows it afterwards.
    sherlock_ui::color::set_active(appearance_config::theme_now(&config.paths.appearance));
    let (width, height) = cr::WINDOW;
    let app = iced::application(move || App::new(config.clone()), App::update, App::view)
        .title(App::title)
        .theme(|_: &App| theme::window_theme())
        .subscription(App::subscription)
        .default_font(typography::UI)
        .window(iced::window::Settings {
            size: iced::Size::new(width, height),
            min_size: Some(iced::Size::from(cr::MIN_WINDOW)),
            // The window draws its own title bar, as Evidence does.
            decorations: false,
            // Closing writes the settings still waiting first.
            exit_on_close_request: false,
            // Application id (Wayland) and WM_CLASS (X11), as in `control-room.desktop`.
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: String::from(APP_ID),
                ..Default::default()
            },
            ..Default::default()
        });
    typography::FONTS
        .into_iter()
        .fold(app, |app, font| app.font(font))
        .run()
}
