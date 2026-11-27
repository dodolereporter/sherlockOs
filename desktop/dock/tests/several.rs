//! Applications with several windows (lot D-21c), after the macOS dock: one icon per
//! application whatever the kind of its windows, a click with none, one or several windows,
//! a new window from the middle button, Ctrl+click or the menu, the menu that lists the
//! windows, the dot that fades, and captures of an application with three windows.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dock::app::{Dock, Message, Pointer, Sources};
use dock::entry::Catalog;
use dock::icons::IconLookup;
use dock::menu::{Entry, Key};
use dock::windows::{Recorder, Request, Window, Windows};
use dock_config::DockConfig;
use iced::widget::{container, pin, stack};
use iced::window::Id;
use iced::{Element, Length, Point, keyboard, mouse};
use iced_test::simulator::Simulator;
use sherlock_ui::color::{self, Theme};
use sherlock_ui::{dock as tokens, theme};

const FOUR: [&str; 4] = [
    "org.example.Files",
    "org.example.Terminal",
    "example-editor",
    "example-browser",
];

/// Place of the browser among the files, the terminal, Sherlock, the editor and the browser.
const BROWSER: usize = 4;
const TERMINAL: usize = 1;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data")
}

/// The dock over the fixtures and `extra` data folders, `pinned`, acting through `recorder`.
fn dock_with(extra: &[PathBuf], pinned: &[&str], recorder: &Recorder) -> Dock {
    let mut data = vec![fixtures()];
    data.extend_from_slice(extra);
    let sources = Sources {
        catalog: Catalog::load(&data),
        lookup: IconLookup::new(&data, None, Some("Exemple")),
        config: DockConfig::default(),
        config_path: None,
        pinned: pinned.iter().map(|p| String::from(*p)).collect(),
        data_dirs: data,
        home: None,
        gtk_theme: None,
        control: Arc::new(recorder.clone()),
        applications: false,
    };
    let mut dock = Dock::new(sources);
    let _ = dock.update(Message::Surface(Id::unique()));
    dock
}

fn window(id: u64, app_id: &str, title: &str, recency: u64) -> Window {
    Window {
        title: String::from(title),
        recency,
        ..Window::new(id, app_id, false)
    }
}

/// Three windows of the browser, two Wayland ones under its desktop file id and an X11 one
/// under its `StartupWMClass` through XWayland, then one of the terminal. The browser's
/// second window is the most recent, the first the least.
fn mixed() -> Windows {
    Windows {
        list: vec![
            window(11, "example-browser", "Accueil — Example Browser", 1),
            window(
                12,
                "ExampleBrowser",
                "Documentation de l'interface de recherche des fichiers récents — Example Browser",
                3,
            ),
            window(
                13,
                "example-browser",
                "Notes de réunion — Example Browser",
                2,
            ),
            window(20, "org.example.terminal", "", 0),
        ],
        controllable: true,
        expose: true,
        ..Windows::default()
    }
}

fn names(dock: &Dock) -> Vec<&str> {
    dock.items.iter().map(|i| i.name.as_str()).collect()
}

fn settle(dock: &mut Dock) {
    let _ = dock.tick(dock.now() + Duration::from_secs(3));
    assert!(!dock.animating(), "no frame is asked for at rest");
}

fn center(dock: &Dock, index: usize) -> Point {
    dock.layout(dock.now()).icons[index].center()
}

#[test]
fn the_windows_of_one_application_share_one_icon() {
    let recorder = Recorder::default();
    let mut dock = dock_with(&[], &FOUR, &recorder);
    let _ = dock.update(Message::Windows(mixed()));
    assert_eq!(dock.items.len(), 5, "nothing after a separator");
    assert_eq!(dock.separator, None);
    assert_eq!(dock.open_windows(BROWSER), Some((3, false)));
    assert_eq!(dock.open_windows(TERMINAL), Some((1, false)));
    // Not pinned: X11 and Wayland windows still make one icon, after the separator.
    let mut dock = dock_with(&[], &FOUR[..2], &recorder);
    let _ = dock.update(Message::Windows(mixed()));
    assert_eq!(
        names(&dock),
        [
            "Fichiers",
            "Sherlock",
            "Terminal d'exemple",
            "Example Browser"
        ]
    );
    assert_eq!(dock.separator, Some(3));
    assert_eq!(dock.open_windows(3), Some((3, false)));
}

