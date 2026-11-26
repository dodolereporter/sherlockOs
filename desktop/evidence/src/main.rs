//! Evidence: `cargo run -p evidence [folder]`.

use evidence::app::{App, Config};
use evidence::startup;
use sherlock_ui::{metrics, theme, typography};

/// Desktop identifier of the application.
const APP_ID: &str = "evidence";

fn main() -> iced::Result {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    // The start log counts from here.
    startup::begin();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tracing::info!(?renderer, "renderer");
    startup::step("renderer chosen", std::time::Duration::ZERO);
    // The theme of the file before the first frame; the subscription follows it afterwards.
    startup::timed("theme read", || {
        if let Some(path) = appearance_config::path() {
            sherlock_ui::color::set_active(appearance_config::theme_now(&path));
        }
    });
    let (width, height) = metrics::REFERENCE_WINDOW;
    let app = iced::application(
        || App::new(startup::timed("configuration read", Config::from_env)),
        App::update,
        App::view,
    )
    .title(App::title)
    .theme(|_: &App| theme::window_theme())
    .subscription(App::subscription)
    .default_font(typography::UI)
    .window(iced::window::Settings {
        size: iced::Size::new(width, height),
        // Small enough for half a laptop screen, large enough for the right-click menu
        // and the application chooser to keep their rows whole.
        min_size: Some(iced::Size::from(sherlock_ui::evidence::MIN_WINDOW)),
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
