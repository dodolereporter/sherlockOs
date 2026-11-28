//! Rendering the dock and an icon's menu without a screen, from the neutral fixtures.

use std::path::PathBuf;
use std::time::Duration;

use dock::app::Sources;
use dock::app::{Dock, Message, Pointer};
use dock::entry::Catalog;
use dock::icons::IconLookup;
use dock::windows::{Placeholder, Recorder, Request};
use dock_config::{DockConfig, Position};
use iced::widget::container;
use iced::window::Id;
use iced::{Element, Event, Length, Point, mouse};
use iced_test::simulator::Simulator;
use sherlock_ui::theme;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use sherlock_ui::color::{self, Palette, Theme};

/// A capture sets the active theme: one at a time.
static THEME: Mutex<()> = Mutex::new(());

/// Makes `which` the active theme until the guard is dropped; no other capture runs meanwhile.
fn in_theme(which: Theme) -> MutexGuard<'static, ()> {
    let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(which);
    guard
}

/// The dock at rest, hovered over Sherlock, and an icon's menu in the light theme:
/// `dock-rest-light`, `dock-hover-light`, `dock-menu-light`, compared with DockClair and
/// MenusClair.
#[test]
fn the_dock_renders_in_the_light_theme() {
    let _theme = in_theme(Theme::Light);
    let dock = sample();
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-rest-light").expect("screenshot written") > 2_000);
    let dock = hovered(icon_center(&sample(), 2));
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-hover-light").expect("screenshot written") > 2_000);
    let mut dock = sample();
    let _ = dock.update(Message::Surface(Id::unique()));
    let _ = dock.update(Message::Pointer(Pointer::Moved(icon_center(&dock, 1))));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    unfolded(&mut dock);
    let (width, height) = dock::menu::surface_size(&dock.menu_lines(1));
    let size = (width as f32, height as f32);
    let ui = Simulator::with_size(dock::settings(), size, on_desktop(dock.view_menu()));
    assert!(capture(ui, "dock-menu-light").expect("screenshot written") > 2_000);
    color::set_active(Theme::Dark);
}

/// The dock's own menu, from a right click between two icons, in both themes:
/// `dock-own-menu-dark`, `dock-own-menu-light`.
#[test]
fn the_docks_own_menu_renders_and_toggles_sherlock() {
    for (which, name) in [
        (Theme::Dark, "dock-own-menu-dark"),
        (Theme::Light, "dock-own-menu-light"),
    ] {
        let _theme = in_theme(which);
        let mut config = DockConfig::default();
        config.magnification = false;
        let mut dock = with_config(config);
        let _ = dock.update(Message::Surface(Id::unique()));
        let layout = dock.layout(Duration::ZERO);
        let (files, terminal) = (layout.icons[0], layout.icons[1]);
        let between = Point::new(
            (files.x + files.width + terminal.x) / 2.0,
            layout.bar.center_y(),
        );
        let _ = dock.update(Message::Pointer(Pointer::Moved(between)));
        let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
        assert!(dock.menu_open(), "a right click between two icons");
        unfolded(&mut dock);
        let (width, height) = dock::menu::surface_size(&dock::menu::dock_entries(true));
        let size = (width as f32, height as f32);
        let mut ui = Simulator::with_size(dock::settings(), size, dock.view_menu());
        assert!(ui.find("Dock").is_ok(), "its header");
        ui.click("Afficher Sherlock dans le Dock")
            .expect("entry found");
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::MenuPick(dock::menu::Entry::ShowSherlock)))
        );
        let ui = Simulator::with_size(dock::settings(), size, on_desktop(dock.view_menu()));
        assert!(capture(ui, name).expect("screenshot written") > 2_000);
    }
    color::set_active(Theme::Dark);
}

