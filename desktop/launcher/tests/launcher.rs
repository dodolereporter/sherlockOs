//! The launcher from the neutral fixtures: what it lists, how it ranks, the keyboard, the
//! start of a program, and its rendering without a screen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::widget::container;
use iced::window::Id;
use iced::{Element, Event, Length, Point, keyboard};
use iced_test::simulator::Simulator;
use launcher::app::{self, Launcher, Message};
use launcher::apps::Apps;
use launcher::source::{Action, Command, Query, Source};
use mime_apps::{DesktopEntry, Dirs};
use sherlock_ui::launcher::{self as look, text as texts};
use sherlock_ui::theme;

fn fixtures() -> Dirs {
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data");
    Dirs {
        config_home: data.join("config"),
        config_dirs: Vec::new(),
        data_home: data,
        data_dirs: Vec::new(),
    }
}

fn terminal() -> Option<Vec<std::ffi::OsString>> {
    Some(vec!["test-terminal".into(), "-e".into()])
}

fn apps() -> Apps {
    Apps::load(&fixtures(), &["fr_FR".into(), "fr".into()], terminal())
}

fn titles(apps: &Apps, query: &str) -> Vec<String> {
    apps.search(&Query::new(query), 50)
        .into_iter()
        .map(|h| h.title)
        .collect()
}

/// A launcher on the fixtures whose starts are recorded instead of run.
fn recorded() -> (Launcher, Arc<Mutex<Vec<Command>>>) {
    let started = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&started);
    let spawn: app::Spawn = Arc::new(move |command: &Command, _: Option<&str>| {
        if let Ok(mut record) = record.lock() {
            record.push(command.clone());
        }
        Ok(1)
    });
    (Launcher::new(vec![Box::new(apps())], spawn), started)
}

/// The message a key press gives, when the launcher acts on it.
fn key(named: keyboard::key::Named) -> Option<Message> {
    let event = Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(named),
        modified_key: keyboard::Key::Named(named),
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    });
    // The search field takes Escape: the launcher still sees it.
    app::on_event(event, iced::event::Status::Captured, Id::unique())
}

#[test]
fn hidden_deleted_and_missing_applications_are_left_out() {
    let all = titles(&apps(), "k");
    for name in ["Kitty", "Konsole", "Kate", "KWrite"] {
        assert!(all.iter().any(|t| t == name), "{name} missing from {all:?}");
    }
    // NoDisplay, Hidden, TryExec missing.
    for name in ["Kiosk helper", "Kiwi deleted", "Kilo missing"] {
        assert!(!all.iter().any(|t| t == name), "{name} listed");
    }
    // A terminal application needs a terminal.
    assert_eq!(titles(&apps(), "top"), ["Top"]);
    let without = Apps::load(&fixtures(), &[], None);
    assert!(titles(&without, "top").is_empty());
    // The names are localized, and found in a subfolder (`kde/`).
    assert_eq!(titles(&apps(), "calculatrice"), ["Calculatrice"]);
    assert_eq!(titles(&apps(), "konsole"), ["Konsole"]);
}

#[test]
fn the_ranking_ignores_case_accents_and_file_order() {
    let apps = apps();
    assert_eq!(titles(&apps, "ki")[0], "Kitty");
    assert_eq!(titles(&apps, "KI")[0], "Kitty");
    // By generic name, accents or not: equal scores go by name.
    assert_eq!(titles(&apps, "editeur"), ["Kate", "KWrite"]);
    assert_eq!(titles(&apps, "ÉDITEUR"), ["Kate", "KWrite"]);
    assert_eq!(titles(&apps, "navigateur"), ["Firefox"]);
    assert_eq!(titles(&apps, "web"), ["Firefox"]);
    // The order does not depend on the order the files were read in.
    let mut entries = mime_apps::read_applications(&fixtures(), &[]);
    let forward = titles(&Apps::new(entries.clone(), terminal()), "e");
    entries.reverse();
    assert_eq!(titles(&Apps::new(entries, terminal()), "e"), forward);
    assert!(forward.len() > 3, "{forward:?}");
}

