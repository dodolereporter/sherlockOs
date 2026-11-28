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
use bar::system::keyboard::{KeyboardLight, Through};
use bar::system::network::{Network as WifiNetwork, Outcome, Security, Wifi};
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
            minutes: Some(192),
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
    state.user = String::from("Camille");
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

/// Lot UI-09: without a GPU the bar renders with tiny-skia, whose buffers lose their alpha.
/// Its glass is then drawn opaque, the color it shows over the desktop, not mixed with black:
/// in the light theme a light glass, and « 12 % » of a low battery readable. At 1920 and 1366,
/// `bar-software-light-1920` and `bar-software-light-1366`, copied to `screenshots/`.
#[test]
fn in_software_the_bar_glass_is_opaque_with_its_own_color() {
    for width in [1920.0, 1366.0] {
        in_theme(Theme::Light, || {
            let mut state = sample();
            let low = Battery {
                percent: 12,
                charge: Charge::Discharging,
                minutes: Some(25),
            };
            let _ = state.update(Message::Battery(Some(low)));
            let size = (
                width - 2.0 * metrics::TOP_BAR_MARGIN_SIDE,
                metrics::TOP_BAR_HEIGHT,
            );
            let ui = Simulator::with_size(bar::settings(), size, state.view_bar());
            let name = format!("bar-software-light-{width}");
            let (path, _) = capture(ui, &name).unwrap();
            // tiny-skia drew it: the glass is opaque from the first frame on.
            assert_eq!(sherlock_ui::surface::gpu_drawn(), Some(false));
            let p = color::palette();
            assert!(p.top_bar_glass.a >= 1.0);
            let expected = color::Palette::LIGHT
                .top_bar_glass
                .over(color::Palette::LIGHT.desktop);
            assert_eq!(p.top_bar_glass, expected);
            // A pixel of glass in the middle of the bar, between the workspaces and the
            // clock: opaque, of that color (at the capture's scale).
            let image = image::open(&path).unwrap().to_rgba8();
            let pixel = image.get_pixel(image.width() / 2, image.height() - 4);
            assert_eq!(pixel.0[3], 255, "opaque");
            let close = |a: u8, b: u8| a.abs_diff(b) <= 2;
            assert!(
                close(pixel.0[0], expected.r) && close(pixel.0[1], expected.g),
                "{:?} for {expected:?}",
                pixel.0
            );
            // « 12 % » in amber on it: 4.5 to 1 at least.
            assert!(p.amber_text.contrast(p.top_bar_glass) >= 4.5);
            let copy = format!("{}/screenshots/{name}.png", env!("CARGO_MANIFEST_DIR"));
            std::fs::copy(&path, copy).unwrap();
        });
    }
    color::set_opaque_glass(false);
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
    assert!(ui.find("Camille").is_ok());
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
    let (width, height) = panel::surface_size(which, state.panel_margins, state.panel_shared);
    (width as f32, height as f32)
}