#[test]
fn a_new_theme_fades_the_dock() {
    let _theme = in_theme(Theme::Dark);
    let mut dock = sample();
    let _ = dock.update(Message::Theme(Theme::Dark));
    assert!(!dock.theme.running());
    let start = Instant::now();
    let _ = dock.update(Message::Theme(Theme::Light));
    assert!(dock.animating(), "frames are asked for the fade");
    let fade = sherlock_ui::motion::THEME_FADE;
    let _ = dock.update(Message::Frame(start + fade / 3));
    assert_ne!(color::palette(), &Palette::DARK);
    assert_ne!(color::palette(), &Palette::LIGHT);
    let _ = dock.update(Message::Frame(start + fade * 2));
    assert!(!dock.theme.running());
    assert_eq!(color::palette(), &Palette::LIGHT);
    color::set_active(Theme::Dark);
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// What the dock reads, from the fixtures: four pinned applications, no configuration file.
fn sources(config: DockConfig) -> Sources {
    let data = fixtures().join("data");
    let catalog = Catalog::load(std::slice::from_ref(&data));
    let lookup = IconLookup::new(std::slice::from_ref(&data), None, Some("Exemple"));
    let pinned: Vec<String> = [
        "org.example.Files",
        "org.example.Terminal",
        "example-editor",
        "example-browser",
    ]
    .map(String::from)
    .to_vec();
    Sources {
        catalog,
        lookup,
        config,
        config_path: None,
        pinned,
        data_dirs: vec![data],
        home: None,
        gtk_theme: None,
        control: Arc::new(Recorder::default()),
        applications: false,
    }
}

/// Four applications of the fixtures, Sherlock in the middle; the terminal open and active,
/// the browser open.
fn sample() -> Dock {
    with_config(DockConfig::default())
}

fn with_config(config: DockConfig) -> Dock {
    let mut dock = Dock::new(sources(config));
    let Placeholder(windows) = Placeholder::new("terminal,ExampleBrowser", Some("terminal"));
    let _ = dock.update(Message::Windows(windows));
    dock
}

/// The dock with the pointer at `point`, once the springs have settled.
fn hovered(point: Point) -> Dock {
    let mut dock = sample();
    let _ = dock.hover(Some(point), Duration::ZERO);
    dock.frame(Duration::from_secs(2));
    assert!(!dock.animating(), "settled");
    dock
}

/// Lets the menu that just opened unfold.
fn unfolded(dock: &mut Dock) {
    let _ = dock.tick(dock.now() + Duration::from_secs(1));
    assert!(!dock.animating(), "the menu is open, still");
}

/// The center of icon `index` at rest.
fn icon_center(dock: &Dock, index: usize) -> Point {
    dock.layout(Duration::ZERO).icons[index].center()
}

/// The dock on the desktop background, as on the Dock design board.
fn on_desktop(element: Element<'_, Message>) -> Element<'_, Message> {
    container(element)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::desktop)
        .into()
}

fn size(dock: &Dock) -> (f32, f32) {
    (dock.surface_size.width, dock.surface_size.height)
}

/// Writes the screenshot to `target/tmp/<name>-tiny-skia.png`, returns its size in bytes.
fn capture(mut ui: Simulator<'_, Message>, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    Ok(std::fs::metadata(&written)?.len())
}

#[test]
fn the_dock_renders_at_rest() {
    let _theme = in_theme(Theme::Dark);
    let dock = sample();
    assert_eq!(dock.items.len(), 5);
    assert_eq!(dock.items[2].name, "Sherlock", "Sherlock in the middle");
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    let bytes = capture(ui, "dock-rest").expect("screenshot written");
    assert!(bytes > 2_000, "empty screenshot");
}

#[test]
fn the_dock_renders_magnified_with_its_label() {
    let _theme = in_theme(Theme::Dark);
    let rest = sample();
    let dock = hovered(icon_center(&rest, 2));
    let layout = dock.layout(Duration::from_secs(2));
    // The pointer over Sherlock: Sherlock at 52 × 1.7, its neighbours pushed away.
    assert_eq!(layout.icons[2].width.round(), 88.0);
    assert!(layout.bar.width > rest.layout(Duration::ZERO).bar.width);
    let mut ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(ui.find("Sherlock").is_ok(), "label of the hovered icon");
    let bytes = capture(ui, "dock-hover").expect("screenshot written");
    assert!(bytes > 2_000, "empty screenshot");
}

#[test]
fn the_pointer_is_tracked_over_the_surface() {
    let dock = sample();
    let point = icon_center(&dock, 1);
    let mut ui = Simulator::with_size(dock::settings(), size(&dock), dock.view_dock());
    ui.point_at(point);
    let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position: point })]);
    let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Right,
    ))]);
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(matches!(messages[0], Message::Pointer(Pointer::Moved(p)) if p == point));
    assert!(matches!(
        messages[1],
        Message::Pointer(Pointer::Pressed(mouse::Button::Right))
    ));
}

