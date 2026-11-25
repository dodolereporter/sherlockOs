//! Evidence: `cargo run -p evidence [folder]`.

use evidence::app::{App, Config};
use sherlock_ui::{metrics, theme, typography};

/// Desktop identifier of the application.
const APP_ID: &str = "evidence";

fn main() -> iced::Result {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!(?renderer, "renderer");
    let (width, height) = metrics::REFERENCE_WINDOW;
    let app = iced::application(|| App::new(Config::from_env()), App::update, App::view)
        .title(App::title)
        .theme(theme::window_theme())
        .subscription(App::subscription)
        .default_font(typography::UI)
        .window(iced::window::Settings {
            size: iced::Size::new(width, height),
            // The window draws its own title bar, as it does under the project's compositor.
            decorations: false,
            // Application id (Wayland) and WM_CLASS (X11), as in `evidence.desktop`.
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