#[test]
fn the_first_key_already_filters() {
    let (mut launcher, _) = recorded();
    assert_eq!(launcher.hits().count(), 0, "nothing before a key");
    let _ = launcher.update(Message::Query("f".into()));
    let first: Vec<&str> = launcher.hits().map(|h| h.title.as_str()).collect();
    assert_eq!(first[0], "Firefox");
    assert!(first.len() < apps().len());
}

/// What typing `letter` into the search field of `launcher`, as drawn now, sends.
fn type_into_field(launcher: &Launcher, letter: &str) -> Vec<Message> {
    let mut ui = Simulator::with_size(
        launcher::settings(),
        (look::WIDTH, launcher.height()),
        launcher.view_panel(),
    );
    let field = Point::new(look::WIDTH / 2.0, look::PADDING + look::SEARCH_HEIGHT / 2.0);
    ui.point_at(field);
    let _ = ui.simulate(iced_test::simulator::click());
    let _ = ui.typewrite(letter);
    ui.into_messages().collect()
}

#[test]
fn kitty_starts_with_three_keys() {
    let (mut launcher, started) = recorded();
    // Two letters typed into the search field, then Enter.
    for letter in ["k", "i"] {
        let typed = type_into_field(&launcher, letter);
        assert_eq!(typed.len(), 1, "one message per key: {typed:?}");
        for message in typed {
            let _ = launcher.update(message);
        }
    }
    assert_eq!(launcher.query, "ki");
    // Third key: Enter.
    let _ = launcher.update(key(keyboard::key::Named::Enter).expect("a launcher key"));
    let programs = |started: &Mutex<Vec<Command>>| -> Vec<std::ffi::OsString> {
        let started = started.lock().expect("record");
        started.iter().map(|c| c.program.clone()).collect()
    };
    assert_eq!(programs(&started), ["kitty"]);
    assert!(launcher.closing, "the launcher closes after a start");
    // A second Enter, or a click, before the exit is handled starts nothing more.
    let _ = launcher.update(key(keyboard::key::Named::Enter).expect("a launcher key"));
    let _ = launcher.update(Message::Pick(0));
    assert_eq!(programs(&started), ["kitty"]);
}

/// Typing « fir » in one interface kept from frame to frame, as the running launcher does: the
/// first letter brings the results, the field keeps the keyboard for the next ones (lot Q-45).
#[test]
fn the_field_keeps_the_keyboard_when_the_results_come() {
    use iced_test::core::renderer::Headless as _;
    use iced_test::core::{Font, Size, clipboard, mouse};
    use iced_test::runtime::user_interface::{Cache, UserInterface};

    let (mut launcher, _) = recorded();
    let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
        Font::with_name("Fira Sans"),
        iced::Pixels(16.0),
        None,
    ))
    .expect("a headless renderer");
    let field = Point::new(look::WIDTH / 2.0, look::PADDING + look::SEARCH_HEIGHT / 2.0);
    let cursor = mouse::Cursor::Available(field);
    let mut cache = Cache::default();
    for (n, letter) in ["f", "i", "r"].into_iter().enumerate() {
        let size = Size::new(look::WIDTH, launcher.height());
        let mut ui = UserInterface::build(launcher.view_panel(), size, cache, &mut renderer);
        let mut events: Vec<Event> = Vec::new();
        if n == 0 {
            events.extend(iced_test::simulator::click());
        }
        events.extend(iced_test::simulator::typewrite(letter));
        let mut messages = Vec::new();
        let _ = ui.update(
            &events,
            cursor,
            &mut renderer,
            &mut clipboard::Null,
            &mut messages,
        );
        cache = ui.into_cache();
        for message in messages {
            let _ = launcher.update(message);
        }
    }
    assert_eq!(launcher.query, "fir");
}