#[test]
fn the_menu_of_an_icon_renders_its_entries() {
    let _theme = in_theme(Theme::Dark);
    let mut dock = sample();
    let _ = dock.update(Message::Surface(Id::unique()));
    let point = icon_center(&dock, 1);
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    assert!(dock.menu_open());
    unfolded(&mut dock);
    let entries = dock.menu_lines(1);
    let (width, height) = dock::menu::surface_size(&entries);
    let mut ui = Simulator::with_size(
        dock::settings(),
        (width as f32, height as f32),
        dock.view_menu(),
    );
    // The terminal has one window, without title: listed under the application's name.
    for text in [
        "1 fenêtre",
        "Nouvelle fenêtre",
        "Nouvel onglet",
        "Retirer du Dock",
        "Quitter",
    ] {
        assert!(ui.find(text).is_ok(), "{text} missing from the menu");
    }
    // « Quitter » is disabled: a placeholder cannot close windows. Clicking it does nothing.
    ui.click("Quitter").expect("entry found");
    assert!(
        !ui.into_messages()
            .any(|m| matches!(m, Message::MenuPick(_))),
        "a disabled entry picks nothing"
    );
    let ui = Simulator::with_size(
        dock::settings(),
        (width as f32, height as f32),
        dock.view_menu(),
    );
    let bytes = capture(ui, "dock-menu").expect("screenshot written");
    assert!(bytes > 2_000, "empty screenshot");
}

#[test]
fn unpinning_keeps_sherlock_in_its_place() {
    let mut dock = sample();
    let _ = dock.unpin(0);
    let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Terminal d'exemple",
            "Sherlock",
            "Éditeur",
            "Example Browser"
        ]
    );
    // Its place is written, rather than the middle of the three that are left.
    assert_eq!(
        dock.pinned,
        [
            "org.example.Terminal",
            "sherlock",
            "example-editor",
            "example-browser"
        ]
    );
    // Sherlock can be unpinned too.
    let _ = dock.unpin(1);
    assert_eq!(dock.items.len(), 3);
    assert!(!dock.sherlock);
}

#[test]
fn an_open_application_can_be_kept_and_unpinned_again() {
    let mut dock = sample();
    // The terminal is open: unpinned, it moves after the separator with its dot.
    let _ = dock.unpin(1);
    let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Fichiers",
            "Sherlock",
            "Éditeur",
            "Example Browser",
            "Terminal d'exemple"
        ]
    );
    assert_eq!(dock.separator, Some(4));
    assert_eq!(dock.open_windows(4), Some((1, true)));
    let abilities = dock.abilities(4);
    assert!(abilities.keep && !abilities.unpin);
    // « Garder dans le Dock »: back among the pinned ones, at their end.
    let _ = dock.keep(4);
    assert_eq!(dock.separator, None);
    assert_eq!(
        dock.pinned.last().map(String::as_str),
        Some("org.example.Terminal")
    );
    assert_eq!(dock.items[1].name, "Sherlock");
}

#[test]
fn the_dock_renders_with_an_open_application_after_the_separator() {
    let _theme = in_theme(Theme::Dark);
    let mut dock = sample();
    let _ = dock.unpin(1);
    // The terminal moves after the separator: once it is there.
    let _ = dock.tick(dock.now() + Duration::from_secs(3));
    assert!(!dock.animating());
    let layout = dock.layout(dock.now());
    assert!(layout.separator.is_some());
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    let bytes = capture(ui, "dock-open").expect("screenshot written");
    assert!(bytes > 2_000, "empty screenshot");
}

#[test]
fn a_new_configuration_moves_and_resizes_the_dock() {
    let mut dock = sample();
    let bottom = dock.surface_size;
    let mut config = DockConfig::default();
    config.position = Position::Left;
    config.icon_size = 40.0;
    config.magnification_factor = 2.0;
    let _ = dock.update(Message::Config(Box::new(config)));
    assert_eq!(dock.geometry.position, Position::Left);
    assert!(
        dock.surface_size.height > dock.surface_size.width,
        "a tall surface"
    );
    assert_ne!(dock.surface_size, bottom);
    let layout = dock.layout(Duration::ZERO);
    assert_eq!(layout.icons[0].width, 40.0);
    assert_eq!(layout.bar.x, 8.0, "the gap of 8 from the left edge");
    // Without magnification nothing grows under the pointer.
    let mut config = DockConfig::default();
    config.magnification = false;
    let _ = dock.update(Message::Config(Box::new(config)));
    let point = dock.layout(Duration::ZERO).icons[2].center();
    let _ = dock.hover(Some(point), Duration::ZERO);
    dock.frame(Duration::from_secs(2));
    assert_eq!(dock.layout(Duration::from_secs(2)).icons[2].width, 52.0);
}