/// The quick settings of the Panels board: Wi-Fi on « Maison », Bluetooth off, sound at 62 %,
/// brightness at 80 %, the balanced profile; no service actually read.
fn board_quick(state: &mut Bar) {
    let maison = WifiNetwork {
        ssid: String::from("Maison"),
        strength: 90,
        security: Security::Psk,
        active: true,
        saved: true,
        path: String::from("/ap/1"),
    };
    let cafe = WifiNetwork {
        ssid: String::from("Café du coin"),
        strength: 60,
        security: Security::Open,
        active: false,
        saved: false,
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
            devices: Vec::new(),
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

/// The panels in each theme: `calendar` (the calendar over the board's notifications),
/// `control-centre` (the board's quick settings), `control-centre-keyboard` (the same with a
/// keyboard backlight), `control-centre-empty` (no service: what a
/// machine without NetworkManager, BlueZ, backlight nor power-profiles-daemon shows),
/// `wifi-networks` (the network page, a password asked), then their `-light` versions.
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
            state.history = board_history();
            let size = panel_size(&state, Panel::Calendar);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Calendar));
            assert_written(capture(ui, &format!("calendar{suffix}")));

            let size = panel_size(&state, Panel::Control);
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

            board_quick(&mut state);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("control-centre{suffix}")));

            // A machine with a keyboard backlight: its slider under the brightness (D-35d).
            let light = KeyboardLight {
                brightness: 1,
                max: 3,
                through: Through::UPower,
            };
            let _ = state
                .quick
                .update(quick::Message::Keyboard(Load::Ready(light)));
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("control-centre-keyboard{suffix}")));
            state.quick.keyboard = Load::Loading;

            let _ = state.quick.update(quick::Message::ShowNetworks(true));
            // The page slid in.
            state.quick.tick(state.quick.now + Duration::from_secs(5));
            let maison = state.quick.wifi.ready().map(|w| w.networks[0].clone());
            let mut other = maison.unwrap();
            other.active = false;
            other.path = String::from("/ap/3");
            other.ssid = String::from("Voisin");
            state.quick.joining = Some(other.clone());
            if let Load::Ready(wifi) = &mut state.quick.wifi {
                wifi.networks.push(other);
            }
            state.quick.password = quick::Secret::new(String::from("secret"));
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("wifi-networks{suffix}")));

            // Its password refused (D-20e): asked again, said under the field.
            state.quick.password = quick::Secret::default();
            state.quick.refused = Some((String::from("Voisin"), Outcome::WrongPassword));
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("wifi-wrong-password{suffix}")));
            // « Autre réseau… » open, its password shown.
            state.quick.refused = None;
            state.quick.joining = None;
            state.quick.other = Some((String::from("Cachette"), Security::Psk));
            state.quick.password = quick::Secret::new(String::from("visible"));
            state.quick.reveal = true;
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("wifi-other-network{suffix}")));
            state.quick.other = None;
            state.quick.reveal = false;

            // A network name too long for its tile and its line ends with « … » (UI-01).
            let long = "Livebox-Résidence-des-Lilas-étage-3-5GHz";
            if let Load::Ready(wifi) = &mut state.quick.wifi {
                wifi.networks[0].ssid = String::from(long);
            }
            state.quick.joining = None;
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("wifi-long-name{suffix}")));
            let _ = state.quick.update(quick::Message::ShowNetworks(false));
            state.quick.tick(state.quick.now + Duration::from_secs(5));
            let mut ui =
                Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            // Selectors still find the whole name.
            assert!(ui.find(long).is_ok());
            assert_written(capture(ui, &format!("control-centre-long-name{suffix}")));
        });
    }
}

/// Do not disturb for a time (D-23e): the moon among the indicators (`bar-do-not-disturb`), the
/// tile saying until when and its durations page (`do-not-disturb-durations`).
#[test]
fn do_not_disturb_shows_in_the_bar_and_its_durations() {
    let mut state = sample();
    state.do_not_disturb = true;
    let ui = Simulator::with_size(bar::settings(), bar_size(), state.view_bar());
    assert_written(capture(ui, "bar-do-not-disturb"));
    state.panel_margins = panel::margins(false);
    board_quick(&mut state);
    state.quick.do_not_disturb = Some(true);
    state.quick.dnd_until = Some(notifications_config::epoch_now() + 3600);
    let size = panel_size(&state, Panel::Control);
    {
        let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
        assert!(ui.find("Ne pas déranger").is_ok());
    }
    let _ = state.quick.update(quick::Message::ShowFocus(true));
    state.quick.tick(state.quick.now + Duration::from_secs(5));
    let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
    assert!(ui.find("Pendant 1 heure").is_ok());
    assert!(ui.find("Jusqu’à demain matin").is_ok());
    assert_written(capture(ui, "do-not-disturb-durations"));
}