#[test]
fn a_click_with_zero_one_or_several_windows() {
    let recorder = Recorder::default();
    let mut dock = dock_with(&[], &FOUR, &recorder);
    let _ = dock.update(Message::Windows(mixed()));
    // No window: started (the fixture's program does not exist), nothing asked.
    dock.click(0);
    assert!(recorder.requests().is_empty() && recorder.targeted().is_empty());
    // One window, behind: brought forward.
    dock.click(TERMINAL);
    assert_eq!(recorder.targeted(), [(Request::Activate, 20)]);
    // Several, behind: all forward, the most recent last, on top with the keyboard.
    dock.click(BROWSER);
    assert_eq!(
        recorder.targeted()[1..],
        [
            (Request::Activate, 11),
            (Request::Activate, 13),
            (Request::Activate, 12)
        ]
    );
    // One minimized while others are shown: it stays down.
    let mut windows = mixed();
    windows.list[1].minimized = true;
    let _ = dock.update(Message::Windows(windows.clone()));
    dock.click(BROWSER);
    assert_eq!(
        recorder.targeted()[4..],
        [(Request::Activate, 11), (Request::Activate, 13)]
    );
    // The terminal in front, alone: minimized, the author's choice.
    windows.list[3].activated = true;
    let _ = dock.update(Message::Windows(windows));
    dock.click(TERMINAL);
    assert_eq!(recorder.requests(), [Request::Minimize]);
}

/// A data folder with an application that starts and exits at once (`true`), whose windows
/// are `QuickExample`, with a « new window » action when `action`.
fn quick_app(name: &str, action: bool) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-several"));
    let _ = std::fs::remove_dir_all(&dir);
    let apps = dir.join("applications");
    assert!(std::fs::create_dir_all(&apps).is_ok(), "folder");
    let mut entry = String::from(
        "[Desktop Entry]\nType=Application\nName=Rapide\nExec=true\n\
         StartupWMClass=QuickExample\n",
    );
    if action {
        entry.push_str("Actions=new-window;\n[Desktop Action new-window]\nName=New\nExec=true\n");
    }
    assert!(
        std::fs::write(apps.join("org.example.Quick.desktop"), entry).is_ok(),
        "entry"
    );
    dir
}

fn quick_windows(count: u64, recent: u64) -> Message {
    Message::Windows(Windows {
        list: (1..=count)
            .map(|id| {
                window(
                    id,
                    "QuickExample",
                    "Rapide",
                    if id == count { recent } else { 0 },
                )
            })
            .collect(),
        controllable: true,
        ..Windows::default()
    })
}

#[test]
fn the_middle_button_and_ctrl_click_open_a_new_window() {
    for (name, action) in [("quick-action", true), ("quick-exec", false)] {
        let recorder = Recorder::default();
        let mut dock = dock_with(
            &[quick_app(name, action)],
            &["org.example.Quick"],
            &recorder,
        );
        let _ = dock.update(quick_windows(1, 1));
        settle(&mut dock);
        // The middle button: started again, it bounces, nothing asked of its window.
        let point = center(&dock, 0);
        let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
        let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Middle)));
        assert_eq!(dock.launches.len(), 1, "{name}: bounces");
        assert!(recorder.targeted().is_empty());
        // Its second window comes: the bounce stops.
        let _ = dock.update(quick_windows(2, 2));
        assert!(dock.launches[0].bounce.stopping());
        settle(&mut dock);
        // Ctrl+click: the same, not a click on its windows.
        let _ = dock.update(Message::Modifiers(keyboard::Modifiers::CTRL));
        let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Left)));
        let _ = dock.update(Message::Pointer(Pointer::Released));
        assert_eq!(dock.launches.len(), 1, "{name}: Ctrl+click bounces");
        assert!(recorder.targeted().is_empty() && recorder.requests().is_empty());
        // An application with a single instance raises the window it had: the bounce stops.
        let _ = dock.update(quick_windows(2, 3));
        assert!(dock.launches[0].bounce.stopping(), "{name}: raised");
        // Without Ctrl, a click acts on its windows again.
        settle(&mut dock);
        let _ = dock.update(Message::Modifiers(keyboard::Modifiers::empty()));
        let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Left)));
        let _ = dock.update(Message::Pointer(Pointer::Released));
        assert_eq!(recorder.targeted().len(), 2, "{name}: both brought forward");
    }
}

/// Opens the menu of icon `index` and lets it unfold.
fn open_menu(dock: &mut Dock, index: usize) {
    let point = center(dock, index);
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    assert!(dock.menu_open());
    settle(dock);
}