#[test]
fn the_dock_renders_on_the_left_edge() {
    let _theme = in_theme(Theme::Dark);
    let mut config = DockConfig::default();
    config.position = Position::Left;
    let rest = with_config(config.clone());
    let point = rest.layout(Duration::ZERO).icons[2].center();
    let mut dock = with_config(config);
    let _ = dock.hover(Some(point), Duration::ZERO);
    dock.frame(Duration::from_secs(2));
    let layout = dock.layout(Duration::from_secs(2));
    assert_eq!(
        layout.icons[2].height.round(),
        88.0,
        "magnified along the vertical axis"
    );
    let mut ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(ui.find("Sherlock").is_ok(), "label beside the icon");
    let bytes = capture(ui, "dock-left").expect("screenshot written");
    assert!(bytes > 2_000, "empty screenshot");
}

#[test]
fn a_click_brings_forward_or_minimizes_an_open_application() {
    let recorder = Recorder::default();
    let mut sources = sources(DockConfig::default());
    sources.control = Arc::new(recorder.clone());
    let mut dock = Dock::new(sources);
    let Placeholder(mut windows) = Placeholder::new("terminal,ExampleBrowser", Some("terminal"));
    // A source that can act on its windows, unlike the placeholder.
    windows.controllable = true;
    let _ = dock.update(Message::Windows(windows));
    let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names[1], "Terminal d'exemple");
    assert_eq!(names[4], "Example Browser");
    dock.click(1);
    dock.click(4);
    // The terminal in front is minimized; the browser behind has its window brought forward.
    assert_eq!(recorder.requests(), [Request::Minimize]);
    assert_eq!(recorder.targeted(), [(Request::Activate, 2)]);
    // An application without window is started, not asked anything.
    dock.click(0);
    assert_eq!(recorder.requests().len(), 1);
    assert_eq!(recorder.targeted().len(), 1);
}

#[test]
fn crossing_the_gap_between_grown_icons_keeps_the_magnification() {
    let recorder = Recorder::default();
    let mut sources = sources(DockConfig::default());
    sources.control = Arc::new(recorder.clone());
    let mut dock = Dock::new(sources);
    let _ = dock.update(Message::Surface(Id::unique()));
    let at = |dock: &Dock, s: u64| dock.layout(Duration::from_secs(s));
    let _ = dock.hover(Some(at(&dock, 0).icons[2].center()), Duration::ZERO);
    dock.frame(Duration::from_secs(1));
    // Along the top of the grown icons, into the gap between Sherlock and the terminal.
    let layout = at(&dock, 1);
    let (left, middle) = (layout.icons[1], layout.icons[2]);
    let gap = Point::new(left.x + left.width + 4.0, middle.y + 20.0);
    assert!(gap.y < layout.bar.y && layout.icon_at(gap).is_none());
    let _ = dock.hover(Some(gap), Duration::from_secs(1));
    dock.frame(Duration::from_secs(3));
    assert!(at(&dock, 3).icons[2].width > 70.0, "still magnified");
    // A click in the gap starts nothing and asks nothing.
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Left)));
    assert!(recorder.requests().is_empty());
    // Leaving the band lets everything come back to rest.
    let _ = dock.hover(Some(Point::new(gap.x, 1.0)), Duration::from_secs(3));
    dock.frame(Duration::from_secs(5));
    assert_eq!(at(&dock, 5).icons[2].width, 52.0);
}

#[test]
fn the_menu_follows_its_application_when_the_icons_move() {
    let mut dock = sample();
    let _ = dock.update(Message::Surface(Id::unique()));
    let point = dock.layout(Duration::ZERO).icons[1].center();
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    assert!(dock.menu_open());
    // The pinned order changes while the menu is open: the terminal moves to the first place,
    // the files take its old one.
    let mut config = DockConfig::default();
    config.pinned = Some(
        [
            "org.example.Terminal",
            "org.example.Files",
            "example-editor",
            "example-browser",
        ]
        .map(String::from)
        .to_vec(),
    );
    let _ = dock.update(Message::Config(Box::new(config)));
    assert!(dock.menu_open(), "same size: the menu stays");
    let _ = dock.update(Message::MenuPick(dock::menu::Entry::Unpin));
    // The terminal went, not the application now at its old place.
    assert_eq!(
        dock.pinned,
        [
            "org.example.Files",
            "sherlock",
            "example-editor",
            "example-browser"
        ]
    );
}

