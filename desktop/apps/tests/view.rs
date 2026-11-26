//! The applications view on neutral fixtures: what it lists, the search, the pages, the
//! keyboard, starting an application, the frames it asks for, and its rendering without a
//! screen in both themes.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use apps::app::{Key, Message, View};
use apps::catalog::Catalog;
use iced::mouse::ScrollDelta;
use iced::widget::container;
use iced::{Element, Length, Point, Size};
use iced_test::simulator::Simulator;
use launcher::apps::Apps;
use launcher::source::Command;
use mime_apps::{DesktopEntry, Dirs};
use sherlock_ui::apps::{self as look, text as texts};
use sherlock_ui::color::{self, Theme};
use sherlock_ui::theme;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// The user's folder first, then the system's: the same desktop id is read once, from the
/// user's folder.
fn dirs() -> Dirs {
    Dirs {
        config_home: fixtures().join("config"),
        config_dirs: Vec::new(),
        data_home: fixtures().join("home"),
        data_dirs: vec![fixtures().join("system")],
    }
}

fn terminal() -> Option<Vec<OsString>> {
    Some(vec!["test-terminal".into(), "-e".into()])
}

/// A `hicolor` theme holding the desktop's own application icons (`sherlock-ui/icons`).
fn icons() -> icon_theme::Lookup {
    let data = std::env::temp_dir().join(format!("apps-icons-{}", std::process::id()));
    if let Err(error) = write_icons(&data) {
        panic!("cannot copy the icons: {error}");
    }
    icon_theme::Lookup::new(&[data], None, None)
}

fn write_icons(data: &std::path::Path) -> std::io::Result<()> {
    let theme = data.join("icons/hicolor");
    let apps = theme.join("scalable/apps");
    std::fs::create_dir_all(&apps)?;
    std::fs::write(
        theme.join("index.theme"),
        "[Icon Theme]\nName=Hicolor\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=128\n\
         MinSize=1\nMaxSize=512\nContext=Applications\nType=Scalable\n",
    )?;
    let ours = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../sherlock-ui/icons/hicolor/scalable/apps");
    for file in std::fs::read_dir(ours)?.flatten() {
        std::fs::copy(file.path(), apps.join(file.file_name()))?;
    }
    Ok(())
}

fn catalog() -> Catalog {
    let apps = Apps::load(&dirs(), &["fr_FR".into(), "fr".into()], terminal())
        .with_icons(icons())
        .with_icon_size(look::ICON_LOOKUP, look::ICON_SCALE);
    Catalog::new(apps)
}

/// `count` applications named « Application 001 » and on.
fn many(count: usize) -> Catalog {
    let entries: Vec<DesktopEntry> = (0..count)
        .filter_map(|i| {
            let text = format!(
                "[Desktop Entry]\nType=Application\nName=Application {i:03}\nExec=app{i}\n"
            );
            let path = PathBuf::from(format!("/apps/app{i}.desktop"));
            mime_apps::entry::parse(&format!("app{i}.desktop"), &path, &text, &[])
        })
        .collect();
    assert_eq!(entries.len(), count);
    Catalog::new(Apps::new(entries, None))
}

type Started = Arc<Mutex<Vec<Command>>>;

/// A view whose starts are recorded instead of run.
fn recorded(catalog: Catalog) -> (View, Started) {
    let started: Started = Arc::default();
    let record = Arc::clone(&started);
    let spawn: launcher::app::Spawn = Arc::new(move |command: &Command| {
        if let Ok(mut record) = record.lock() {
            record.push(command.clone());
        }
        Ok(1)
    });
    (View::new(catalog, spawn), started)
}

fn names(view: &View) -> Vec<&str> {
    view.tiles().map(|t| t.name()).collect()
}

fn later(seconds: u64) -> Message {
    Message::Frame(Instant::now() + Duration::from_secs(seconds))
}

#[test]
fn every_application_once_by_name_in_its_language() {
    let catalog = catalog();
    let all: Vec<&str> = catalog.tiles().iter().map(|t| t.name()).collect();
    assert_eq!(
        all,
        [
            "Calculatrice",
            "Control Room",
            "Éditeur de texte",
            "Evidence",
            "Firefox",
            "Gestionnaire de paquets et de mises à jour",
            "Inquiry",
            "Installeur",
            "Jeux",
            "Kitty",
            "Konsole",
            "Labo",
            "Top",
        ]
    );
    // The user's entry hides the system's one of the same id, even when it is `Hidden`.
    let editor = catalog
        .tiles()
        .iter()
        .find(|t| t.id() == "org.example.Editor")
        .expect("editor");
    assert_eq!(editor.command.program, OsString::from("example-editor"));
    assert!(!all.contains(&"Old editor") && !all.contains(&"Removed app"));
    // In a subfolder, its id takes the folder.
    assert!(
        catalog
            .tiles()
            .iter()
            .any(|t| t.id() == "kde-org.kde.konsole")
    );
    // English without a French name; a terminal application without a terminal is left out.
    let english = Catalog::load(&dirs(), &[], None);
    let names: Vec<&str> = english.tiles().iter().map(|t| t.name()).collect();
    assert!(names.contains(&"Calculator") && names.contains(&"Editor"));
    assert!(!names.contains(&"Top"));
    // A terminal application runs in the terminal; the icons come from their theme.
    let top = catalog
        .tiles()
        .iter()
        .find(|t| t.name() == "Top")
        .expect("top");
    assert_eq!(top.command.program, OsString::from("test-terminal"));
    let evidence = &catalog.tiles()[3];
    assert!(
        catalog
            .icon(evidence)
            .is_some_and(|p| p.ends_with("evidence.svg"))
    );
    assert_eq!(catalog.icon(&catalog.tiles()[4]), None, "Firefox has none");
}

