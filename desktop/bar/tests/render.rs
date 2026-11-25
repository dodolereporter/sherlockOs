//! Rendering the bar and the Sherlock menu without a screen, from fake sources.

use std::path::PathBuf;

use bar::app::{Bar, Message};
use bar::menu;
use bar::source::battery::{Battery, Charge};
use bar::source::clock::LocalTime;
use bar::source::network::{Link, Network};
use bar::source::sound::Sound;
use bar::source::workspaces::Workspaces;
use iced_test::simulator::Simulator;
use sherlock_ui::{metrics, theme};

/// The bar of the Main design board: three workspaces, the first one active, the editor
/// focused.
fn sample() -> Bar {
    let mut state = Bar::default();
    let messages = [
        Message::Clock(LocalTime {
            weekday: 2,
            day: 24,
            month: 11,
            hour: 21,
            minute: 46,
        }),
        Message::Battery(Some(Battery {
            percent: 82,
            charge: Charge::Discharging,
        })),
        Message::Network(Network::Connected {
            link: Link::Wifi,
            name: String::from("Maison"),
        }),
        Message::Sound(Sound {
            volume: 55,
            muted: false,
        }),
        Message::Workspaces(Workspaces::placeholder(
            Some("Code,Web,Jeu"),
            Some("Éditeur"),
        )),
    ];
    for message in messages {
        let _ = state.update(message);
    }
    state.user = String::from("Dodo");
    state.menu_margins = menu::margins(false);
    state
}

/// Writes the screenshot to `target/tmp/<name>-<renderer>.png` (`tiny-skia`, or `wgpu` with
/// `ICED_TEST_BACKEND=wgpu`), returns its path and size.
fn capture(
    mut ui: Simulator<'_, Message>,
    name: &str,
) -> Result<(PathBuf, u64), Box<dyn std::error::Error>> {
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let written = path.with_file_name(format!("{name}-{renderer}.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    let size = std::fs::metadata(&written)?.len();
    Ok((written, size))
}

/// A screenshot that was written and is not empty.
fn assert_written(result: Result<(PathBuf, u64), Box<dyn std::error::Error>>) {
    match result {
        Ok((path, size)) => assert!(size > 2_000, "{} too small", path.display()),
        Err(error) => panic!("cannot capture: {error}"),
    }
}

/// Width of the bar on the reference screen, margins removed.
fn bar_size() -> (f32, f32) {
    let (width, _) = metrics::REFERENCE_SCREEN;
    let width = width - 2.0 * metrics::TOP_BAR_MARGIN_SIDE;
    (width, metrics::TOP_BAR_HEIGHT)
}

fn menu_size(state: &Bar) -> (f32, f32) {
    let (width, height) = menu::surface_size(state.menu_margins);
    (width as f32, height as f32)
}

#[test]
fn the_bar_renders_without_a_screen() {
    let state = sample();
    let ui = Simulator::with_size(bar::settings(), bar_size(), state.view_bar());
    assert_written(capture(ui, "bar"));
}

#[test]
fn the_menu_renders_without_a_screen() {
    let mut state = sample();
    state.cursor = Some(1);
    let ui = Simulator::with_size(bar::settings(), menu_size(&state), state.view_menu());
    assert_written(capture(ui, "sherlock-menu"));
}

#[test]
fn the_menu_renders_with_room_for_its_shadow() {
    // The surface the GPU gets: the blurred shadow drawn around the menu. In software (the
    // default for tests), the crisp shadow is drawn instead.
    let mut state = sample();
    state.menu_margins = menu::margins(true);
    let ui = Simulator::with_size(bar::settings(), menu_size(&state), state.view_menu());
    assert_written(capture(ui, "sherlock-menu-shadow"));
}

#[test]
fn a_click_on_the_shadow_closes_the_menu() {
    // The surface sized for the GPU's blurred shadow: a click there is outside the menu.
    let mut state = sample();
    state.menu_margins = menu::margins(true);
    let mut ui = Simulator::with_size(bar::settings(), menu_size(&state), state.view_menu());
    ui.point_at(iced::Point::new(4.0, 4.0));
    ui.simulate(iced_test::simulator::click());
    ui.click("Verrouiller").expect("entry found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(
        messages[..],
        [
            Message::MenuKey(menu::Key::Close),
            Message::Activate(menu::Action::Lock)
        ]
    ));
}

#[test]
fn the_bar_shows_time_date_and_workspaces() {
    let state = sample();
    let mut ui = Simulator::with_size(bar::settings(), bar_size(), state.view_bar());
    for text in ["21:46", "mar. 24 nov.", "82\u{202F}%", "Éditeur", "3 · Jeu"] {
        assert!(ui.find(text).is_ok(), "{text} missing from the bar");
    }
    ui.click("2 · Web").expect("workspace found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(messages[..], [Message::SelectWorkspace(1)]));
}

#[test]
fn a_click_in_the_menu_picks_its_entry() {
    let state = sample();
    let mut ui = Simulator::with_size(bar::settings(), menu_size(&state), state.view_menu());
    assert!(ui.find("Dodo").is_ok());
    ui.click("Verrouiller").expect("entry found");
    let activated = ui
        .into_messages()
        .any(|m| matches!(m, Message::Activate(menu::Action::Lock)));
    assert!(activated);
}
