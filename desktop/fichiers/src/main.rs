//! Fichiers : `cargo run -p fichiers [dossier]`.

use fichiers::app::{App, Config};
use sherlock_ui::{metrics, theme, typography};

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let (width, height) = metrics::REFERENCE_WINDOW;
    // La fenêtre dessine sa barre de titre, comme sous le compositeur du projet.
    let app = iced::application(|| App::new(Config::from_env()), App::update, App::view)
        .title(App::title)
        .theme(theme::window_theme())
        .subscription(App::subscription)
        .default_font(typography::UI)
        .decorations(false)
        .window_size((width, height));
    typography::FONTS
        .into_iter()
        .fold(app, |app, font| app.font(font))
        .run()
}
