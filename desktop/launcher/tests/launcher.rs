//! The launcher from the neutral fixtures: what it lists, how it ranks, the keyboard, the
//! start of a program, and its rendering without a screen.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::widget::container;
use iced::window::Id;
use iced::{Element, Event, Length, Point, keyboard};
use iced_test::simulator::Simulator;
use launcher::app::{self, Key, Launcher, Message};
use launcher::apps::Apps;
use launcher::source::{Action, Command, Source};
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
    let terms = launcher::search::terms(query);
    apps.search(&terms, 50)
        .into_iter()
        .map(|h| h.title)
        .collect()
}

/// A launcher on the fixtures whose starts are recorded instead of run.
fn recorded() -> (Launcher, Arc<Mutex<Vec<Command>>>) {
    let started = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&started);
    let spawn: app::Spawn = Arc::new(move |command: &Command| {
        record.lock().expect("record").push(command.clone());
        Ok(1)
    });
    (Launcher::new(vec![Box::new(apps())], spawn), started)
}

fn key(named: keyboard::key::Named) -> Message {
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
    app::on_event(event, iced::event::Status::Captured, Id::unique()).expect("a launcher key")
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
    let _ = launcher.update(key(keyboard::key::Named::Enter));
    let started = started.lock().expect("record");
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].program, "kitty");
    assert!(launcher.closing, "the launcher closes after a start");
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
fn arrows_choose_and_escape_closes() {
    let (mut launcher, started) = recorded();
    let _ = launcher.update(Message::Query("e".into()));
    let count = launcher.hits().count();
    assert!(count > 2);
    let _ = launcher.update(key(keyboard::key::Named::ArrowUp));
    assert_eq!(launcher.selected, 0, "the first stays first");
    let _ = launcher.update(key(keyboard::key::Named::ArrowDown));
    let _ = launcher.update(key(keyboard::key::Named::ArrowDown));
    assert_eq!(launcher.selected, 2);
    for _ in 0..count {
        let _ = launcher.update(key(keyboard::key::Named::ArrowDown));
    }
    assert_eq!(launcher.selected, count - 1, "the last stays last");
    let chosen = launcher.hits().nth(count - 1).cloned().expect("hit");
    let _ = launcher.update(key(keyboard::key::Named::Enter));
    let Action::Start(command) = chosen.action;
    assert_eq!(started.lock().expect("record").as_slice(), [command]);
    // A new query brings the choice back to the first result.
    let (mut other, started) = recorded();
    let _ = other.update(Message::Query("e".into()));
    let _ = other.update(key(keyboard::key::Named::ArrowDown));
    let _ = other.update(Message::Query("ed".into()));
    assert_eq!(other.selected, 0);
    let _ = other.update(key(keyboard::key::Named::Escape));
    assert!(other.closing);
    assert!(started.lock().expect("record").is_empty());
}

#[test]
fn a_click_starts_its_result_and_the_working_directory_is_kept() {
    let (mut launcher, started) = recorded();
    let _ = launcher.update(Message::Query("calc".into()));
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
    assert!(launcher::launch::spawn(&missing).is_err());
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
    for query in ["e", "ki", "edit web", "numero 42", "zzz", "o t l"] {
        let start = Instant::now();
        let terms = launcher::search::terms(query);
        let _ = apps.search(&terms, look::MAX_RESULTS);
        let spent = start.elapsed();
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

/// Writes the screenshot to `target/tmp/<name>-tiny-skia.png`, returns its size in bytes.
fn capture(launcher: &Launcher, name: &str) -> u64 {
    let size = (look::WIDTH, launcher.height());
    let mut ui = Simulator::with_size(
        launcher::settings(),
        size,
        on_desktop(launcher.view_panel()),
    );
    let snapshot = ui.snapshot(&theme::theme()).expect("snapshot");
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path).expect("written");
    std::fs::metadata(&written).expect("screenshot").len()
}

#[test]
fn the_panel_renders_its_results() {
    let (mut launcher, _) = recorded();
    assert_eq!(launcher.height(), look::panel_height(0, 0));
    assert!(capture(&launcher, "launcher-empty") > 2_000);
    let _ = launcher.update(Message::Query("te".into()));
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
    assert!(capture(&launcher, "launcher-results") > 2_000);
    let _ = launcher.update(Message::Query("zzz".into()));
    assert_eq!(launcher.height(), look::panel_height(0, 1));
    let mut ui = Simulator::with_size(
        launcher::settings(),
        (look::WIDTH, launcher.height()),
        launcher.view_panel(),
    );
    assert!(ui.find(texts::NOTHING).is_ok());
    drop(ui);
    assert!(capture(&launcher, "launcher-nothing") > 2_000);
    // The surface follows the panel, at the board's place.
    let surface = launcher::surface(launcher.height());
    assert_eq!(surface.margin.top, look::TOP as i32);
    assert_eq!(surface.exclusive_zone, -1);
}
