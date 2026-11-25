//! Galerie des composants de sherlock-ui : `cargo run -p sherlock-ui --example galerie`.
//! La procédure de comparaison aux planches est dans `docs/sherlock-ui.md`.

mod vue;

use sherlock_ui::{metrics, theme, typography};
use vue::Gallery;

fn main() -> iced::Result {
    let (width, height) = metrics::REFERENCE_SCREEN;
    let app = iced::application(Gallery::new, Gallery::update, Gallery::view)
        .title("Galerie de sherlock-ui")
        .theme(theme::theme())
        .subscription(Gallery::subscription)
        .default_font(typography::UI)
        .window_size((width, height));
    typography::FONTS
        .into_iter()
        .fold(app, |app, font| app.font(font))
        .run()
}