/// The next theme of the appearance file, five seconds at most.
fn next_theme(themes: &mut iced::futures::channel::mpsc::UnboundedReceiver<Theme>) -> Theme {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(theme) = themes.try_recv() {
            return theme;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("no theme from the appearance file");
}

/// `Theme=` rewritten in the appearance file: the dock follows without restarting. Its glass,
/// its dots, the label and Sherlock's tile take the palette shown: `dock-live-dark`, then
/// `dock-live-light`, hovered over Sherlock with two applications open.
#[test]
fn the_dock_follows_a_new_theme_in_the_file() {
    let _theme = in_theme(Theme::Dark);
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("dock-live-theme");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch folder");
    let file = dir.join("appearance.conf");
    let write = |theme: &str| {
        let new = file.with_extension("new");
        let text = format!("[Appearance]\nTheme={theme}\n");
        std::fs::write(&new, text).expect("appearance file");
        std::fs::rename(&new, &file).expect("appearance file");
    };
    write("dark");
    let mut themes = dock::app::appearance(&file);
    let mut dock = hovered(icon_center(&sample(), 2));
    let _ = dock.update(Message::Theme(next_theme(&mut themes)));
    assert_eq!(color::palette(), &Palette::DARK);
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-live-dark").expect("screenshot written") > 2_000);

    write("light");
    let start = Instant::now();
    let _ = dock.update(Message::Theme(next_theme(&mut themes)));
    assert!(dock.animating(), "frames are asked for the fade");
    let fade = sherlock_ui::motion::THEME_FADE;
    let _ = dock.update(Message::Frame(start + fade * 2));
    assert!(!dock.animating(), "nothing more once it is over");
    assert_eq!(color::palette(), &Palette::LIGHT);
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-live-light").expect("screenshot written") > 2_000);
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let dark = std::fs::read(tmp.join("dock-live-dark-tiny-skia.png")).expect("capture");
    let light = std::fs::read(tmp.join("dock-live-light-tiny-skia.png")).expect("capture");
    assert_ne!(dark, light, "the dock changed with the theme");
    color::set_active(Theme::Dark);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The « Applications » button (D-41): first, Sherlock still in the middle of the applications,
/// its menu disabled; `dock-applications`, the pointer over it.
#[test]
fn the_applications_button_comes_first() {
    let _theme = in_theme(Theme::Dark);
    let mut sources = sources(DockConfig::default());
    sources.applications = true;
    let mut dock = Dock::new(sources);
    let Placeholder(windows) = Placeholder::new("terminal,ExampleBrowser", Some("terminal"));
    let _ = dock.update(Message::Windows(windows));
    let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names[0], sherlock_ui::dock::text::APPLICATIONS);
    assert_eq!(
        names[3], "Sherlock",
        "in the middle of the four applications"
    );
    assert_eq!(names.len(), 6);
    let can = dock.abilities(0);
    assert!(!can.launch && !can.keep && !can.unpin && !can.act);
    let point = icon_center(&dock, 0);
    let _ = dock.hover(Some(point), Duration::ZERO);
    dock.frame(Duration::from_secs(2));
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-applications").expect("screenshot written") > 2_000);
}

/// Lot D-21j: the trash comes last, after the separator, full here; its menu opens it or empties
/// it, the second click only: the first one arms « Reclique pour vider » and the menu stays.
/// `dock-trash`, `dock-trash-menu`.
#[test]
fn the_trash_ends_the_dock_and_empties_on_a_second_click() {
    let _theme = in_theme(Theme::Dark);
    let mut dock = sample();
    let _ = dock.update(Message::Surface(Id::unique()));
    let _ = dock.update(Message::Trash(true));
    let _ = dock.tick(Duration::from_secs(5));
    let last = dock.items.len() - 1;
    assert_eq!(dock.items[last].name, "Corbeille");
    assert!(dock.separator.is_some_and(|s| s <= last));
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-trash").expect("screenshot") > 2_000);
    let point = icon_center(&dock, last);
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    assert!(dock.menu_open());
    unfolded(&mut dock);
    let labels: Vec<String> = dock
        .menu_lines(last)
        .iter()
        .map(|l| l.label.clone())
        .collect();
    assert_eq!(labels, ["Ouvrir", "Vider la corbeille"]);
    let _ = dock.update(Message::MenuPick(dock::menu::Entry::EmptyTrash));
    assert!(dock.menu_open(), "a first click only arms");
    assert_eq!(dock.menu_lines(last)[1].label, "Reclique pour vider");
    let entries = dock.menu_lines(last);
    let (width, height) = dock::menu::surface_size(&entries);
    {
        let mut ui = Simulator::with_size(
            dock::settings(),
            (width as f32, height as f32),
            on_desktop(dock.view_menu()),
        );
        assert!(
            ui.find("Reclique pour vider").is_ok(),
            "the open menu says it"
        );
        assert!(capture(ui, "dock-trash-menu").expect("screenshot") > 2_000);
    }
    // Emptied elsewhere: the trash shows empty and its menu cannot empty it.
    let _ = dock.update(Message::Trash(false));
    let last = dock.items.len() - 1;
    assert!(!dock.menu_lines(last)[1].enabled);
}

/// Lot D-21l: on a screen too narrow for its icons, the dock shrinks them and its surface stays
/// inside the screen, 8 from each end; on 1920 it keeps its size. `dock-fitted-1920`,
/// `dock-fitted-520`.
#[test]
fn the_dock_shrinks_to_fit_a_narrow_screen() {
    use dock::app::Screen;
    use iced::window::Id;
    for width in [1920, 520] {
        let mut dock = sample();
        let id = Id::unique();
        let _ = dock.update(Message::Surface(id));
        let _ = dock.update(Message::Screen(
            id,
            Some(Screen {
                name: Some(String::from("eDP-1")),
                size: Some((width, 800)),
            }),
        ));
        let _ = dock.tick(Duration::from_secs(5));
        let room = width as f32 - 2.0 * sherlock_ui::dock::SCREEN_ROOM;
        assert!(
            dock.surface_size.width <= room,
            "{width}: {:?}",
            dock.surface_size
        );
        if width == 1920 {
            assert_eq!(dock.geometry, dock.base);
        } else {
            assert!(dock.geometry.icon < dock.base.icon);
        }
        let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
        let written = capture(ui, &format!("dock-fitted-{width}")).expect("capture");
        assert!(written > 1_000);
    }
}

/// Lot D-41d: an application dragged from the Applications view opens a hole among the pinned
/// icons under the pointer, moves it, closes it out of the dock, and is pinned where it is
/// dropped. `dock-hole`, `dock-dropped`.
#[test]
fn an_application_dragged_from_the_view_opens_a_hole_and_is_pinned() {
    use dock::dnd::Event;
    use dock::items::{HOLE, Kind};
    let _theme = in_theme(Theme::Dark);
    let mut dock = sample();
    let _ = dock.tick(Duration::from_secs(5));
    let (files, terminal) = (icon_center(&dock, 0), icon_center(&dock, 1));
    let between = Point::new((files.x + terminal.x) / 2.0, files.y);
    let _ = dock.update(Message::Dnd(Event::Over(between.x, between.y)));
    assert_eq!(
        dock.items[1].kind,
        Kind::Hole,
        "the hole before the terminal"
    );
    let _ = dock.tick(Duration::from_secs(10));
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-hole").expect("screenshot") > 2_000);
    // Out of the dock's band: it closes.
    let _ = dock.update(Message::Dnd(Event::Over(between.x, 0.0)));
    assert!(dock.items.iter().all(|i| i.kind != Kind::Hole));
    let _ = dock.update(Message::Dnd(Event::Over(between.x, between.y)));
    let _ = dock.update(Message::Dnd(Event::Dropped(
        "org.example.OtherBrowser".into(),
    )));
    assert_eq!(dock.pinned[1], "org.example.OtherBrowser");
    assert!(dock.pinned.iter().all(|p| p != HOLE), "never kept");
    let _ = dock.tick(Duration::from_secs(15));
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    assert!(capture(ui, "dock-dropped").expect("screenshot") > 2_000);
    // Dropped again where it already is: it stays pinned once.
    let _ = dock.update(Message::Dnd(Event::Over(between.x, between.y)));
    let _ = dock.update(Message::Dnd(Event::Dropped(
        "org.example.OtherBrowser".into(),
    )));
    let count = dock
        .pinned
        .iter()
        .filter(|p| *p == "org.example.OtherBrowser")
        .count();
    assert_eq!(count, 1);
    // Something unknown: the hole closes, nothing is pinned.
    let _ = dock.update(Message::Dnd(Event::Over(between.x, between.y)));
    let _ = dock.update(Message::Dnd(Event::Dropped("not.installed".into())));
    assert!(
        dock.pinned
            .iter()
            .all(|p| p != HOLE && p != "not.installed")
    );
}
