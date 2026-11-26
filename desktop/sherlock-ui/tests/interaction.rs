//! The pointer shape each component asks for, in each of its states (lot UI-01): the
//! pointing hand over what acts, the open then closed hand on a slider, the text cursor in a
//! field, the forbidden sign over what is disabled.

use iced::advanced::clipboard;
use iced::mouse::{self, Interaction};
use iced::{Element, Event, Point, Size};
use iced_runtime::user_interface::{Cache, State, UserInterface};
use sherlock_ui::component::{self as ui, Trailing};
use sherlock_ui::icon::Icon;
use sherlock_ui::panels::{self, Glyph};
use sherlock_ui::{pointer, typography};

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Press,
    Toggle(bool),
    Value(u8),
    Text(String),
}

const SIZE: Size = Size::new(320.0, 60.0);

fn renderer() -> Option<iced::Renderer> {
    use iced::advanced::renderer::Headless;
    iced::futures::executor::block_on(iced::Renderer::new(
        typography::UI,
        iced::Pixels(typography::UI_SIZE),
        Some("tiny-skia"),
    ))
}

/// The shape asked for after `events`, the pointer at `at`.
fn shape_after(view: Element<'_, Msg>, at: Point, events: &[Event]) -> Interaction {
    let Some(mut renderer) = renderer() else {
        panic!("tiny-skia starts without a screen");
    };
    let mut ui = UserInterface::build(view, SIZE, Cache::default(), &mut renderer);
    let mut messages = Vec::new();
    let mut moved = vec![Event::Mouse(mouse::Event::CursorMoved { position: at })];
    moved.extend_from_slice(events);
    let (state, _) = ui.update(
        &moved,
        mouse::Cursor::Available(at),
        &mut renderer,
        &mut clipboard::Null,
        &mut messages,
    );
    match state {
        State::Updated {
            mouse_interaction, ..
        } => mouse_interaction,
        State::Outdated => panic!("the interface is outdated"),
    }
}

/// The shape with the pointer at `at`.
fn shape(view: Element<'_, Msg>, at: Point) -> Interaction {
    shape_after(view, at, &[])
}

/// Over the top left of the component: every component is laid at the top left.
const OVER: Point = Point::new(12.0, 12.0);
/// Outside every component.
const AWAY: Point = Point::new(310.0, 58.0);

fn press() -> Option<Msg> {
    Some(Msg::Press)
}

/// Each clickable component, enabled then disabled.
fn controls(enabled: bool) -> Vec<(&'static str, Element<'static, Msg>)> {
    let on = enabled.then_some(Msg::Press);
    let toggle: Option<fn(bool) -> Msg> = enabled.then_some(Msg::Toggle as fn(bool) -> Msg);
    vec![
        (
            "primary button",
            ui::primary_button("Relancer", on.clone()).into(),
        ),
        (
            "secondary button",
            ui::secondary_button("Logs", on.clone()).into(),
        ),
        (
            "danger button",
            ui::danger_button("Vider", on.clone()).into(),
        ),
        (
            "icon button",
            ui::icon_button(Icon::Close, "Fermer", on.clone()).into(),
        ),
        (
            "toolbar button",
            ui::toolbar_button(Icon::Back, "Précédent", on.clone()).into(),
        ),
        ("toggle", ui::toggle(true, toggle).into()),
        (
            "menu entry",
            ui::menu_item("Réglages", Trailing::Submenu, on.clone()).into(),
        ),
        (
            "navigation entry",
            ui::nav_item(Icon::Home, "Accueil", None, false, on.clone()).into(),
        ),
        (
            "workspace chip",
            ui::chip("1 · Code", false, on.clone()).into(),
        ),
        (
            "Sherlock button",
            ui::sherlock_button(false, on.clone()).into(),
        ),
        (
            "bar button",
            panels::bar_button(ui::body("21:46"), false, on.clone()).into(),
        ),
        (
            "quick tile",
            panels::quick_tile(Glyph::Wifi, "Wi-Fi", "Maison".into(), 1.0, on.clone(), None),
        ),
        (
            "segmented control",
            panels::segmented(vec![("Auto", false, on.clone()), ("AMD", false, None)]),
        ),
        (
            "glyph button",
            panels::glyph_button(Glyph::Volume, "Couper le son", on.clone()).into(),
        ),
        (
            "round button",
            panels::round_button(Glyph::Lock, "Verrouiller", false, on.clone()).into(),
        ),
        (
            "clear button",
            panels::clear_button("Tout effacer", on.clone()).into(),
        ),
        (
            "notification action",
            panels::action_button("Ouvrir", true, on.clone()).into(),
        ),
        (
            "device",
            panels::device_row(Glyph::Headphones, "Casque", String::new(), on, None),
        ),
    ]
}

#[test]
fn what_acts_shows_the_pointing_hand() {
    for (name, view) in controls(true) {
        assert_eq!(shape(view, OVER), Interaction::Pointer, "{name}");
    }
}

#[test]
fn what_is_disabled_shows_the_forbidden_sign() {
    for (name, view) in controls(false) {
        assert_eq!(shape(view, OVER), Interaction::NotAllowed, "{name}");
    }
}

#[test]
fn away_from_a_component_the_pointer_is_left_alone() {
    for enabled in [true, false] {
        for (name, view) in controls(enabled) {
            let away = shape(view, AWAY);
            assert!(
                matches!(away, Interaction::None | Interaction::Idle),
                "{name}: {away:?}"
            );
        }
    }
}

/// The volume row of the control centre: its mute button, then the slider.
fn slider(enabled: bool) -> Element<'static, Msg> {
    let on_change: Option<fn(u8) -> Msg> = enabled.then_some(Msg::Value as fn(u8) -> Msg);
    let mute = panels::glyph_button(
        Glyph::Volume,
        "Couper le son",
        enabled.then(press).flatten(),
    );
    panels::slider_row(mute, 40, on_change)
}

#[test]
fn a_slider_shows_the_open_hand_then_the_closed_one_while_dragged() {
    let rail = Point::new(200.0, panels::ROW_BUTTON / 2.0);
    assert_eq!(shape(slider(true), rail), Interaction::Grab);
    let down = [Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))];
    assert_eq!(
        shape_after(slider(true), rail, &down),
        Interaction::Grabbing
    );
    // The mute button next to it is a button.
    assert_eq!(shape(slider(true), OVER), Interaction::Pointer);
    // Without a sound output: nothing to grab, nothing to press.
    assert_eq!(shape(slider(false), rail), Interaction::NotAllowed);
    assert_eq!(shape(slider(false), OVER), Interaction::NotAllowed);
}

#[test]
fn a_text_field_shows_the_text_cursor_but_disabled() {
    let field = |enabled: bool| -> Element<'static, Msg> {
        let on_input: Option<fn(String) -> Msg> = enabled.then_some(Msg::Text as fn(String) -> Msg);
        let field = ui::text_field("Mot de passe", "", on_input).secure(true);
        if enabled {
            field.into()
        } else {
            pointer::disabled(field).into()
        }
    };
    assert_eq!(shape(field(true), OVER), Interaction::Text);
    assert_eq!(shape(field(false), OVER), Interaction::NotAllowed);
}