#[test]
fn every_application_starts_with_two_letters_and_enter() {
    let apps = apps();
    assert!(titles(&apps, "").is_empty(), "no query, no result");
    let mut names: Vec<String> = mime_apps::read_applications(&fixtures(), &["fr".into()])
        .into_iter()
        .filter(|a| !a.no_display)
        .map(|a| a.name)
        .collect();
    names.sort();
    for name in names {
        let reached = name.chars().collect::<Vec<_>>().windows(2).any(|pair| {
            let query: String = pair.iter().collect();
            titles(&apps, &query).first() == Some(&name)
        });
        assert!(reached, "{name} needs more than two letters");
    }
}

#[test]
fn only_the_launcher_keys_are_taken() {
    assert!(key(keyboard::key::Named::Tab).is_none());
    assert!(matches!(
        key(keyboard::key::Named::Escape),
        Some(Message::Key(app::Key::Escape))
    ));
}

#[test]
fn arrows_choose_and_escape_closes() {
    let (mut launcher, started) = recorded();
    let _ = launcher.update(Message::Query("e".into()));
    let count = launcher.hits().count();
    assert!(count > 2);
    let _ = launcher.update(key(keyboard::key::Named::ArrowUp).expect("a launcher key"));
    assert_eq!(launcher.selected, 0, "the first stays first");
    let _ = launcher.update(key(keyboard::key::Named::ArrowDown).expect("a launcher key"));
    let _ = launcher.update(key(keyboard::key::Named::ArrowDown).expect("a launcher key"));
    assert_eq!(launcher.selected, 2);
    for _ in 0..count {
        let _ = launcher.update(key(keyboard::key::Named::ArrowDown).expect("a launcher key"));
    }
    assert_eq!(launcher.selected, count - 1, "the last stays last");
    let chosen = launcher.hits().nth(count - 1).cloned().expect("hit");
    let _ = launcher.update(key(keyboard::key::Named::Enter).expect("a launcher key"));
    let Action::Start(command) = chosen.action else {
        panic!("an application starts");
    };
    assert_eq!(started.lock().expect("record").as_slice(), [command]);
    // A new query brings the choice back to the first result.
    let (mut other, started) = recorded();
    let _ = other.update(Message::Query("e".into()));
    let _ = other.update(key(keyboard::key::Named::ArrowDown).expect("a launcher key"));
    let _ = other.update(Message::Query("ed".into()));
    assert_eq!(other.selected, 0);
    let _ = other.update(key(keyboard::key::Named::Escape).expect("a launcher key"));
    assert!(other.closing);
    assert!(started.lock().expect("record").is_empty());
}

#[test]
fn a_click_starts_its_result_and_the_working_directory_is_kept() {
    let (mut launcher, started) = recorded();
    let _ = launcher.update(Message::Query("calc".into()));
    settled(&mut launcher);
    let mut ui = Simulator::with_size(
        launcher::settings(),
        (look::WIDTH, launcher.height()),
        launcher.view_panel(),
    );
    ui.click("Calculatrice").expect("result shown");
    for message in ui.into_messages().collect::<Vec<_>>() {
        let _ = launcher.update(message);
    }
    let started = started.lock().expect("record");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].program, "example-calculator");
    assert_eq!(started[0].dir, Some(PathBuf::from("/tmp")));
}