#[test]
fn the_menu_lists_the_windows_then_the_actions() {
    let recorder = Recorder::default();
    let mut dock = dock_with(&[], &FOUR, &recorder);
    let mut windows = mixed();
    windows.list[1].activated = true;
    let _ = dock.update(Message::Windows(windows));
    let lines = dock.menu_lines(BROWSER);
    let labels: Vec<(&str, bool, bool)> = lines
        .iter()
        .map(|l| (l.label.as_str(), l.enabled, l.checked))
        .collect();
    assert_eq!(
        labels,
        [
            ("Accueil — Example Browser", true, false),
            (
                "Documentation de l'interface de recherche des fichiers récents — Example Browser",
                true,
                true
            ),
            ("Notes de réunion — Example Browser", true, false),
            ("Nouvelle fenêtre", true, false),
            ("Nouvelle fenêtre de navigation privée", true, false),
            ("Retirer du Dock", true, false),
            ("Afficher toutes les fenêtres", true, false),
            ("Quitter", true, false),
        ]
    );
    // An untitled window takes the application's name.
    assert_eq!(dock.menu_lines(TERMINAL)[0].label, "Terminal d'exemple");
    // A window of the menu comes forward, alone.
    open_menu(&mut dock, BROWSER);
    let _ = dock.update(Message::MenuPick(Entry::Window(13)));
    assert_eq!(recorder.targeted(), [(Request::Activate, 13)]);
    // The menu folds back, then its popup goes.
    assert!(dock.menu_open() && dock.animating());
    settle(&mut dock);
    assert!(!dock.menu_open());
    // « Quitter »: every window of the application is asked to close, which may ask first.
    open_menu(&mut dock, BROWSER);
    let _ = dock.update(Message::MenuPick(Entry::Quit));
    assert_eq!(recorder.requests(), [Request::Close]);
    // While it folds back, it takes no other pick.
    let _ = dock.update(Message::MenuPick(Entry::Window(11)));
    assert_eq!(recorder.targeted().len(), 1);
}

/// App Expose (lot D-21c2): a click on the application in front with several windows, its
/// menu, and a swipe down over its icon.
#[test]
fn app_expose_shows_the_windows_of_the_application() {
    let recorder = Recorder::default();
    let mut dock = dock_with(&[], &FOUR, &recorder);
    let mut windows = mixed();
    windows.list[1].activated = true;
    let _ = dock.update(Message::Windows(windows.clone()));
    // In front with three windows: App Expose on them, nothing minimized.
    dock.click(BROWSER);
    assert_eq!(recorder.exposed(), [vec![11, 13, 12]]);
    assert!(recorder.requests().is_empty());
    // From the menu, the minimized ones left out.
    windows.list[0].minimized = true;
    let _ = dock.update(Message::Windows(windows.clone()));
    open_menu(&mut dock, BROWSER);
    let _ = dock.update(Message::MenuPick(Entry::ShowAll));
    assert_eq!(recorder.exposed()[1], [13, 12]);
    settle(&mut dock);
    // A swipe down over the icon: once, however far it goes, then again after a pause.
    let point = center(&dock, BROWSER);
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let pixels = |y| Message::Scroll(mouse::ScrollDelta::Pixels { x: 0.0, y });
    let _ = dock.update(pixels(tokens::EXPOSE_SWIPE / 2.0));
    assert_eq!(recorder.exposed().len(), 2, "not far enough");
    let _ = dock.update(pixels(tokens::EXPOSE_SWIPE));
    let _ = dock.update(pixels(tokens::EXPOSE_SWIPE * 3.0));
    assert_eq!(recorder.exposed().len(), 3, "once per swipe");
    // Up does nothing; a notch of the wheel down, after a pause, opens it again.
    let _ = dock.tick(dock.now() + tokens::EXPOSE_SWIPE_GAP * 2);
    let _ = dock.update(pixels(-tokens::EXPOSE_SWIPE * 2.0));
    assert_eq!(recorder.exposed().len(), 3);
    let _ = dock.tick(dock.now() + tokens::EXPOSE_SWIPE_GAP * 2);
    let _ = dock.update(Message::Scroll(mouse::ScrollDelta::Lines {
        x: 0.0,
        y: -1.0,
    }));
    assert_eq!(recorder.exposed().len(), 4);
    // Without App Expose (a source before version 4): a click minimizes, the entry is off.
    windows.expose = false;
    let _ = dock.update(Message::Windows(windows));
    dock.click(BROWSER);
    assert_eq!(recorder.requests(), [Request::Minimize]);
    let lines = dock.menu_lines(BROWSER);
    let show_all = lines.iter().find(|l| l.entry == Entry::ShowAll);
    assert!(show_all.is_some_and(|l| !l.enabled));
}

#[test]
fn the_menu_unfolds_and_escape_folds_it_back() {
    let recorder = Recorder::default();
    let mut dock = dock_with(&[], &FOUR, &recorder);
    let _ = dock.update(Message::Windows(mixed()));
    let point = center(&dock, BROWSER);
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    assert!(dock.animating(), "it unfolds");
    settle(&mut dock);
    let _ = dock.update(Message::Key(Key::Close));
    assert!(dock.menu_open(), "still drawn while it folds back");
    let _ = dock.tick(dock.now() + tokens::MENU_FADE);
    assert!(!dock.menu_open());
}

