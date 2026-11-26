//! Rendering the bar and the Sherlock menu without a screen, from fake sources.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use bar::app::{Bar, Message};
use bar::calendar::Month;
use bar::history::{Entry, Group};
use bar::menu;
use bar::panel::{self, History, Panel};
use bar::quick::{self, Load};
use bar::source::battery::{Battery, Charge};
use bar::source::clock::LocalTime;
use bar::source::network::{Link, Network};
use bar::source::sound::Sound;
use bar::source::workspaces::Desktop;
use bar::system::Problem;
use bar::system::audio::Output;
use bar::system::backlight::Backlight;
use bar::system::bluetooth::Bluetooth;
use bar::system::network::{Network, Security, Wifi};
use bar::system::power::{Power, Profile};
use iced_test::simulator::Simulator;
use sherlock_ui::color::{self, Theme};
use sherlock_ui::{metrics, theme};

/// A capture sets the active theme: one at a time.
static THEME: Mutex<()> = Mutex::new(());

/// Runs `capture` in `which`, then goes back to the dark theme.
fn in_theme<R>(which: Theme, capture: impl FnOnce() -> R) -> R {
    let _one = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(which);
    let result = capture();
    color::set_active(Theme::Dark);
    result
}

/// The bar of the Main design board: three workspaces, the first one active, the editor
/// focused.
fn sample() -> Bar {
    let mut state = Bar::default();
    let messages = [
        Message::Clock(LocalTime {
            year: 2026,
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
        Message::Desktop(Desktop::placeholder(Some("Code,Web,Jeu"), Some("Éditeur"))),
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

/// The bar and its menu in each theme: `bar`, `sherlock-menu`, then `bar-light` and
/// `sherlock-menu-light`, compared with the Main, Menus, MainClair and MenusClair boards.
#[test]
fn the_bar_and_its_menu_render_in_both_themes() {
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            let ui = Simulator::with_size(bar::settings(), bar_size(), state.view_bar());
            assert_written(capture(ui, &format!("bar{suffix}")));
            state.cursor = Some(1);
            let size = menu_size(&state);
            let ui = Simulator::with_size(bar::settings(), size, state.view_menu());
            assert_written(capture(ui, &format!("sherlock-menu{suffix}")));
        });
    }
}

/// The bar attached to the top edge, as over a maximized window: the whole width, square
/// corners, opaque. The third workspace holds no window: its chip is bare. `bar-attached`, then
/// `bar-attached-light`.
#[test]
fn the_attached_bar_renders_in_both_themes() {
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            let mut desktop = Desktop::placeholder(Some("Code,Web,Jeu"), Some("Éditeur"));
            desktop.groups[0].workspaces[2].windows = Some(0);
            let _ = state.update(Message::Desktop(desktop));
            let now = Instant::now();
            assert!(state.shape.set(true, now));
            state.shape.tick(now + Duration::from_secs(1));
            let size = (metrics::REFERENCE_SCREEN.0, metrics::TOP_BAR_HEIGHT);
            let ui = Simulator::with_size(bar::settings(), size, state.view_bar());
            assert_written(capture(ui, &format!("bar-attached{suffix}")));
        });
    }
}

