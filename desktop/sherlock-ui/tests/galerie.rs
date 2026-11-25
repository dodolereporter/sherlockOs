//! Tests de la galerie sans écran : capture du rendu, souris, clavier.

use std::borrow::Cow;
use std::path::PathBuf;

use iced::advanced::clipboard;
use iced::advanced::widget::{Operation, operation};
use iced::keyboard::{Key, key::Named};
use iced::{Element, Pixels, Settings, Size, mouse};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_test::simulator;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::gallery::{Gallery, Message};
use sherlock_ui::{metrics, theme, typography};

fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    }
}

/// Capture la galerie, sous-menu ouvert, dans `target/tmp/galerie-tiny-skia.png`, à l'échelle 2.
/// Elle se compare à l'œil aux planches : voir docs/sherlock-ui.md.
#[test]
fn la_galerie_se_rend_sans_ecran() {
    let mut gallery = Gallery::default();
    let _ = gallery.update(Message::Submenu);
    let (width, _) = metrics::REFERENCE_SCREEN;
    let mut ui = simulator::Simulator::with_size(settings(), (width, 1500.0), gallery.view());
    let snapshot = ui.snapshot(&theme::theme()).expect("capture de la galerie");

    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("galerie.png");
    let written = path.with_file_name("galerie-tiny-skia.png");
    let _ = std::fs::remove_file(&written);
    assert!(
        snapshot
            .matches_image(&path)
            .expect("écriture de la capture")
    );
    let size = std::fs::metadata(&written).expect("capture écrite").len();
    assert!(
        size > 100_000,
        "capture trop petite pour une galerie : {size} octets"
    );
    println!("capture : {}", written.display());
}

#[test]
fn un_clic_declenche_l_action() {
    let gallery = Gallery::default();
    let mut ui = simulator::Simulator::with_size(settings(), (1440.0, 1500.0), gallery.view());
    ui.click("Essayer une nouvelle version")
        .expect("bouton trouvé");
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(messages, [Message::Action("nouvelle version")]);
}

/// Fait passer le focus au composant suivant, comme le fait Tab dans un programme.
fn focus_next<M>(
    ui: &mut UserInterface<'_, M, iced::Theme, iced::Renderer>,
    renderer: &iced::Renderer,
) {
    let mut current: Option<Box<dyn Operation>> =
        Some(Box::new(operation::focusable::focus_next()));
    while let Some(mut op) = current.take() {
        ui.operate(renderer, op.as_mut());
        if let operation::Outcome::Chain(next) = op.finish() {
            current = Some(next);
        }
    }
}

#[test]
fn le_clavier_parcourt_et_active_les_composants() {
    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Premier,
        Second,
        Interrupteur(bool),
    }
    let view: Element<'_, Msg> = iced::widget::column![
        ui::primary_button("Premier", Some(Msg::Premier)),
        ui::secondary_button("Ignoré", None),
        ui::menu_item("Second", Trailing::None, Some(Msg::Second)),
        ui::toggle(false, Some(Msg::Interrupteur)),
    ]
    .into();
    // Le moteur logiciel, seul compilé : il dessine sans écran.
    let mut renderer = iced::Renderer::new(typography::UI, Pixels(typography::UI_SIZE));
    let mut ui = UserInterface::build(
        view,
        Size::new(400.0, 300.0),
        Cache::default(),
        &mut renderer,
    );
    let mut messages = Vec::new();
    let enter: Vec<_> = simulator::tap_key(Key::Named(Named::Enter), None).collect();
    let space: Vec<_> = simulator::tap_key(Key::Named(Named::Space), None).collect();

    // Aucun focus : Entrée ne fait rien.
    let _ = ui.update(
        &enter,
        mouse::Cursor::Unavailable,
        &mut renderer,
        &mut clipboard::Null,
        &mut messages,
    );
    assert!(messages.is_empty());

    // Le bouton désactivé est sauté.
    for keys in [&enter, &enter, &space] {
        focus_next(&mut ui, &renderer);
        let _ = ui.update(
            keys,
            mouse::Cursor::Unavailable,
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
    }
    assert_eq!(
        messages,
        [Msg::Premier, Msg::Second, Msg::Interrupteur(true)]
    );
}