#[test]
fn the_dot_fades_in_with_the_first_window_and_out_with_the_last() {
    let recorder = Recorder::default();
    let mut dock = dock_with(&[], &FOUR, &recorder);
    // The first list: in place at once.
    let _ = dock.update(Message::Windows(mixed()));
    assert_eq!(dock.dot(BROWSER), 1.0);
    assert_eq!(dock.dot(0), 0.0);
    // A third window of the terminal, then its last one closes: one dot all along.
    let mut windows = mixed();
    windows.list.push(window(21, "terminal", "", 0));
    let _ = dock.update(Message::Windows(windows));
    assert_eq!(dock.dot(TERMINAL), 1.0, "one dot, however many windows");
    let mut windows = mixed();
    windows.list.retain(|w| w.id != 20);
    let _ = dock.update(Message::Windows(windows));
    let start = dock.now();
    assert_eq!(dock.dot(TERMINAL), 1.0, "it starts fading");
    let _ = dock.tick(start + tokens::DOT_FADE / 2);
    assert!(dock.dot(TERMINAL) > 0.0 && dock.dot(TERMINAL) < 1.0);
    settle(&mut dock);
    assert_eq!(dock.dot(TERMINAL), 0.0);
    // The files open a window: their dot fades in.
    let mut windows = mixed();
    windows.list.push(window(30, "files", "", 0));
    let _ = dock.update(Message::Windows(windows));
    assert_eq!(dock.dot(0), 0.0);
    let _ = dock.tick(dock.now() + tokens::DOT_FADE);
    assert_eq!(dock.dot(0), 1.0);
}

/// A capture sets the active theme: one at a time.
static THEME: Mutex<()> = Mutex::new(());

fn in_theme(which: Theme) -> MutexGuard<'static, ()> {
    let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(which);
    guard
}

/// The dock at the bottom of a screen of `width` by `height`, with the menu of `index` open
/// above its icon, as the compositor places the popup.
fn on_screen(dock: &Dock, index: usize, width: f32, height: f32) -> Element<'_, Message> {
    let surface = dock.surface_size;
    let origin = Point::new(
        ((width - surface.width) / 2.0).round(),
        height - surface.height,
    );
    let layout = dock.layout(dock.now());
    let icon = layout.icons[index];
    let (menu_width, menu_height) = dock::menu::surface_size(&dock.menu_lines(index));
    let (menu_width, menu_height) = (menu_width as f32, menu_height as f32);
    let menu_x = origin.x + icon.center_x() - menu_width / 2.0;
    let menu_y = origin.y + layout.bar.y.min(icon.y) - tokens::MENU_GAP - menu_height;
    let dock_view = container(dock.view_dock())
        .width(surface.width)
        .height(surface.height);
    let menu = container(dock.view_menu())
        .width(menu_width)
        .height(menu_height);
    let layers = stack([
        pin(dock_view).x(origin.x).y(origin.y).into(),
        pin(menu).x(menu_x.round()).y(menu_y.round()).into(),
    ])
    .width(Length::Fill)
    .height(Length::Fill);
    container(layers)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::desktop)
        .into()
}

fn capture(
    element: Element<'_, Message>,
    size: (f32, f32),
    name: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let mut ui = Simulator::with_size(dock::settings(), size, element);
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    Ok(std::fs::metadata(&written)?.len())
}

/// The browser with three windows, one of them in front, its menu open: `dock-windows-<theme>-
/// <width>`, dark and light, on a screen of 1920 × 1080 and of 1366 × 768.
#[test]
fn an_application_with_three_windows_renders_with_its_menu() {
    for which in [Theme::Dark, Theme::Light] {
        let _theme = in_theme(which);
        let recorder = Recorder::default();
        let mut dock = dock_with(&[], &FOUR, &recorder);
        let mut windows = mixed();
        windows.list[1].activated = true;
        let _ = dock.update(Message::Windows(windows));
        open_menu(&mut dock, BROWSER);
        // The pointer leaves for the menu: the icons are back at rest.
        let _ = dock.update(Message::Pointer(Pointer::Left));
        settle(&mut dock);
        assert!(dock.menu_open());
        let theme = if which == Theme::Dark {
            "dark"
        } else {
            "light"
        };
        for (width, height) in [(1920.0, 1080.0), (1366.0, 768.0)] {
            let name = format!("dock-windows-{theme}-{width}");
            let bytes = capture(
                on_screen(&dock, BROWSER, width, height),
                (width, height),
                &name,
            )
            .expect("screenshot written");
            assert!(bytes > 10_000, "{name}: empty screenshot");
        }
    }
    color::set_active(Theme::Dark);
}
