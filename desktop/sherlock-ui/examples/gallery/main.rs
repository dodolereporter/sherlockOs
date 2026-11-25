//! Gallery of the sherlock-ui components: `cargo run -p sherlock-ui --example gallery`.
//! The procedure to compare it with the design boards is in `docs/sherlock-ui.md`.

mod view;

use sherlock_ui::{metrics, theme, typography};
use view::Gallery;

fn main() -> iced::Result {
    // Before anything else: the renderer is chosen through the environment (ADR 0018).
    let renderer = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main`, no other thread exists yet.
    unsafe { renderer.apply() };
    let (width, height) = metrics::REFERENCE_SCREEN;
    let app = iced::application(Gallery::new, Gallery::update, Gallery::view)
        .title("Galerie de sherlock-ui")
        .theme(|_: &Gallery| theme::theme())
        .subscription(Gallery::subscription)
        .default_font(typography::UI)
        .window_size((width, height));
    typography::FONTS
        .into_iter()
        .fold(app, |app, font| app.font(font))
        .run()
}