#[test]
fn a_program_starts_without_a_shell_in_its_directory() {
    let dir = std::env::temp_dir().join(format!("launcher-start-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("folder");
    // A shell would run the second command; without one, `touch` gets one odd file name.
    let command = Command {
        program: "touch".into(),
        args: vec!["started; touch injected".into()],
        dir: Some(dir.clone()),
    };
    launcher::launch::spawn(&command).expect("started");
    let marker = dir.join("started; touch injected");
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(marker.exists());
    assert_eq!(std::fs::read_dir(&dir).expect("listed").count(), 1);
    let missing = Command {
        program: "/nonexistent/program".into(),
        args: Vec::new(),
        dir: None,
    };
    assert!(matches!(
        launcher::launch::spawn(&missing),
        Err(launcher::launch::LaunchError::Spawn { .. })
    ));
    // A missing working directory is told apart from a missing program.
    let nowhere = Command {
        program: "touch".into(),
        args: vec!["x".into()],
        dir: Some(dir.join("gone")),
    };
    let error = launcher::launch::spawn(&nowhere).expect_err("no folder");
    assert!(matches!(
        error,
        launcher::launch::LaunchError::MissingDir { .. }
    ));
    assert!(error.to_string().contains("working directory"), "{error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn searching_500_applications_takes_less_than_50_ms() {
    let words = [
        "text", "web", "image", "music", "video", "code", "mail", "game",
    ];
    let entries: Vec<DesktopEntry> = (0..500)
        .map(|i| {
            let word = words[i % words.len()];
            let text = format!(
                "[Desktop Entry]\nType=Application\nName=Éditeur {word} {i}\n\
                 GenericName=Outil {word}\nKeywords={word};numéro{i};\nExec=app{i} %U\n"
            );
            let path = PathBuf::from(format!("/apps/app{i}.desktop"));
            mime_apps::entry::parse(&format!("app{i}.desktop"), &path, &text, &[]).expect("entry")
        })
        .collect();
    let apps = Apps::new(entries, None);
    assert_eq!(apps.len(), 500);
    // The best of 5 runs: a loaded machine can pause the test thread during one run, which
    // says nothing about the cost of the search. The threshold stays the same in every build.
    for query in ["e", "ki", "edit web", "numero 42", "zzz", "o t l"] {
        let spent = (0..5)
            .map(|_| {
                let start = Instant::now();
                let _ = apps.search(&Query::new(query), look::MAX_RESULTS);
                start.elapsed()
            })
            .min()
            .unwrap_or(Duration::MAX);
        assert!(spent < Duration::from_millis(50), "{query}: {spent:?}");
    }
}

/// The launcher on the desktop background.
fn on_desktop(element: Element<'_, Message>) -> Element<'_, Message> {
    container(element)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::desktop)
        .into()
}

/// Lets the rows, the panel and the selection reach their place (lot P-02d).
fn settled(launcher: &mut Launcher) {
    let _ = launcher.update(Message::Frame(Instant::now() + Duration::from_secs(5)));
}

/// Writes the screenshot to `target/tmp/<name>-tiny-skia.png`, returns its size in bytes.
fn capture(launcher: &Launcher, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let size = (look::WIDTH, launcher.height());
    let mut ui = Simulator::with_size(
        launcher::settings(),
        size,
        on_desktop(launcher.view_panel()),
    );
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    Ok(std::fs::metadata(&written)?.len())
}

#[test]
fn the_panel_renders_its_results() {
    let (mut launcher, _) = recorded();
    assert_eq!(launcher.height(), look::panel_height(0, 0));
    assert!(capture(&launcher, "launcher-empty").expect("screenshot written") > 2_000);
    let _ = launcher.update(Message::Query("te".into()));
    settled(&mut launcher);
    let rows = launcher.hits().count();
    assert!(rows > 1);
    assert_eq!(launcher.height(), look::panel_height(1, rows));
    let mut ui = Simulator::with_size(
        launcher::settings(),
        (look::WIDTH, launcher.height()),
        launcher.view_panel(),
    );
    for text in [texts::ESCAPE, texts::ENTER, "APPLICATIONS", "Kitty"] {
        assert!(ui.find(text).is_ok(), "{text} missing");
    }
    drop(ui);
    assert!(capture(&launcher, "launcher-results").expect("screenshot written") > 2_000);
    let _ = launcher.update(Message::Query("zzz".into()));
    settled(&mut launcher);
    assert_eq!(launcher.height(), look::panel_height(0, 1));
    let mut ui = Simulator::with_size(
        launcher::settings(),
        (look::WIDTH, launcher.height()),
        launcher.view_panel(),
    );
    assert!(ui.find(texts::NOTHING).is_ok());
    drop(ui);
    assert!(capture(&launcher, "launcher-nothing").expect("screenshot written") > 2_000);
    // The surface follows the panel, at the board's place.
    let surface = launcher::surface(launcher.height());
    assert_eq!(surface.margin.top, look::TOP as i32);
    assert_eq!(surface.exclusive_zone, -1);
}

#[test]
fn the_surface_is_resized_when_the_rows_change() {
    let (mut launcher, _) = recorded();
    let _ = launcher.update(Message::Surface(Id::unique()));
    // From nothing to « Aucun résultat »: one row more, a resize.
    assert!(launcher.update(Message::Query("zzz".into())).units() > 0);
    // Still nothing: the same height, no resize.
    assert_eq!(launcher.update(Message::Query("zzzz".into())).units(), 0);
    assert!(launcher.update(Message::Query("k".into())).units() > 0);
}

#[test]
fn a_right_click_keeps_an_application_in_the_dock() {
    let dir = std::env::temp_dir().join(format!("launcher-dock-{}", std::process::id()));
    let path = dir.join("dock/dock.conf");
    std::fs::create_dir_all(dir.join("dock")).expect("folder");
    std::fs::write(&path, "[Dock]\nPinned=firefox;\n").expect("dock file");
    let (launcher, _) = recorded();
    let mut launcher = launcher.with_dock_file(path.clone());
    let _ = launcher.update(Message::Query("kit".into()));
    settled(&mut launcher);
    let _ = launcher.update(Message::Menu(0));
    assert_eq!(launcher.menu, Some((0, false)));
    let mut ui = Simulator::with_size(
        launcher::settings(),
        (look::WIDTH, launcher.height()),
        launcher.view_panel(),
    );
    assert!(ui.find(sherlock_ui::dock::text::KEEP).is_ok());
    drop(ui);
    assert!(capture(&launcher, "launcher-menu").expect("screenshot written") > 2_000);
    let _ = launcher.update(Message::Pin);
    let text = std::fs::read_to_string(&path).expect("dock file");
    assert_eq!(text, "[Dock]\nPinned=firefox;kitty;\n");
    assert_eq!(launcher.menu, None);
    // Pinned: « Retirer du Dock »; Escape closes the menu, not the launcher.
    let _ = launcher.update(Message::Menu(0));
    assert_eq!(launcher.menu, Some((0, true)));
    let _ = launcher.update(Message::Key(app::Key::Escape));
    assert_eq!(launcher.menu, None);
    assert!(!launcher.closing);
    // Without its menu, nothing is pinned.
    let _ = launcher.update(Message::Pin);
    assert_eq!(std::fs::read_to_string(&path).expect("dock file"), text);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_results_slide_into_place_and_the_panel_follows() {
    let (mut launcher, _) = recorded();
    let _ = launcher.update(Message::Surface(Id::unique()));
    let base = Instant::now();
    let at = |ms: u64| Message::Frame(base + Duration::from_millis(ms));
    let _ = launcher.update(at(2_000));
    assert!(!launcher.animating(), "open: nothing moves");
    // The first letter: the rows fade in, the panel grows to them.
    let _ = launcher.update(Message::Query("e".into()));
    assert!(launcher.animating());
    let target = launcher.height();
    assert_eq!(launcher.drawn_height(), look::panel_height(0, 0));
    let _ = launcher.update(at(2_060));
    let growing = launcher.drawn_height();
    assert!(
        growing > look::panel_height(0, 0) && growing < target,
        "{growing}"
    );
    let _ = launcher.update(at(4_000));
    assert!(!launcher.animating(), "at rest: no frame");
    assert!((launcher.drawn_height() - target).abs() < 1.0);
    // The selection slides to the next row.
    let _ = launcher.update(key(keyboard::key::Named::ArrowDown).expect("a launcher key"));
    assert!(launcher.animating());
    let _ = launcher.update(at(6_000));
    assert!(!launcher.animating());
    // Fewer results: the panel shrinks gently, never at once.
    let _ = launcher.update(Message::Query("kit".into()));
    assert!(launcher.height() < target);
    assert!(launcher.drawn_height() > launcher.height());
    assert!(launcher.animating());
    let _ = launcher.update(at(8_000));
    assert!(!launcher.animating());
    assert!((launcher.drawn_height() - launcher.height()).abs() < 1.0);
    // A new opening starts at rest, on an empty search.
    let _ = launcher.update(key(keyboard::key::Named::Escape).expect("a launcher key"));
    let _ = launcher.update(at(10_000));
}

/// The board of the launcher (lot P-02d): for each theme, a row for the opening, then for each
/// of « e », « ed » typed and an arrow down, frames 0, 40, 80 and 240 ms after, in
/// `target/tmp/launcher-board.png`.
#[test]
#[ignore = "slow: cargo test -p launcher --test launcher -- --ignored board"]
fn the_typing_board() {
    use image::{GenericImage as _, RgbaImage, imageops};
    use sherlock_ui::color::{self, Theme};
    const TIMES: [u64; 4] = [0, 40, 80, 240];
    const SCREEN: (f32, f32) = (760.0, 600.0);
    const SIDE: (u32, u32) = (380, 300);
    const GAP: u32 = 12;
    let steps: [Option<Message>; 4] = [
        None,
        Some(Message::Query("e".into())),
        Some(Message::Query("ed".into())),
        key(keyboard::key::Named::ArrowDown),
    ];
    let rows = 2 * steps.len() as u32;
    let mut board = RgbaImage::from_pixel(
        TIMES.len() as u32 * (SIDE.0 + GAP) + GAP,
        rows * (SIDE.1 + GAP) + GAP,
        image::Rgba([40, 44, 52, 255]),
    );
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let mut line = 0;
    for (which, name) in [(Theme::Dark, "dark"), (Theme::Light, "light")] {
        color::set_active(which);
        let (mut launcher, _) = recorded();
        let base = Instant::now();
        let _ = launcher.update(Message::Surface(Id::unique()));
        let mut start = Duration::ZERO;
        for (step, message) in steps.iter().enumerate() {
            if let Some(message) = message {
                // Past the time the captures took: the launcher's clock never goes back to an
                // earlier moment than the real one.
                start = base.elapsed() + Duration::from_secs(1);
                let _ = launcher.update(Message::Frame(base + start));
                let _ = launcher.update(message.clone());
            }
            for (column, ms) in TIMES.iter().enumerate() {
                let _ = launcher.update(Message::Frame(base + start + Duration::from_millis(*ms)));
                let mut ui = Simulator::with_size(
                    launcher::settings(),
                    SCREEN,
                    on_desktop(launcher.view_screen()),
                );
                let snapshot = ui.snapshot(&theme::theme()).expect("snapshot");
                let frame = format!("launcher-board-{name}-{step}-{column}");
                let path = tmp.join(format!("{frame}.png"));
                let written = tmp.join(format!("{frame}-tiny-skia.png"));
                let _ = std::fs::remove_file(&written);
                let _ = snapshot.matches_image(&path);
                let shot = image::open(&written).expect("frame").to_rgba8();
                let small = imageops::resize(&shot, SIDE.0, SIDE.1, imageops::FilterType::Triangle);
                let x = GAP + column as u32 * (SIDE.0 + GAP);
                let y = GAP + line * (SIDE.1 + GAP);
                board.copy_from(&small, x, y).expect("copied");
            }
            line += 1;
        }
    }
    color::set_active(Theme::Dark);
    board.save(tmp.join("launcher-board.png")).expect("board");
}
