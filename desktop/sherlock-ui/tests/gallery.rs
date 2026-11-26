//! Headless tests of the gallery: render snapshot, mouse, keyboard.

#[path = "../examples/gallery/view.rs"]
mod view;

use std::borrow::Cow;
use std::path::PathBuf;

use iced::advanced::clipboard;
use iced::advanced::widget::{Operation, operation};
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{button, column, row};
use iced::{Element, Event, Pixels, Settings, Size, mouse};
use iced_runtime::user_interface::{Cache, UserInterface};
use iced_test::simulator;
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::{color, metrics, theme, typography};
use view::{Gallery, Message};

/// Height that shows the whole gallery at once, logos included.
const FULL_HEIGHT: f32 = 2900.0;

fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    }
}

/// Captures the whole gallery, submenu open, in each theme: into
/// `target/tmp/gallery-tiny-skia.png` and `gallery-light-tiny-skia.png`, at scale 2. They are
/// compared by eye with the design boards: see docs/sherlock-ui.md.
#[test]
fn the_gallery_renders_headless_in_both_themes() {
    for (which, name) in [
        (color::Theme::Dark, "gallery"),
        (color::Theme::Light, "gallery-light"),
    ] {
        color::set_active(which);
        let mut gallery = Gallery::new();
        let _ = gallery.update(Message::Submenu);
        // The gallery listens to Tab and to the appearance file: the subscription builds
        // without a window.
        let _ = gallery.subscription();
        let (width, _) = metrics::REFERENCE_SCREEN;
        let mut ui =
            simulator::Simulator::with_size(settings(), (width, FULL_HEIGHT), gallery.view());
        let snapshot = ui
            .snapshot(&theme::theme())
            .expect("snapshot of the gallery");

        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
        let written = path.with_file_name(format!("{name}-tiny-skia.png"));
        let _ = std::fs::remove_file(&written);
        assert!(snapshot.matches_image(&path).expect("writing the snapshot"));
        let size = std::fs::metadata(&written).expect("snapshot written").len();
        assert!(
            size > 100_000,
            "snapshot too small for a gallery: {size} bytes"
        );
        println!("snapshot: {}", written.display());
    }
    color::set_active(color::Theme::Dark);
}

/// Each component of the design system on its own, in its states, in each theme (UI-01): into
/// `target/tmp/components/<name>-tiny-skia.png` and `<name>-light-tiny-skia.png`, at scale 2.
/// The documentation keeps a copy in `docs/sherlock-ui/components/`.
#[test]
fn each_component_renders_in_both_themes() {
    type Sample = fn() -> iced::Element<'static, Message>;
    let samples: [(&str, Sample); 14] = [
        ("buttons", || {
            let act = Some(Message::Action("bouton"));
            row![
                ui::primary_button("Relancer", act.clone()),
                ui::secondary_button("Voir les logs", act.clone()),
                ui::danger_button("Vider", act),
                ui::primary_button("Désactivé", None),
            ]
            .spacing(metrics::GAP_INLINE)
            .into()
        }),
        ("toggle", || {
            row![
                ui::toggle(false, Some(Message::Quiet)),
                ui::toggle(true, Some(Message::Quiet)),
                ui::toggle(true, None),
                ui::toggle(true, Some(Message::Quiet)).status(button::Status::Pressed),
            ]
            .spacing(metrics::GAP_INLINE)
            .into()
        }),
        ("checkbox", || {
            row![
                ui::checkbox("Non cochée", false, Some(Message::Quiet)),
                ui::checkbox("Cochée", true, Some(Message::Quiet)),
                ui::checkbox("Désactivée", true, None),
            ]
            .spacing(metrics::GAP_INLINE)
            .into()
        }),
        ("radio", || {
            let pick = Some(Message::Action("radio"));
            row![
                ui::radio("Automatique", true, pick.clone()),
                ui::radio("Toujours", false, pick),
                ui::radio("Jamais", false, None),
            ]
            .spacing(metrics::GAP_INLINE)
            .into()
        }),
        ("slider", || {
            column![ui::slider(62, Some(Message::Volume)), ui::slider(30, None)]
                .spacing(metrics::GAP_INLINE)
                .into()
        }),
        ("segmented", || {
            let pick = Some(Message::Segment(0));
            ui::segmented(vec![
                ("Économie", false, pick.clone()),
                ("Équilibré", true, pick),
                ("Performance", false, None),
            ])
        }),
        ("progress", || {
            column![
                ui::progress_bar(0.0),
                ui::progress_bar(0.38),
                ui::progress_bar(1.0)
            ]
            .spacing(metrics::GAP_INLINE)
            .into()
        }),
        ("spinner", || {
            row![
                ui::spinner(metrics::SPINNER),
                ui::description("Recherche des réseaux…")
            ]
            .spacing(metrics::GAP_ICON)
            .align_y(iced::Alignment::Center)
            .into()
        }),
        ("text-field", || {
            column![
                ui::text_field("Rechercher un réglage", "", Some(Message::Search)),
                ui::text_field("Mot de passe", "Wi-Fi", Some(Message::Search)),
                ui::text_field("Désactivé", "", None),
            ]
            .spacing(metrics::GAP_INLINE)
            .into()
        }),
        ("menu", || {
            let act = Some(Message::Action("menu"));
            ui::menu(
                [
                    ui::menu_item("Réduire", Trailing::Shortcut("Super+M"), act.clone()).into(),
                    ui::open(ui::menu_item(
                        "Déplacer vers le bureau",
                        Trailing::Submenu,
                        act.clone(),
                    ))
                    .into(),
                    ui::menu_separator(),
                    ui::menu_item(
                        "Un réseau au nom bien trop long pour ce menu",
                        Trailing::Checked,
                        act,
                    )
                    .into(),
                ],
                metrics::MENU_WIDTH,
            )
            .into()
        }),
        ("list", || {
            column![
                ui::list_item(
                    Icon::Folder,
                    "Projets",
                    Some("12 éléments"),
                    None,
                    Some(Message::Action("liste"))
                ),
                ui::list_item(
                    Icon::Document,
                    "Rapport annuel de la résidence des Lilas, version finale.pdf",
                    Some("Hier à 18:04"),
                    None,
                    Some(Message::Action("liste"))
                ),
            ]
            .into()
        }),
        ("badges", || {
            row![
                ui::badge("Modifié", theme::Tone::Warning, false),
                ui::badge("Nouveau", theme::Tone::Success, false),
                ui::badge("Ignoré", theme::Tone::Neutral, false),
            ]
            .spacing(metrics::GAP_TIGHT * 3.0)
            .into()
        }),
        ("tooltip", || {
            iced::widget::container(ui::body("Fermer"))
                .padding(metrics::MENU_PADDING)
                .style(theme::tooltip)
                .into()
        }),
        ("ellipsis", || {
            column![
                ui::ellipsis("Livebox-Résidence-des-Lilas-étage-3-5GHz").width(180.0),
                ui::ellipsis("Maison").width(180.0),
            ]
            .into()
        }),
    ];
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("components");
    std::fs::create_dir_all(&dir).expect("folder of the captures");
    for (which, suffix) in [(color::Theme::Dark, ""), (color::Theme::Light, "-light")] {
        color::set_active(which);
        for (name, sample) in samples {
            let page = iced::widget::container(sample())
                .padding(metrics::GAP_SECTION)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill)
                .style(theme::desktop);
            let mut ui = simulator::Simulator::with_size(settings(), (560.0, 150.0), page);
            let snapshot = ui.snapshot(&theme::theme()).expect("snapshot");
            let path = dir.join(format!("{name}{suffix}.png"));
            // An image already there would be compared rather than written again.
            let _ = std::fs::remove_file(dir.join(format!("{name}{suffix}-tiny-skia.png")));
            assert!(snapshot.matches_image(&path).expect("writing the snapshot"));
        }
    }
    color::set_active(color::Theme::Dark);
}