/// The outputs and the inputs of the sound (D-20g): the speakers in use, HDMI and a
/// Bluetooth headset, two inputs and the microphone's slider. `sound-devices`, both themes.
#[test]
fn the_outputs_and_inputs_render_in_both_themes() {
    use bar::system::audio::Endpoint;
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            state.panel_margins = panel::margins(false);
            board_quick(&mut state);
            let endpoint = |name: &str, description: &str| Endpoint {
                name: name.to_owned(),
                description: description.to_owned(),
            };
            let devices = quick::Devices {
                outputs: vec![
                    endpoint("speakers", "Haut-parleurs intégrés"),
                    endpoint("hdmi", "HDMI / DisplayPort"),
                    endpoint("headset", "Casque WH-1000XM4"),
                ],
                inputs: vec![
                    endpoint("mic", "Microphone intégré"),
                    endpoint("headset-mic", "Casque WH-1000XM4"),
                ],
                output: String::from("speakers"),
                input: String::from("mic"),
                microphone: Some(Sound {
                    volume: 70,
                    muted: false,
                }),
            };
            let _ = state.quick.update(quick::Message::ShowSound(true));
            let _ = state
                .quick
                .update(quick::Message::Audio(Load::Ready(devices)));
            state.quick.tick(state.quick.now + Duration::from_secs(5));
            let size = panel_size(&state, Panel::Control);
            let mut ui =
                Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert!(ui.find("HDMI / DisplayPort").is_ok());
            let _ = ui.click("HDMI / DisplayPort");
            let messages: Vec<_> = ui.into_messages().collect();
            assert!(
                messages.iter().any(
                    |m| matches!(m, Message::Quick(quick::Message::ChooseOutput(n)) if n == "hdmi")
                ),
                "{messages:?}"
            );
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("sound-devices{suffix}")));
        });
    }
}

/// The panels on short screens (UI-08), the logical heights of 1280 × 800 at 1.5 (`-533`),
/// 1366 × 768 at 1.25 (`-614`) and 1920 × 1080 at 1.5 (`-720`): `calendar-<height>`, the
/// centre shorter, or both scrolling together on the shortest; `control-centre-<height>`, the
/// settings scrolling in a panel that ends above the bottom of the screen.
#[test]
fn the_panels_fit_short_screens() {
    for (screen, tag) in [(533.3, "533"), (614.4, "614"), (720.0, "720")] {
        let mut state = sample();
        state.panel_margins = panel::margins(false);
        state.month = Some(Month {
            year: 2026,
            month: 11,
        });
        state.history = board_history();
        board_quick(&mut state);
        let light = KeyboardLight {
            brightness: 1,
            max: 3,
            through: Through::UPower,
        };
        let _ = state
            .quick
            .update(quick::Message::Keyboard(Load::Ready(light)));
        panel::reserve_keyboard(true);
        panel::reserve_screen(Some(screen));
        for (which, name) in [
            (Panel::Calendar, "calendar"),
            (Panel::Control, "control-centre"),
        ] {
            let size = panel_size(&state, which);
            let top = 54.0;
            assert!(top + size.1 <= screen, "{name} on {screen}: {size:?}");
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(which));
            assert_written(capture(ui, &format!("{name}-{tag}")));
        }
        panel::reserve_screen(None);
        panel::reserve_keyboard(false);
    }
}

/// The motions inside a panel, caught halfway (UI-01): `page-sliding`, the Wi-Fi networks
/// coming in over the main page going out, both cut to the glass; `panels-switching`, the
/// control centre turning into the calendar in the surface both share on the GPU, a mask going
/// from one height to the other. Then their `-light` versions.
#[test]
fn the_motions_of_the_panels_stay_inside_them() {
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            state.panel_margins = panel::margins(false);
            state.month = Some(Month {
                year: 2026,
                month: 11,
            });
            state.history = board_history();
            board_quick(&mut state);
            let _ = state.quick.update(quick::Message::ShowNetworks(true));
            state
                .quick
                .tick(state.quick.now + Duration::from_millis(60));
            assert!(state.quick.running());
            let size = panel_size(&state, Panel::Control);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("page-sliding{suffix}")));

            state.quick.tick(state.quick.now + Duration::from_secs(5));
            let _ = state.quick.update(quick::Message::ShowNetworks(false));
            state.quick.tick(state.quick.now + Duration::from_secs(5));
            state.panel_shared = true;
            // The surface of the GPU, with the room for the shadow.
            state.panel_margins = panel::margins(true);
            let start = Duration::from_secs(1);
            let mut motion = bar::unfold::Unfold::opening(start);
            motion.tick(start + Duration::from_millis(35));
            state.swap = Some((Panel::Control, motion));
            let size = panel_size(&state, Panel::Calendar);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Calendar));
            assert_written(capture(ui, &format!("panels-switching{suffix}")));
        });
    }
}