#[test]
fn the_search_filters_as_the_launcher_does() {
    let (mut view, _) = recorded(catalog());
    assert_eq!(view.shown.len(), 13);
    assert_eq!(view.selected, None, "nothing selected before a key");
    let _ = view.update(Message::Query("calc".into()));
    assert_eq!(names(&view), ["Calculatrice"]);
    assert_eq!(view.selected, Some(0), "the first one is selected");
    // Accents, generic names, scattered letters.
    let _ = view.update(Message::Query("EDIT".into()));
    assert_eq!(names(&view)[0], "Éditeur de texte");
    let _ = view.update(Message::Query("navigateur".into()));
    assert_eq!(names(&view), ["Firefox"]);
    let _ = view.update(Message::Query("ffx".into()));
    assert_eq!(names(&view), ["Firefox"]);
    let _ = view.update(Message::Query("zzz".into()));
    assert!(view.shown.is_empty());
    assert_eq!(view.selected, None);
    // Escape empties the search, then closes.
    let _ = view.update(Message::Key(Key::Escape));
    assert_eq!((view.query.as_str(), view.shown.len()), ("", 13));
    assert!(!view.closing);
    let _ = view.update(Message::Key(Key::Escape));
    assert!(view.closing);
}

#[test]
fn the_pages_hold_seven_by_five() {
    let (mut view, _) = recorded(many(80));
    assert_eq!(view.pager.count(), 3);
    // The arrows: the first one selects, Right goes on to the next page.
    let _ = view.update(Message::Key(Key::Down));
    assert_eq!(view.selected, Some(0));
    for _ in 0..34 {
        let _ = view.update(Message::Key(Key::Right));
    }
    assert_eq!((view.selected, view.pager.page()), (Some(34), 0));
    // Down stays on the page.
    let _ = view.update(Message::Key(Key::Down));
    assert_eq!(view.selected, Some(34));
    let _ = view.update(Message::Key(Key::Right));
    assert_eq!((view.selected, view.pager.page()), (Some(35), 1));
    let _ = view.update(Message::Key(Key::Up));
    assert_eq!(view.selected, Some(35), "first row of its page");
    // A wheel notch, a dot.
    let _ = view.update(Message::Scroll(ScrollDelta::Lines { x: 0.0, y: -1.0 }));
    assert_eq!(view.pager.page(), 2);
    let _ = view.update(Message::Page(0));
    assert_eq!(view.pager.page(), 0);
    // A smaller screen: six by four.
    let _ = view.update(Message::Resized(Size::new(800.0, 600.0)));
    assert_eq!((view.pager.count(), view.pager.page()), (4, 0));
    // A search starts on its first page.
    let _ = view.update(Message::Page(3));
    let _ = view.update(Message::Query("application 07".into()));
    assert_eq!((view.pager.count(), view.pager.page()), (1, 0));
}

#[test]
fn enter_starts_the_first_result_once() {
    let (mut view, started) = recorded(catalog());
    // Enter before any key starts nothing.
    let _ = view.update(Message::Key(Key::Enter));
    assert!(started.lock().expect("record").is_empty());
    let _ = view.update(Message::Query("kit".into()));
    let _ = view.update(Message::Key(Key::Enter));
    let _ = view.update(Message::Key(Key::Enter));
    let _ = view.update(Message::Launch(0));
    let started = started.lock().expect("record").clone();
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0].program, OsString::from("kitty"));
    assert!(view.closing);
    assert_eq!(view.launched, Some(0));
}