#[test]
fn the_menu_surface_sized_for_the_gpu_renders() {
    // The surface the GPU gets, with room for the shadow. Tests render in software, which
    // draws the crisp shadow: this only checks that the capture is written. The blurred shadow
    // shows with `ICED_TEST_BACKEND=wgpu` and the variables of ADR 0018 (docs/bar.md).
    in_theme(Theme::Dark, || {
        let mut state = sample();
        state.menu_margins = menu::margins(true);
        let ui = Simulator::with_size(bar::settings(), menu_size(&state), state.view_menu());
        assert_written(capture(ui, "sherlock-menu-shadow"));
    });
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

/// A notification of the Panels board.
fn entry(
    id: u32,
    app: &str,
    summary: &str,
    body: &str,
    when: &str,
    actions: &[(&str, &str)],
) -> Entry {
    Entry {
        id,
        app: app.to_owned(),
        summary: summary.to_owned(),
        body: body.to_owned(),
        actions: actions
            .iter()
            .map(|(k, l)| ((*k).to_owned(), (*l).to_owned()))
            .collect(),
        open: !actions.is_empty(),
        received: 0,
        critical: false,
        when: when.to_owned(),
    }
}

/// The notification centre of the Panels board: three applications, the second one with its
/// two buttons, Sherlock with two notifications.
fn board_history() -> History {
    History::Ready(vec![
        Group {
            app: String::from("Sherlock"),
            entries: vec![
                entry(
                    5,
                    "Sherlock",
                    "Build terminé",
                    "compositeur a compilé sans erreur.",
                    "21:44",
                    &[],
                ),
                entry(
                    2,
                    "Sherlock",
                    "Tests passés",
                    "412 tests, aucun échec.",
                    "20:02",
                    &[],
                ),
            ],
        },
        Group {
            app: String::from("Conteneurs"),
            entries: vec![entry(
                4,
                "Conteneurs",
                "front-next s'est arrêté",
                "Le conteneur ne répond plus.",
                "21:31",
                &[("restart", "Relancer"), ("logs", "Voir les logs")],
            )],
        },
        Group {
            app: String::from("Mises à jour"),
            entries: vec![entry(
                3,
                "Mises à jour",
                "3 paquets à mettre à jour",
                "mesa, pipewire et le noyau.",
                "20:15",
                &[],
            )],
        },
    ])
}

fn panel_size(state: &Bar, which: Panel) -> (f32, f32) {
    let (width, height) = panel::surface_size(which, state.panel_margins);
    (width as f32, height as f32)
}

/// The quick settings of the Panels board: Wi-Fi on « Maison », Bluetooth off, sound at 62 %,
/// brightness at 80 %, the balanced profile; no service actually read.
fn board_quick(state: &mut Bar) {
    let maison = Network {
        ssid: String::from("Maison"),
        strength: 90,
        security: Security::Psk,
        active: true,
        path: String::from("/ap/1"),
    };
    let cafe = Network {
        ssid: String::from("Café du coin"),
        strength: 60,
        security: Security::Open,
        active: false,
        path: String::from("/ap/2"),
    };
    let messages = [
        quick::Message::Wifi(Load::Ready(Wifi {
            enabled: true,
            hardware_off: false,
            device: Some(String::from("/dev/1")),
            networks: vec![maison, cafe],
        })),
        quick::Message::Bluetooth(Load::Ready(Bluetooth {
            adapter: String::from("/org/bluez/hci0"),
            powered: false,
            connected: 0,
        })),
        quick::Message::Sound(Load::Ready(Output {
            sound: Sound {
                volume: 62,
                muted: false,
            },
            name: String::from("Haut-parleurs intégrés"),
        })),
        quick::Message::Brightness(Load::Ready(Backlight {
            name: String::from("amdgpu_bl1"),
            brightness: 204,
            max: 255,
        })),
        quick::Message::Energy(Load::Ready(Power::new(
            Some(Profile::Balanced),
            Profile::ALL.to_vec(),
        ))),
    ];
    for message in messages {
        let _ = state.quick.update(message);
    }
    state.quick.do_not_disturb = Some(false);
    // Only for the tile to be clickable: the tests never run the messages.
    state.quick.settings = Some(PathBuf::from("notifications.conf"));
}

/// The calendar and the control centre in each theme: `calendar`, `control-centre` (the
/// board's notifications and quick settings), `control-centre-empty` (no notification, no
/// service: what a machine without NetworkManager, BlueZ, backlight nor power-profiles-daemon
/// shows), `wifi-networks` (the network page, a password asked), then their `-light`
/// versions.
#[test]
fn the_panels_render_in_both_themes() {
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            state.panel_margins = panel::margins(false);
            state.month = Some(Month {
                year: 2026,
                month: 11,
            });
            let size = panel_size(&state, Panel::Calendar);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Calendar));
            assert_written(capture(ui, &format!("calendar{suffix}")));

            let size = panel_size(&state, Panel::Control);
            state.history = History::Ready(Vec::new());
            let absent = || Problem::Absent(String::from("test"));
            for message in [
                quick::Message::Wifi(Load::Failed(absent())),
                quick::Message::Bluetooth(Load::Failed(absent())),
                quick::Message::Sound(Load::Failed(absent())),
                quick::Message::Brightness(Load::Failed(absent())),
                quick::Message::Energy(Load::Failed(absent())),
            ] {
                let _ = state.quick.update(message);
            }
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("control-centre-empty{suffix}")));

            state.history = board_history();
            board_quick(&mut state);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("control-centre{suffix}")));

            let _ = state.quick.update(quick::Message::ShowNetworks(true));
            let maison = state.quick.wifi.ready().map(|w| w.networks[0].clone());
            let mut other = maison.unwrap();
            other.active = false;
            other.path = String::from("/ap/3");
            other.ssid = String::from("Voisin");
            state.quick.joining = Some(other.clone());
            if let Load::Ready(wifi) = &mut state.quick.wifi {
                wifi.networks.push(other);
            }
            state.quick.password = String::from("secret");
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("wifi-networks{suffix}")));
        });
    }
}