/// The control centre halfway through its opening, as on the GPU (UI-04): its icons as faded
/// and as far along as its texts and its tiles. `control-centre-opening`, then
/// `control-centre-opening-light`.
#[test]
fn the_icons_of_an_opening_panel_fade_with_it() {
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            state.panel_margins = panel::margins(true);
            board_quick(&mut state);
            let start = Duration::from_secs(1);
            let mut unfold = bar::unfold::Unfold::opening(start);
            unfold.tick(start + Duration::from_millis(45));
            assert!(unfold.opacity() > 0.3 && unfold.opacity() < 0.9);
            state.unfold = unfold;
            let size = panel_size(&state, Panel::Control);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("control-centre-opening{suffix}")));
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
    // The arrows are icons: their label only shows on hover. « Mois suivant » is the last
    // button of the month row, at the right of the panel, under its padding.
    let inset = sherlock_ui::panels::PADDING + sherlock_ui::metrics::BORDER_WIDTH;
    let button = sherlock_ui::metrics::ICON_BUTTON;
    let x = Panel::Calendar.size().0 - inset - button / 2.0;
    let y = state.panel_margins.top + inset + sherlock_ui::panels::MONTH_ROW / 2.0;
    ui.point_at(iced::Point::new(x, y));
    ui.simulate(iced_test::simulator::click());
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
    // The containers' card alone: under the calendar, the list shows the first card or two.
    state.history = match board_history() {
        History::Ready(groups) => History::Ready(
            groups
                .into_iter()
                .filter(|g| g.app == "Conteneurs")
                .collect(),
        ),
        other => other,
    };
    let size = panel_size(&state, Panel::Calendar);
    let mut ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Calendar));
    for text in ["novembre 2026", "Notifications", "Conteneurs", "21:31"] {
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

/// The bar with the control centre open: the indicators, one button, keep one veil and the
/// green mark under the whole group (UI-02). `bar-open`, then `bar-open-light`.
#[test]
fn the_button_of_the_open_panel_is_marked() {
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            state.panel_motion = Some(false);
            let _ = state.update(Message::Surface(iced::window::Id::unique()));
            let _ = state.update(Message::TogglePanel(Panel::Control));
            assert_eq!(state.panel_open(), Some(Panel::Control));
            let ui = Simulator::with_size(bar::settings(), bar_size(), state.view_bar());
            assert_written(capture(ui, &format!("bar-open{suffix}")));
            // The whole bar of the author's 1920 × 1080, the group marked under the open
            // control centre.
            if which == Theme::Dark {
                let size = (
                    1920.0 - 2.0 * metrics::TOP_BAR_MARGIN_SIDE,
                    metrics::TOP_BAR_HEIGHT,
                );
                let ui = Simulator::with_size(bar::settings(), size, state.view_bar());
                assert_written(capture(ui, "bar-open-1920"));
            }
            let _ = state.update(Message::TogglePanel(Panel::Control));
            assert_eq!(state.panel_open(), None);
        });
    }
}

/// A click anywhere on the group of indicators, on any of its icons or between them, opens the
/// control centre (UI-02).
#[test]
fn a_click_anywhere_on_the_indicators_opens_the_control_centre() {
    let state = sample();
    let (width, height) = bar_size();
    // The group sits left of the clock: its battery value, then points across it.
    let mut ui = Simulator::with_size(bar::settings(), (width, height), state.view_bar());
    let battery = ui.find("82\u{202F}%").expect("battery found");
    let right = battery.visible_bounds().map_or(0.0, |b| b.x + b.width);
    let mut points = Vec::new();
    // From the battery value leftwards, every 12 px over the group: the icons and the gaps.
    let mut x = right - 2.0;
    for _ in 0..8 {
        points.push(iced::Point::new(x, height / 2.0));
        x -= 12.0;
    }
    for point in &points {
        ui.point_at(*point);
        ui.simulate(iced_test::simulator::click());
    }
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(messages.len(), points.len(), "{messages:?}");
    assert!(
        messages
            .iter()
            .all(|m| matches!(m, Message::TogglePanel(Panel::Control)))
    );
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
        "Haut-parleurs intégrés",
        "Équilibré",
        "Camille",
        "82\u{202F}% · 3 h 12 restantes",
    ] {
        assert!(
            ui.find(text).is_ok(),
            "{text} missing from the quick settings"
        );
    }
    // Nothing that does nothing (D-20f).
    for gone in ["Mode jeu", "Carte graphique", "Bientôt"] {
        assert!(ui.find(gone).is_err(), "{gone} still shown");
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
}