#[test]
fn a_click_starts_a_tile_and_a_click_beside_closes() {
    for (width, height) in SCREENS {
        let (mut view, _) = recorded(catalog());
        let _ = view.update(Message::Resized(Size::new(width, height)));
        let view = view.opened();
        let layout = view.layout();
        let grid = (
            layout.columns as f32 * layout.cell.0,
            layout.rows as f32 * layout.cell.1,
        );
        let left = (width - grid.0) / 2.0;
        let tile = Point::new(
            left + layout.cell.0 * 1.5,
            layout.grid_top + layout.cell.1 / 2.0,
        );
        let mut ui =
            Simulator::with_size(launcher::settings(), (width, height), view.view_screen());
        ui.point_at(tile);
        let _ = ui.simulate(iced_test::simulator::click());
        ui.point_at(Point::new(20.0, layout.grid_top + grid.1 + 10.0));
        let _ = ui.simulate(iced_test::simulator::click());
        let messages: Vec<Message> = ui.into_messages().collect();
        assert!(
            matches!(messages.as_slice(), [Message::Launch(1), Message::Dismiss]),
            "{width} × {height}: {messages:?}"
        );
    }
}

#[test]
fn nothing_is_drawn_at_rest() {
    let (mut view, _) = recorded(many(80));
    assert!(view.animating(), "it opens");
    let _ = view.update(later(2));
    assert!(!view.animating(), "open: no frame asked for");
    assert!((view.presence() - 1.0).abs() < look::OPEN_PRECISION);
    // Typing, arrows and resizes draw once, and ask for no frame.
    let _ = view.update(Message::Query("app".into()));
    let _ = view.update(Message::Key(Key::Right));
    assert!(!view.animating());
    // A page turn moves until it settles.
    let _ = view.update(Message::Scroll(ScrollDelta::Lines { x: 0.0, y: -1.0 }));
    assert!(view.animating());
    let _ = view.update(later(4));
    assert!(!view.animating());
    // Closing fades out, and the last frame ends the program.
    let _ = view.update(Message::Dismiss);
    assert!(view.animating());
    assert!(view.update(later(6)).units() > 0, "exit");
}

/// A capture sets the active theme: one at a time.
static THEME: Mutex<()> = Mutex::new(());

fn in_theme(which: Theme) -> MutexGuard<'static, ()> {
    let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(which);
    guard
}

/// The view over the desktop background.
fn on_desktop(element: Element<'_, Message>) -> Element<'_, Message> {
    container(element)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::desktop)
        .into()
}

/// Writes the screenshot to `target/tmp/<name>-tiny-skia.png`, returns its size in bytes.
fn capture(view: &View, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let size = (view.size.width, view.size.height);
    let mut ui = Simulator::with_size(launcher::settings(), size, on_desktop(view.view_screen()));
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    Ok(std::fs::metadata(&written)?.len())
}

fn shows(view: &View, text: &str) -> bool {
    let size = (view.size.width, view.size.height);
    let mut ui = Simulator::with_size(launcher::settings(), size, view.view_screen());
    ui.find(text).is_ok()
}

/// The screens of the captures, in logical pixels: 1366 × 768 and 1920 × 1080 at scale 1,
/// 2560 × 1600 at scale 1.5 (1707 × 1067), 3840 × 2160 at scale 2 (1920 × 1080). The simulator
/// draws at scale 2: the 1920 × 1080 capture is the 4K screen, pixel for pixel.
const SCREENS: [(f32, f32); 3] = [
    (1366.0, 768.0),
    (1920.0, 1080.0),
    (2560.0 / 1.5, 1600.0 / 1.5),
];

/// The opened view of the fixtures on a screen of `width` by `height`.
fn opened(width: f32, height: f32) -> View {
    let (mut view, _) = recorded(catalog());
    let _ = view.update(Message::Surface(iced::window::Id::unique()));
    let _ = view.update(Message::Resized(Size::new(width, height)));
    view.opened()
}

#[test]
fn the_view_renders_on_every_screen_in_both_themes() {
    let _theme = in_theme(Theme::Dark);
    for ((width, height), name) in SCREENS
        .into_iter()
        .zip(["1366x768", "1920x1080", "2560x1600"])
    {
        let view = opened(width, height);
        assert!(shows(&view, "Calculatrice"));
        let name = format!("apps-{name}");
        assert!(capture(&view, &name).expect("screenshot written") > 10_000);
    }
    // Two pages: their dots.
    let (mut view, _) = recorded(many(50));
    let _ = view.update(Message::Resized(Size::new(1366.0, 768.0)));
    let view = view.opened();
    assert_eq!(view.pager.count(), 2);
    assert!(capture(&view, "apps-pages").expect("screenshot written") > 10_000);
    for (which, suffix) in [(Theme::Dark, ""), (Theme::Light, "-light")] {
        color::set_active(which);
        let mut view = opened(1920.0, 1080.0);
        if which == Theme::Light {
            assert!(capture(&view, "apps-light").expect("screenshot written") > 10_000);
        }
        let _ = view.update(Message::Query("e".into()));
        let _ = view.update(Message::Key(Key::Right));
        let name = format!("apps-search{suffix}");
        assert!(capture(&view, &name).expect("screenshot written") > 10_000);
        let _ = view.update(Message::Query("zzz".into()));
        assert!(shows(&view, texts::NOTHING));
    }
    color::set_active(Theme::Dark);
}