#[test]
fn the_calendar_marks_today_and_walks_the_months() {
    let mut state = sample();
    state.panel_margins = panel::margins(false);
    let size = panel_size(&state, Panel::Calendar);
    let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Calendar));
    for text in ["novembre 2026", "lu", "di", "24", "30"] {
        assert!(ui.find(text).is_ok(), "{text} missing from the calendar");
    }
    ui.click("Mois suivant").expect("arrow found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(
        messages[..],
        [Message::ShowMonth(Month {
            year: 2026,
            month: 12
        })]
    ));
    let _ = state.update(Message::ShowMonth(Month {
        year: 2026,
        month: 12,
    }));
    let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Calendar));
    let mut ui = ui;
    assert!(ui.find("décembre 2026").is_ok());
}

#[test]
fn the_centre_runs_an_action_and_clears() {
    let mut state = sample();
    state.panel_margins = panel::margins(false);
    state.history = board_history();
    let size = panel_size(&state, Panel::Control);
    let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
    for text in [
        "Notifications",
        "Build terminé",
        "Conteneurs",
        "21:31",
        "Mises à jour",
    ] {
        assert!(ui.find(text).is_ok(), "{text} missing from the centre");
    }
    ui.click("Relancer").expect("action found");
    ui.click("Tout effacer").expect("button found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(
        &messages[..],
        [Message::InvokeAction(4, key), Message::ClearHistory] if key == "restart"
    ));
}

#[test]
fn a_click_on_the_indicators_or_the_clock_opens_their_panel() {
    let state = sample();
    let mut ui = Simulator::with_size(bar::settings(), bar_size(), state.view_bar());
    ui.click("21:46").expect("clock found");
    ui.click("82\u{202F}%").expect("battery found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(
        messages[..],
        [
            Message::TogglePanel(Panel::Calendar),
            Message::TogglePanel(Panel::Control)
        ]
    ));
}

#[test]
fn the_quick_settings_answer_their_clicks() {
    let mut state = sample();
    state.panel_margins = panel::margins(false);
    state.history = History::Ready(Vec::new());
    board_quick(&mut state);
    let size = panel_size(&state, Panel::Control);
    let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
    for text in [
        "Wi-Fi",
        "Maison",
        "Bluetooth",
        "Désactivé",
        "Mode jeu",
        "Bientôt",
        "Haut-parleurs intégrés",
        "Équilibré",
        "Dodo",
    ] {
        assert!(
            ui.find(text).is_ok(),
            "{text} missing from the quick settings"
        );
    }
    ui.click("Bluetooth").expect("tile found");
    ui.click("Économie").expect("profile found");
    ui.click("Ne pas déranger").expect("tile found");
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(matches!(
        messages[..],
        [
            Message::Quick(quick::Message::ToggleBluetooth),
            Message::Quick(quick::Message::Profile(Profile::Saver)),
            Message::Quick(quick::Message::ToggleDoNotDisturb),
        ]
    ));
    // The game mode and the graphics card do nothing yet.
    let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
    ui.click("Mode jeu").expect("tile found");
    ui.click("NVIDIA").expect("segment found");
    assert_eq!(ui.into_messages().count(), 0);
}