/// The battery of the bar at 5, 50 and 100 % on its own, charging at 54 %, plugged in and
/// full, and low at 12 %: `battery`, then `battery-light`.
#[test]
fn the_battery_fills_with_its_level_in_both_themes() {
    use bar::source::battery::LOW_PERCENT;
    use sherlock_ui::component as ui;
    use sherlock_ui::icon::BatteryMark;
    let states = [
        (5, BatteryMark::None),
        (50, BatteryMark::None),
        (100, BatteryMark::None),
        (54, BatteryMark::Charging),
        (100, BatteryMark::Plugged),
        (12, BatteryMark::None),
    ];
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let p = color::palette();
            let gauges = states.map(|(percent, mark)| {
                let low = mark == BatteryMark::None && percent < LOW_PERCENT;
                let tint = if low { p.amber_text } else { p.text_secondary };
                let value = Some(sherlock_ui::strings::percent(percent));
                ui::battery::<Message>(percent, mark, value, tint)
            });
            let line = iced::widget::Row::with_children(gauges)
                .spacing(metrics::STATUS_GAP)
                .padding(8);
            let back = iced::widget::container(line).style(sherlock_ui::theme::top_bar_shaped(
                0.0,
                metrics::TOP_BAR_RADIUS,
            ));
            let ui = Simulator::with_size(bar::settings(), (420.0, 36.0), back);
            assert_written(capture(ui, &format!("battery{suffix}")));
        });
    }
}

/// Every state of the live icons of the bar (UI-01), in each theme, `indicators` then
/// `indicators-light`: the Wi-Fi at 0 to 3 arcs, without the Internet, without a network, and
/// the four steps of a connection; the sound at 0 to 3 waves and muted; Bluetooth off, on and
/// connected.
#[test]
fn every_state_of_the_indicators_renders_in_both_themes() {
    use sherlock_ui::component as ui;
    use sherlock_ui::icon::{self, Radio, WifiMark};
    let wifi: Vec<String> = (0..=icon::LEVELS)
        .map(|level| icon::wifi_svg(level, WifiMark::None))
        .chain([
            icon::wifi_svg(2, WifiMark::NoInternet),
            icon::wifi_svg(0, WifiMark::Off),
        ])
        .collect();
    let sound: Vec<String> = (0..=icon::LEVELS)
        .map(|level| icon::sound_svg(level, false))
        .chain([icon::sound_svg(2, true)])
        .collect();
    let radio = [Radio::Off, Radio::On, Radio::Connected].map(icon::bluetooth_svg);
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let tint = color::palette().text_secondary;
            let line = |documents: &[String]| {
                let icons = documents
                    .iter()
                    .map(|d| ui::live_icon::<Message>(vec![d.clone()], tint));
                iced::widget::Row::with_children(icons).spacing(metrics::STATUS_GAP)
            };
            let all = iced::widget::column![line(&wifi), line(&sound), line(&radio)]
                .spacing(metrics::STATUS_GAP)
                .padding(10);
            let back = iced::widget::container(all).style(sherlock_ui::theme::top_bar_shaped(
                0.0,
                metrics::TOP_BAR_RADIUS,
            ));
            let ui = Simulator::with_size(bar::settings(), (220.0, 110.0), back);
            assert_written(capture(ui, &format!("indicators{suffix}")));
        });
    }
}