#[test]
fn a_click_triggers_the_action() {
    let gallery = Gallery::new();
    let (width, _) = metrics::REFERENCE_SCREEN;
    let mut ui = simulator::Simulator::with_size(settings(), (width, FULL_HEIGHT), gallery.view());
    ui.click("Essayer une nouvelle version")
        .expect("button found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(messages, [Message::Action("nouvelle version")]);
}

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    First,
    Second,
    Toggle(bool),
}

type Ui<'a> = UserInterface<'a, Msg, iced::Theme, iced::Renderer>;

/// Three enabled components, a disabled one among them. `first` tells whether the first one
/// is enabled.
fn view<'a>(first: bool) -> Element<'a, Msg> {
    iced::widget::column![
        ui::primary_button("Premier", first.then_some(Msg::First)),
        ui::secondary_button("Ignoré", None),
        ui::menu_item("Second", Trailing::None, Some(Msg::Second)),
        ui::toggle(false, Some(Msg::Toggle)),
    ]
    .into()
}

fn build<'a>(view: Element<'a, Msg>, cache: Cache, renderer: &mut iced::Renderer) -> Ui<'a> {
    UserInterface::build(view, Size::new(400.0, 300.0), cache, renderer)
}

/// Moves the focus to the next component, as Tab does in a program.
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

/// The software renderer: it draws without a screen, and tests stay in software (ADR 0018).
fn software() -> Option<iced::Renderer> {
    use iced::advanced::renderer::Headless;
    iced::futures::executor::block_on(iced::Renderer::new(
        typography::UI,
        Pixels(typography::UI_SIZE),
        Some("tiny-skia"),
    ))
}

#[test]
fn the_keyboard_walks_and_activates_the_components() {
    let mut renderer = software().expect("tiny-skia starts without a screen");
    let mut ui = build(view(true), Cache::default(), &mut renderer);
    let mut messages = Vec::new();
    let enter = press(Named::Enter, false);

    // No focus: Enter does nothing.
    send(&mut ui, &enter, &mut renderer, &mut messages);
    assert!(messages.is_empty());

    // Enter held down activates only once.
    focus_next(&mut ui, &renderer);
    send(&mut ui, &enter, &mut renderer, &mut messages);
    send(
        &mut ui,
        &press(Named::Enter, true),
        &mut renderer,
        &mut messages,
    );
    // The disabled button is skipped.
    focus_next(&mut ui, &renderer);
    send(&mut ui, &enter, &mut renderer, &mut messages);
    focus_next(&mut ui, &renderer);
    send(
        &mut ui,
        &press(Named::Space, false),
        &mut renderer,
        &mut messages,
    );
    assert_eq!(messages, [Msg::First, Msg::Second, Msg::Toggle(true)]);
}

#[test]
fn a_disabled_component_loses_the_focus() {
    let mut renderer = software().expect("tiny-skia starts without a screen");
    let mut messages = Vec::new();
    let enter = press(Named::Enter, false);

    let mut ui = build(view(true), Cache::default(), &mut renderer);
    focus_next(&mut ui, &renderer);
    // The first button has the focus, then becomes disabled, then enabled again.
    let ui = build(view(false), ui.into_cache(), &mut renderer);
    let mut ui = build(view(true), ui.into_cache(), &mut renderer);
    send(&mut ui, &enter, &mut renderer, &mut messages);
    assert!(
        messages.is_empty(),
        "the focus should have gone: {messages:?}"
    );
}
