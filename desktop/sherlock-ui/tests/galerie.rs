//! Tests de la galerie sans écran : capture du rendu, souris, clavier.

#[path = "../examples/galerie/vue.rs"]
mod vue;

use std::borrow::Cow;
use std::path::PathBuf;

use iced::advanced::clipboard;
use iced::advanced::widget::{Operation, operation};
use iced::keyboard::{self, Key, key::Named};
use iced::{Element, Event, Pixels, Settings, Size, mouse};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_test::simulator;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::{metrics, theme, typography};
use vue::{Gallery, Message};

/// Hauteur qui montre toute la galerie d'un coup, logos compris.
const FULL_HEIGHT: f32 = 2400.0;

fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    }
}

/// Capture toute la galerie, sous-menu ouvert, dans `target/tmp/galerie-tiny-skia.png`, à
/// l'échelle 2. Elle se compare à l'œil aux planches : voir docs/sherlock-ui.md.
#[test]
fn la_galerie_se_rend_sans_ecran() {
    let mut gallery = Gallery::new();
    let _ = gallery.update(Message::Submenu);
    // La galerie écoute Tab : l'abonnement se construit sans fenêtre.
    let _ = gallery.subscription();
    let (width, _) = metrics::REFERENCE_SCREEN;
    let mut ui = simulator::Simulator::with_size(settings(), (width, FULL_HEIGHT), gallery.view());
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
    let gallery = Gallery::new();
    let (width, _) = metrics::REFERENCE_SCREEN;
    let mut ui = simulator::Simulator::with_size(settings(), (width, FULL_HEIGHT), gallery.view());
    ui.click("Essayer une nouvelle version")
        .expect("bouton trouvé");
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(messages, [Message::Action("nouvelle version")]);
}

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Premier,
    Second,
    Interrupteur(bool),
}

type Ui<'a> = UserInterface<'a, Msg, iced::Theme, iced::Renderer>;

/// Trois composants actifs, un désactivé entre eux. `premier` dit si le premier est actif.
fn view<'a>(premier: bool) -> Element<'a, Msg> {
    iced::widget::column![
        ui::primary_button("Premier", premier.then_some(Msg::Premier)),
        ui::secondary_button("Ignoré", None),
        ui::menu_item("Second", Trailing::None, Some(Msg::Second)),
        ui::toggle(false, Some(Msg::Interrupteur)),
    ]
    .into()
}

fn build<'a>(view: Element<'a, Msg>, cache: Cache, renderer: &mut iced::Renderer) -> Ui<'a> {
    UserInterface::build(view, Size::new(400.0, 300.0), cache, renderer)
}

/// Fait passer le focus au composant suivant, comme le fait Tab dans un programme.
fn focus_next(ui: &mut Ui<'_>, renderer: &iced::Renderer) {
    let mut current: Option<Box<dyn Operation>> =
        Some(Box::new(operation::focusable::focus_next()));
    while let Some(mut op) = current.take() {
        ui.operate(renderer, op.as_mut());
        if let operation::Outcome::Chain(next) = op.finish() {
            current = Some(next);
        }
    }
}

fn press(key: Named, repeat: bool) -> Vec<Event> {
    simulator::tap_key(Key::Named(key), None)
        .map(|event| match event {
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                location,
                modifiers,
                text,
                ..
            }) => Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                location,
                modifiers,
                text,
                repeat,
            }),
            other => other,
        })
        .collect()
}

fn send(ui: &mut Ui<'_>, events: &[Event], renderer: &mut iced::Renderer, out: &mut Vec<Msg>) {
    let _ = ui.update(
        events,
        mouse::Cursor::Unavailable,
        renderer,
        &mut clipboard::Null,
        out,
    );
}

#[test]
fn le_clavier_parcourt_et_active_les_composants() {
    // Le moteur logiciel, seul compilé : il dessine sans écran.
    let mut renderer = iced::Renderer::new(typography::UI, Pixels(typography::UI_SIZE));
    let mut ui = build(view(true), Cache::default(), &mut renderer);
    let mut messages = Vec::new();
    let enter = press(Named::Enter, false);

    // Aucun focus : Entrée ne fait rien.
    send(&mut ui, &enter, &mut renderer, &mut messages);
    assert!(messages.is_empty());

    // Entrée gardée enfoncée n'active qu'une fois.
    focus_next(&mut ui, &renderer);
    send(&mut ui, &enter, &mut renderer, &mut messages);
    send(
        &mut ui,
        &press(Named::Enter, true),
        &mut renderer,
        &mut messages,
    );
    // Le bouton désactivé est sauté.
    focus_next(&mut ui, &renderer);
    send(&mut ui, &enter, &mut renderer, &mut messages);
    focus_next(&mut ui, &renderer);
    send(
        &mut ui,
        &press(Named::Space, false),
        &mut renderer,
        &mut messages,
    );
    assert_eq!(
        messages,
        [Msg::Premier, Msg::Second, Msg::Interrupteur(true)]
    );
}

#[test]
fn un_composant_desactive_perd_le_focus() {
    let mut renderer = iced::Renderer::new(typography::UI, Pixels(typography::UI_SIZE));
    let mut messages = Vec::new();
    let enter = press(Named::Enter, false);

    let mut ui = build(view(true), Cache::default(), &mut renderer);
    focus_next(&mut ui, &renderer);
    // Le premier bouton a le focus, puis devient désactivé, puis redevient actif.
    let ui = build(view(false), ui.into_cache(), &mut renderer);
    let mut ui = build(view(true), ui.into_cache(), &mut renderer);
    send(&mut ui, &enter, &mut renderer, &mut messages);
    assert!(
        messages.is_empty(),
        "le focus aurait dû partir : {messages:?}"
    );
}