/// The Bluetooth devices of fictitious devices, in each theme: `bluetooth-devices` (« Casque »
/// connected at 80 %, « Souris » paired, « Clavier » found while the scan runs), then
/// `bluetooth-pairing` (« Clavier » pairing, the agent asking to confirm its code), and their
/// `-light` versions.
#[test]
fn the_bluetooth_devices_render_in_both_themes() {
    use bar::system::bluetooth::{Asked, Device, Kind, Pairing, Reply};
    let device = |path: &str, name: &str, kind, paired, connected, battery| Device {
        path: format!("/org/bluez/hci0/{path}"),
        name: String::from(name),
        kind,
        paired,
        connected,
        battery,
    };
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        in_theme(which, || {
            let mut state = sample();
            state.panel_margins = panel::margins(false);
            board_quick(&mut state);
            let _ = state
                .quick
                .update(quick::Message::Bluetooth(Load::Ready(Bluetooth {
                    adapter: String::from("/org/bluez/hci0"),
                    powered: true,
                    connected: 1,
                    devices: vec![
                        device("dev_1", "Casque", Kind::Audio, true, true, Some(80)),
                        device("dev_2", "Souris", Kind::Mouse, true, false, None),
                        device("dev_3", "Clavier", Kind::Keyboard, false, false, None),
                    ],
                })));
            let _ = state.quick.update(quick::Message::ShowDevices(true));
            state.quick.tick(state.quick.now + Duration::from_secs(5));
            let size = panel_size(&state, Panel::Control);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            assert_written(capture(ui, &format!("bluetooth-devices{suffix}")));

            let _ = state.quick.update(quick::Message::Device(String::from(
                "/org/bluez/hci0/dev_3",
            )));
            let (sender, _answer) = iced::futures::channel::oneshot::channel();
            let asked = Asked::Confirm(String::from("482913"));
            let event = Pairing::Ask(asked, Reply::new(sender));
            let _ = state.quick.update(quick::Message::Pairing(event));
            let mut ui =
                Simulator::with_size(bar::settings(), size, state.view_panel(Panel::Control));
            for text in [
                "MES APPAREILS",
                "Casque",
                "Connecté · 80\u{202F}%",
                "Clavier",
                "482913",
            ] {
                assert!(ui.find(text).is_ok(), "{text} missing from the devices");
            }
            assert_written(capture(ui, &format!("bluetooth-pairing{suffix}")));
        });
    }
}

/// Six steps of a closing, from 0 to `motion::POPUP_CLOSE`, side by side on the desktop's color:
/// the panel's last image shrunk and faded as the compositor draws it when a click elsewhere
/// closes it (lot UI-06, `compositor/src/popup_anim.rs`).
fn closing_steps(panel: &image::RgbaImage) -> image::RgbaImage {
    use sherlock_ui::motion;
    let (w, h) = panel.dimensions();
    let gap = 24;
    let desktop = color::palette().desktop;
    let mut out = image::RgbaImage::from_pixel(
        6 * w + 7 * gap,
        h + 2 * gap,
        image::Rgba([desktop.r, desktop.g, desktop.b, 255]),
    );
    let fade = motion::Fade::eased(
        1.0,
        0.0,
        Duration::ZERO,
        motion::POPUP_CLOSE,
        motion::WINDOW_CLOSE_EASE,
    );
    for step in 0..6u32 {
        let at = motion::POPUP_CLOSE * step / 5;
        let (scale, alpha) = motion::popup_closing(fade.value(at));
        let (sw, sh) = ((f64::from(w) * scale) as u32, (f64::from(h) * scale) as u32);
        let mut shrunk =
            image::imageops::resize(panel, sw.max(1), sh.max(1), image::imageops::Triangle);
        for pixel in shrunk.pixels_mut() {
            pixel.0[3] = (f32::from(pixel.0[3]) * alpha).round() as u8;
        }
        // Around its center, as the compositor scales it.
        let x = gap + step * (w + gap) + (w - sw) / 2;
        let y = gap + (h - sh) / 2;
        image::imageops::overlay(&mut out, &shrunk, i64::from(x), i64::from(y));
    }
    out
}

/// The control centre and the notification centre closing on a click elsewhere: six steps
/// each, `control-centre-closing` and `notifications-closing` in `screenshots/`.
#[test]
fn the_panels_close_with_their_last_image() {
    in_theme(Theme::Dark, || {
        let mut state = sample();
        state.panel_margins = panel::margins(false);
        state.month = Some(Month {
            year: 2026,
            month: 11,
        });
        state.history = board_history();
        board_quick(&mut state);
        for (which, name) in [
            (Panel::Control, "control-centre-closing"),
            (Panel::Calendar, "notifications-closing"),
        ] {
            let size = panel_size(&state, which);
            let ui = Simulator::with_size(bar::settings(), size, state.view_panel(which));
            let (path, _) = capture(ui, &format!("{name}-panel")).unwrap();
            let panel = image::open(&path).unwrap().to_rgba8();
            let steps = closing_steps(&panel);
            let out = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("screenshots")
                .join(format!("{name}.png"));
            steps.save(&out).unwrap();
            assert!(std::fs::metadata(&out).unwrap().len() > 10_000);
        }
    });
}
