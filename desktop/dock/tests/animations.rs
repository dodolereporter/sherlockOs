//! The animations of the dock without a screen: icons arriving and leaving, reordering by
//! drag and drop, unpinning by dragging out, the bounce of a starting application.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dock::app::{Dock, Message, Pointer, Sources};
use dock::drag::Gesture;
use dock::entry::Catalog;
use dock::icons::IconLookup;
use dock::layout::Layout;
use dock::windows::{Placeholder, Recorder, Window, Windows};
use dock_config::DockConfig;
use iced::widget::container;
use iced::window::Id;
use iced::{Element, Length, Point, mouse};
use iced_test::simulator::Simulator;
use sherlock_ui::color::{self, Theme};
use sherlock_ui::{dock as tokens, theme};

const FOUR: [&str; 4] = [
    "org.example.Files",
    "org.example.Terminal",
    "example-editor",
    "example-browser",
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A folder of its own under the tests' temporary directory.
fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-anim"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "scratch folder");
    dir
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data")
}

/// The dock over the fixtures and `extra` data folders, `pinned`, its file at `path`.
fn dock(extra: &[PathBuf], pinned: &[&str], config: DockConfig, path: Option<PathBuf>) -> Dock {
    let mut data = vec![fixtures()];
    data.extend_from_slice(extra);
    let sources = Sources {
        catalog: Catalog::load(&data),
        lookup: IconLookup::new(&data, None, Some("Exemple")),
        config,
        config_path: path,
        pinned: pinned.iter().map(|p| String::from(*p)).collect(),
        data_dirs: data,
        home: None,
        gtk_theme: None,
        control: Arc::new(Recorder::default()),
        applications: false,
    };
    let mut dock = Dock::new(sources);
    let _ = dock.update(Message::Surface(Id::unique()));
    dock
}

/// Without magnification: the places do not depend on where the pointer is.
fn flat() -> DockConfig {
    let mut config = DockConfig::default();
    config.magnification = false;
    config
}

/// Lets every animation end; returns the time then.
fn settle(dock: &mut Dock) -> Duration {
    let end = dock.now() + Duration::from_secs(3);
    let _ = dock.tick(end);
    assert!(!dock.animating(), "no frame is asked for at rest");
    end
}

fn names(dock: &Dock) -> Vec<&str> {
    dock.items.iter().map(|i| i.name.as_str()).collect()
}

fn open(dock: &mut Dock, app_ids: &str) {
    let Placeholder(windows) = Placeholder::new(app_ids, None);
    let _ = dock.update(Message::Windows(windows));
}

/// The left button goes down on `point`.
fn press(dock: &mut Dock, point: Point) {
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Left)));
}

/// The pointer moves to `to` in small steps, the button held.
fn drag_to(dock: &mut Dock, from: Point, to: Point) {
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        let point = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        let _ = dock.update(Message::Gesture(Gesture::Moved(point)));
    }
}

fn middle(layout: &Layout, index: usize) -> Point {
    layout.icons[index].center()
}

#[test]
fn a_pinned_application_arrives_and_its_neighbours_move_aside() {
    let mut dock = dock(&[], &FOUR, DockConfig::default(), None);
    let t = settle(&mut dock);
    let rest = dock.layout(t);
    // The settings pin one more application: Sherlock goes back to the middle.
    let mut config = DockConfig::default();
    let mut pinned: Vec<String> = FOUR.map(String::from).to_vec();
    pinned.push(String::from("org.example.OtherBrowser"));
    config.pinned = Some(pinned);
    let _ = dock.update(Message::Config(Box::new(config)));
    assert_eq!(
        names(&dock),
        [
            "Fichiers",
            "Terminal d'exemple",
            "Éditeur",
            "Sherlock",
            "Example Browser",
            "Other Browser"
        ]
    );
    let t0 = dock.now();
    assert!(dock.animating());
    // At first nothing moved: the new icon has no room yet, every icon is where it was. The
    // surface grew: places are compared from the middle of the dock, which stays put.
    let start = dock.layout(t0);
    assert!(dock.surface_size.width > rest.surface.width);
    assert!((start.bar.width - rest.bar.width).abs() < 0.5);
    let along = |layout: &Layout, index: usize| layout.offset(layout.icons[index]).0;
    assert!(
        (along(&start, 3) - along(&rest, 2)).abs() < 0.5,
        "Sherlock where it was"
    );
    assert!(
        (along(&start, 2) - along(&rest, 3)).abs() < 0.5,
        "the editor too"
    );
    // Then they move without a jump, and the bar widens.
    let mut previous = start.clone();
    for step in 1..=40 {
        let layout = dock.layout(t0 + ms(16 * step));
        for (now, before) in layout.icons.iter().zip(&previous.icons) {
            assert!(
                (now.x - before.x).abs() < 20.0,
                "a jump at {} ms",
                16 * step
            );
        }
        previous = layout;
    }
    // Half way in the fade, the new icon is half drawn; at the end, everything is in place.
    let half = dock.layout(t0 + ms(80));
    assert!(half.bar.width > rest.bar.width + 10.0);
    let end = settle(&mut dock);
    let target = Layout::new(dock.geometry, dock.surface_size, &[52.0; 6], None);
    for (icon, place) in dock.layout(end).icons.iter().zip(&target.icons) {
        assert!((icon.x - place.x).abs() < 0.5 && (icon.y - place.y).abs() < 0.5);
    }
}

#[test]
fn an_unpinned_application_leaves_and_the_gap_closes() {
    let mut dock = dock(&[], &FOUR, DockConfig::default(), None);
    let t = settle(&mut dock);
    let rest = dock.layout(t);
    let surface = dock.surface_size;
    let _ = dock.unpin(0);
    let t0 = dock.now();
    assert_eq!(dock.items.len(), 4);
    assert_eq!(dock.leaving.len(), 1, "the files still drawn");
    // It leaves from its place; the bar keeps its length, then shrinks.
    let start = dock.layout(t0);
    assert!((start.leaving[0].x - rest.icons[0].x).abs() < 0.5);
    assert!((start.bar.width - rest.bar.width).abs() < 0.5);
    assert_eq!(dock.surface_size, surface, "the surface waits for it to go");
    let later = dock.layout(t0 + ms(120));
    assert!(later.bar.width < rest.bar.width - 20.0);
    let _ = dock.tick(t0 + ms(60));
    assert_eq!(dock.leaving.len(), 1, "still going");
    let end = settle(&mut dock);
    assert!(dock.leaving.is_empty());
    assert!(
        dock.surface_size.width < surface.width,
        "then the surface shrinks"
    );
    let target = Layout::new(dock.geometry, dock.surface_size, &[52.0; 4], None);
    assert!((dock.layout(end).bar.width - target.bar.width).abs() < 0.5);
}

#[test]
fn dragging_a_pinned_icon_reorders_and_writes_the_file() {
    let dir = scratch("reorder");
    let path = dir.join("dock.conf");
    let text = "# kept\n[Dock]\nPinned=org.example.Files;org.example.Terminal;\
                example-editor;example-browser;\nMagnification=false\n";
    std::fs::write(&path, text).expect("file written");
    let (config, _) = DockConfig::parse(text);
    let mut dock = dock(&[], &FOUR, config, Some(path.clone()));
    let t = settle(&mut dock);
    let rest = dock.layout(t);
    let files = middle(&rest, 0);
    press(&mut dock, files);
    assert!(dock.held().is_none(), "not dragged before the threshold");
    // Past the browser, at the right end: the files take the last place. Sherlock is one of
    // the pinned icons: it moves aside like the others.
    let end = Point::new(rest.icons[4].x + rest.icons[4].width + 6.0, files.y);
    drag_to(&mut dock, files, end);
    assert_eq!(
        names(&dock),
        [
            "Terminal d'exemple",
            "Sherlock",
            "Éditeur",
            "Example Browser",
            "Fichiers"
        ]
    );
    let held = dock.held();
    assert_eq!(held, Some((4, false)));
    // The icon is under the pointer; the others made room.
    let now = dock.layout(dock.now());
    assert!((now.icons[4].center_x() - end.x).abs() < 0.5);
    assert_eq!(
        std::fs::read_to_string(&path).ok().as_deref(),
        Some(text),
        "not yet"
    );
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert!(dock.held().is_none());
    // Written once let go, atomically by dock-config, the comment kept, Sherlock's place too:
    // on the dock's thread that saves, never on the interface's.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::fs::read_to_string(&path).ok().as_deref() == Some(text) {
        assert!(std::time::Instant::now() < deadline, "never written");
        std::thread::sleep(Duration::from_millis(10));
    }
    let written = std::fs::read_to_string(&path).expect("file read");
    let (read, _) = DockConfig::parse(&written);
    let order = [
        "org.example.Terminal",
        "sherlock",
        "example-editor",
        "example-browser",
        "org.example.Files",
    ];
    assert_eq!(read.pinned, Some(order.map(String::from).to_vec()));
    assert!(written.contains("# kept"));
    assert_eq!(dock.pinned, order);
    // It flies from the pointer to its place.
    let t0 = dock.now();
    let flying = dock.layout(t0).icons[4];
    assert!(
        (flying.center_x() - end.x).abs() < 0.5,
        "{flying:?} {end:?}"
    );
    let t1 = settle(&mut dock);
    let place = Layout::new(dock.geometry, dock.surface_size, &[52.0; 5], None).icons[4];
    assert!((dock.layout(t1).icons[4].x - place.x).abs() < 0.5);
    let leftovers = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
    assert_eq!(leftovers, 1, "no temporary file left");
}

#[test]
fn a_press_without_moving_is_a_click_and_starts_nothing_when_dragged() {
    let data = quick_app("quick-click");
    let mut dock = dock(&[data], &["org.example.Quick"], flat(), None);
    let t = settle(&mut dock);
    let icon = middle(&dock.layout(t), 0);
    // Pressed and let go in place: a click, which starts the application.
    press(&mut dock, icon);
    let _ = dock.update(Message::Gesture(Gesture::Moved(Point::new(
        icon.x + 2.0,
        icon.y,
    ))));
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert_eq!(dock.launches.len(), 1);
    settle_bounce(&mut dock);
    // Dragged away and back, then let go: no click.
    press(&mut dock, icon);
    drag_to(&mut dock, icon, Point::new(icon.x + 30.0, icon.y));
    drag_to(&mut dock, Point::new(icon.x + 30.0, icon.y), icon);
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert!(dock.launches.is_empty());
}

#[test]
fn dragging_an_icon_out_of_the_dock_unpins_it() {
    let dir = scratch("unpin");
    let path = dir.join("dock.conf");
    let mut dock = dock(&[], &FOUR, flat(), Some(path.clone()));
    let t = settle(&mut dock);
    let rest = dock.layout(t);
    let files = middle(&rest, 0);
    press(&mut dock, files);
    // Up, past the bar: out of the dock. Its room closes, it stays drawn under the pointer.
    let out = Point::new(files.x, 2.0);
    drag_to(&mut dock, files, out);
    assert_eq!(dock.held(), Some((0, true)));
    let t0 = dock.now();
    let _ = dock.tick(t0 + ms(400));
    let closed = dock.layout(t0 + ms(400));
    assert!(closed.bar.width < rest.bar.width - 50.0, "its room closed");
    assert!(closed.icons[0].y < rest.bar.y, "drawn above the bar");
    // Back over the dock, its room opens again; out again and let go: unpinned.
    drag_to(&mut dock, out, files);
    assert_eq!(dock.held(), Some((0, false)));
    drag_to(&mut dock, files, out);
    let _ = dock.update(Message::Gesture(Gesture::Released));
    // Sherlock keeps its place, written in the list, rather than going back to the middle.
    assert_eq!(
        dock.pinned,
        [
            "org.example.Terminal",
            "sherlock",
            "example-editor",
            "example-browser"
        ]
    );
    // Written on the dock's thread that saves, never on the interface's.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(std::time::Instant::now() < deadline, "never written");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (read, _) = DockConfig::parse(&std::fs::read_to_string(&path).expect("written"));
    assert_eq!(read.pinned.as_deref(), Some(&dock.pinned[..]));
    // It vanishes where it was let go: smaller and fainter, then gone.
    assert_eq!(dock.leaving.len(), 1);
    assert!(dock.leaving[0].dropped.is_some());
    let t1 = dock.now();
    assert!(dock.leaving[0].track.opacity.value(t1) <= tokens::DRAG_OUT_OPACITY);
    assert!(dock.leaving[0].track.drawn_scale(t1 + ms(100)) < 0.6);
    settle(&mut dock);
    assert!(dock.leaving.is_empty());
}

/// The file once the dock's thread that saves wrote it: its text when it is no longer `before`.
fn changed(path: &std::path::Path, before: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        if text != before {
            return text;
        }
        assert!(std::time::Instant::now() < deadline, "never written");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Lot D-21d: an open application dragged across the separator among the pinned ones is
/// pinned where it is let go; dragged back after it, it is unpinned; a pinned application let
/// go out of the dock goes up in smoke. The file is read again just before it is written.
#[test]
fn dragging_across_the_separator_pins_and_unpins() {
    let dir = scratch("cross");
    let path = dir.join("dock.conf");
    let mut dock = dock(
        &[],
        &["org.example.Files", "example-editor"],
        flat(),
        Some(path.clone()),
    );
    open(&mut dock, "terminal");
    let t = settle(&mut dock);
    assert_eq!(
        names(&dock),
        ["Fichiers", "Sherlock", "Éditeur", "Terminal d'exemple"]
    );
    assert_eq!(dock.separator, Some(3));
    // Another program changes the file meanwhile: the dock must not undo it.
    std::fs::write(
        &path,
        "[Dock]\nPinned=org.example.Files;example-editor;\nIconSize=40\n",
    )
    .expect("file written");

    // The terminal, after the separator, dragged to the second place.
    let rest = dock.layout(t);
    let terminal = middle(&rest, 3);
    // Over the left half of Sherlock: the second place.
    let between = Point::new(rest.icons[1].center_x() - 8.0, terminal.y);
    press(&mut dock, terminal);
    drag_to(&mut dock, terminal, between);
    assert_eq!(dock.separator, None, "the separator went with it");
    assert_eq!(
        names(&dock),
        ["Fichiers", "Terminal d'exemple", "Sherlock", "Éditeur"],
        "pinned where the pointer is"
    );
    assert_eq!(
        dock.held().map(|(i, _)| i),
        Some(1),
        "it follows the pointer"
    );
    let _ = dock.update(Message::Gesture(Gesture::Released));
    // Written on the dock's thread that saves, never on the interface's.
    let written = changed(
        &path,
        "[Dock]\nPinned=org.example.Files;example-editor;\nIconSize=40\n",
    );
    let (read, _) = DockConfig::parse(&written);
    // Sherlock's place is written with the others (lot D-21h).
    let order = [
        "org.example.Files",
        "org.example.Terminal",
        "sherlock",
        "example-editor",
    ];
    assert_eq!(read.pinned, Some(order.map(String::from).to_vec()));
    assert_eq!(read.icon_size, 40.0, "what Control Room wrote stays");
    let t = settle(&mut dock);

    // Dragged back past the last icon: open, it is unpinned, after a new separator.
    let rest = dock.layout(t);
    let terminal = middle(&rest, 1);
    let end = Point::new(rest.icons[3].x + rest.icons[3].width + 40.0, terminal.y);
    press(&mut dock, terminal);
    drag_to(&mut dock, terminal, end);
    assert_eq!(dock.separator, Some(3));
    assert_eq!(names(&dock)[3], "Terminal d'exemple");
    let _ = dock.update(Message::Gesture(Gesture::Released));
    let (read, _) = DockConfig::parse(&changed(&path, &written));
    let order = ["org.example.Files", "sherlock", "example-editor"];
    assert_eq!(read.pinned, Some(order.map(String::from).to_vec()));
    let t = settle(&mut dock);

    // An open application that is not pinned, let go out of the dock: it comes back, no smoke.
    let rest = dock.layout(t);
    let terminal = middle(&rest, 3);
    press(&mut dock, terminal);
    drag_to(&mut dock, terminal, Point::new(terminal.x, 2.0));
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert!(dock.poofs.is_empty());
    assert_eq!(dock.separator, Some(3));
    let t = settle(&mut dock);

    // The files, pinned and closed, let go out of the dock: unpinned, in a puff of smoke.
    let files = middle(&dock.layout(t), 0);
    press(&mut dock, files);
    drag_to(&mut dock, files, Point::new(files.x, 2.0));
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert_eq!(dock.pinned, ["sherlock", "example-editor"]);
    assert_eq!(dock.poofs.len(), 1);
    assert!(dock.animating(), "frames while it smokes");
    settle(&mut dock);
    assert!(dock.poofs.is_empty(), "gone");
}

#[test]
fn only_pinned_applications_can_be_dragged_out() {
    let mut dock = dock(&[], &FOUR, flat(), None);
    open(&mut dock, "terminal,mystery");
    let t = settle(&mut dock);
    assert_eq!(dock.separator, Some(5));
    let rest = dock.layout(t);
    // An open application that is not pinned does not follow the pointer.
    let mystery = middle(&rest, 5);
    press(&mut dock, mystery);
    drag_to(&mut dock, mystery, Point::new(mystery.x, 2.0));
    assert!(dock.held().is_none(), "dragged");
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert_eq!(dock.items.len(), 6);
    // A pinned application that is open moves after the separator, without vanishing.
    let terminal = middle(&rest, 1);
    press(&mut dock, terminal);
    drag_to(&mut dock, terminal, Point::new(terminal.x, 2.0));
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert!(dock.leaving.is_empty());
    assert_eq!(
        names(&dock),
        [
            "Fichiers",
            "Sherlock",
            "Éditeur",
            "Example Browser",
            "Terminal d'exemple",
            "mystery"
        ]
    );
    assert_eq!(dock.separator, Some(4));
    settle(&mut dock);
}

/// A data folder with an application that starts and exits at once (`true`), whose windows
/// would be `QuickExample`.
fn quick_app(name: &str) -> PathBuf {
    let dir = scratch(name).join("applications");
    assert!(std::fs::create_dir_all(&dir).is_ok(), "folder");
    let entry = "[Desktop Entry]\nType=Application\nName=Rapide\nExec=true\n\
                 StartupWMClass=QuickExample\n";
    let written = std::fs::write(dir.join("org.example.Quick.desktop"), entry);
    assert!(written.is_ok(), "entry");
    dir.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// Two applications that start at once and do nothing (`Exec=true`).
fn quick_pair(name: &str) -> PathBuf {
    let dir = scratch(name).join("applications");
    assert!(std::fs::create_dir_all(&dir).is_ok(), "folder");
    for letter in ["A", "B"] {
        let entry = format!("[Desktop Entry]\nType=Application\nName=Rapide {letter}\nExec=true\n");
        let path = dir.join(format!("org.example.Quick{letter}.desktop"));
        assert!(std::fs::write(path, entry).is_ok(), "entry");
    }
    dir.parent().map(Path::to_path_buf).unwrap_or_default()
}

/// A click at `point`: the icon whose application it started, if any.
fn clicked(dock: &mut Dock, point: Point) -> Option<usize> {
    press(dock, point);
    let _ = dock.update(Message::Gesture(Gesture::Released));
    let launched = dock.launches.first().map(|l| l.target.clone());
    if launched.is_some() {
        settle_bounce(dock);
    }
    launched.and_then(|target| dock.items.iter().position(|i| i.target() == target))
}

/// Lot D-21f: Fitts's law, as on macOS. The gap between the glass and the screen edge and the
/// spaces between the icons are the dock: a click pushed against the bottom edge, or 2 px
/// above it, starts the icon of its column, and one between two icons the closer one.
#[test]
fn a_click_against_the_screen_edge_starts_the_icon_of_its_column() {
    let data = quick_pair("fitts-bottom");
    let pinned = ["org.example.QuickA", "org.example.QuickB"];
    let mut dock = dock(&[data], &pinned, flat(), None);
    let t = settle(&mut dock);
    let layout = dock.layout(t);
    // The first application, Sherlock, the second one.
    let (first, sherlock, last) = (layout.icons[0], layout.icons[1], layout.icons[2]);
    let bottom = dock.surface_size.height;
    assert!(
        bottom - first.y - first.height > 2.0,
        "a gap below the glass"
    );
    // Below the icons, against the edge and 2 px above it.
    assert_eq!(
        clicked(&mut dock, Point::new(first.center_x(), bottom)),
        Some(0)
    );
    assert_eq!(
        clicked(&mut dock, Point::new(last.center_x(), bottom - 2.0)),
        Some(2)
    );
    // In the space between two icons, 1 px from either.
    let row = first.center_y();
    let gap = (first.x + first.width, sherlock.x);
    assert!(gap.1 - gap.0 >= 4.0, "{gap:?}");
    assert_eq!(clicked(&mut dock, Point::new(gap.0 + 1.0, row)), Some(0));
    assert_eq!(layout.icon_for(Point::new(gap.1 - 1.0, row)), Some(1));
    let gap = (sherlock.x + sherlock.width, last.x);
    assert_eq!(layout.icon_for(Point::new(gap.0 + 1.0, row)), Some(1));
    assert_eq!(clicked(&mut dock, Point::new(gap.1 - 1.0, row)), Some(2));
    // The surface takes the pointer down to the edge, and the dock counts it as on itself
    // there: the magnification and the label follow the column.
    let edge = Point::new(last.center_x(), bottom - 0.5);
    assert!(dock.input_area().contains(edge), "{:?}", dock.input_area());
    assert!(layout.contains(edge));
    // Past the ends of the glass, nothing.
    let past = Point::new(layout.bar.x - 4.0, bottom);
    assert_eq!(clicked(&mut dock, past), None);
}

/// Lot D-21f: the same on the left edge, a click pushed against it.
#[test]
fn a_click_against_the_left_edge_starts_the_icon_of_its_row() {
    let data = quick_pair("fitts-left");
    let pinned = ["org.example.QuickA", "org.example.QuickB"];
    let mut config = flat();
    config.position = dock_config::Position::Left;
    let mut dock = dock(&[data], &pinned, config, None);
    let t = settle(&mut dock);
    // The first application, Sherlock, the second one.
    let last = dock.layout(t).icons[2];
    assert!(last.x > 2.0, "a gap beside the glass");
    assert_eq!(
        clicked(&mut dock, Point::new(0.0, last.center_y())),
        Some(2)
    );
}

fn shell(app_ids: &[&str]) -> Message {
    Message::Windows(Windows {
        list: app_ids
            .iter()
            .zip(1..)
            .map(|(a, id)| Window::new(id, a, false))
            .collect(),
        ..Windows::default()
    })
}

/// Lets a bounce end: its timeout, then its last hop.
fn settle_bounce(dock: &mut Dock) {
    let t = dock.now() + tokens::BOUNCE_TIMEOUT;
    let _ = dock.tick(t);
    let _ = dock.tick(t + tokens::BOUNCE_PERIOD);
    assert!(dock.launches.is_empty());
}

#[test]
fn the_icon_bounces_until_the_first_window_comes() {
    let data = quick_app("quick-bounce");
    let mut dock = dock(
        &[data],
        &["org.example.Files", "org.example.Quick"],
        flat(),
        None,
    );
    let _ = dock.update(shell(&["files"]));
    let t = settle(&mut dock);
    let index = dock
        .items
        .iter()
        .position(|i| i.name == "Rapide")
        .expect("pinned");
    assert_eq!(dock.lift(index, t), 0.0);
    dock.activate(index, dock::launch::What::App);
    let t0 = dock.now();
    assert_eq!(dock.launches.len(), 1);
    assert!(dock.animating(), "frames while it bounces");
    // Half a hop up: half the icon high.
    let period = tokens::BOUNCE_PERIOD;
    assert!((dock.lift(index, t0 + period / 2) - 26.0).abs() < 0.01);
    assert_eq!(dock.lift(index, t0 + period), 0.0, "down between two hops");
    // Clicked again while it bounces: not started twice.
    dock.click(index);
    assert_eq!(dock.launches.len(), 1);
    // Other windows change: it goes on.
    let _ = dock.tick(t0 + period * 2);
    let _ = dock.update(shell(&["files", "other"]));
    assert!(!dock.launches[0].bounce.stopping());
    // Its window comes in the middle of a hop: the hop ends, then it rests.
    let _ = dock.tick(t0 + period * 3 + period / 2);
    let _ = dock.update(shell(&["files", "other", "QuickExample"]));
    assert!(dock.launches[0].bounce.stopping());
    let landing = t0 + period * 4;
    assert!(dock.lift(index, landing - ms(100)) > 0.0);
    let _ = dock.tick(landing);
    assert!(dock.launches.is_empty());
    assert_eq!(dock.lift(index, landing), 0.0);
    assert!(!dock.animating(), "no frame is asked for at rest");
}

#[test]
fn the_icon_stops_bouncing_at_the_timeout() {
    let data = quick_app("quick-timeout");
    let mut dock = dock(&[data], &["org.example.Quick"], flat(), None);
    settle(&mut dock);
    dock.activate(0, dock::launch::What::App);
    let t0 = dock.now();
    let _ = dock.tick(t0 + tokens::BOUNCE_TIMEOUT - ms(1));
    assert!(!dock.launches[0].bounce.stopping());
    let _ = dock.tick(t0 + tokens::BOUNCE_TIMEOUT);
    assert!(dock.launches[0].bounce.stopping());
    let _ = dock.tick(t0 + tokens::BOUNCE_TIMEOUT + tokens::BOUNCE_PERIOD);
    assert!(dock.launches.is_empty());
    assert!(!dock.animating());
}

/// A capture sets the active theme: one at a time.
static THEME: Mutex<()> = Mutex::new(());

fn in_dark() -> MutexGuard<'static, ()> {
    let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(Theme::Dark);
    guard
}

fn capture(dock: &Dock, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let size = (dock.surface_size.width, dock.surface_size.height);
    let element: Element<'_, Message> = container(dock.view_dock())
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::desktop)
        .into();
    let mut ui = Simulator::with_size(dock::settings(), size, element);
    let snapshot = ui.snapshot(&theme::theme())?;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let written = path.with_file_name(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    snapshot.matches_image(&path)?;
    Ok(std::fs::metadata(&written)?.len())
}

#[test]
fn the_animations_render_without_a_screen() {
    let _theme = in_dark();
    // An icon at the top of its hop.
    let data = quick_app("quick-render");
    let pinned = [
        "org.example.Files",
        "org.example.Terminal",
        "org.example.Quick",
    ];
    let mut dock = dock(&[data], &pinned, flat(), None);
    settle(&mut dock);
    let index = dock
        .items
        .iter()
        .position(|i| i.name == "Rapide")
        .expect("pinned");
    dock.activate(index, dock::launch::What::App);
    let _ = dock.tick(dock.now() + tokens::BOUNCE_PERIOD / 2);
    assert!(capture(&dock, "dock-bounce").expect("screenshot written") > 2_000);
    // An icon dragged above the dock, between two others that made room.
    let mut dock = self::dock(&[], &FOUR, DockConfig::default(), None);
    let t = settle(&mut dock);
    let rest = dock.layout(t);
    let files = middle(&rest, 0);
    press(&mut dock, files);
    let between = Point::new(rest.icons[3].x - 4.0, rest.bar.y - 10.0);
    drag_to(&mut dock, files, between);
    let _ = dock.tick(dock.now() + ms(400));
    assert!(dock.held().is_some());
    assert!(capture(&dock, "dock-drag").expect("screenshot written") > 2_000);
    color::set_active(Theme::Dark);
}

/// An application without an icon, its initial drawn instead, arriving: at its first frame
/// its side is 0. Text of size 0 made the text engine panic (`cosmic-text`, `buffer.rs`: « line
/// height cannot be 0 »), which killed the dock at the start of a session without screen,
/// where no theme gives the applications their icons (lot Q-32).
#[test]
fn an_initial_arriving_from_nothing_is_drawn() {
    let _theme = in_dark();
    let data = quick_app("quick-arrives");
    let mut dock = dock(&[data], &FOUR, flat(), None);
    settle(&mut dock);
    let mut config = flat();
    let mut pinned: Vec<String> = FOUR.map(String::from).to_vec();
    pinned.push(String::from("org.example.Quick"));
    config.pinned = Some(pinned);
    let _ = dock.update(Message::Config(Box::new(config)));
    assert!(dock.animating(), "it arrives");
    assert!(capture(&dock, "dock-initial-arrives").expect("screenshot written") > 2_000);
    color::set_active(Theme::Dark);
}

#[test]
fn the_first_list_of_windows_is_in_place_at_once() {
    let mut dock = dock(&[], &FOUR, DockConfig::default(), None);
    open(&mut dock, "mystery");
    assert_eq!(dock.items.len(), 6);
    assert!(!dock.animating(), "the dock at startup does not animate");
    // The next lists do.
    open(&mut dock, "mystery,other");
    assert!(dock.animating());
}

#[test]
fn a_drag_back_to_its_place_writes_nothing() {
    let dir = scratch("unchanged");
    let path = dir.join("dock.conf");
    // A pinned application that is not installed sits between two shown ones.
    let pinned = [
        "org.example.Files",
        "org.example.Missing",
        "org.example.Terminal",
        "example-editor",
        "example-browser",
    ];
    let mut dock = dock(&[], &pinned, flat(), Some(path.clone()));
    let t = settle(&mut dock);
    let rest = dock.layout(t);
    let files = middle(&rest, 0);
    press(&mut dock, files);
    // Past the terminal, then back to its place.
    let past = Point::new(middle(&rest, 1).x + 10.0, files.y);
    drag_to(&mut dock, files, past);
    assert_eq!(dock.items[1].name, "Fichiers", "it moved on screen");
    drag_to(&mut dock, past, files);
    assert_eq!(dock.items[0].name, "Fichiers");
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert_eq!(dock.pinned, pinned, "the hidden pin did not move");
    assert!(!path.exists(), "nothing written");
}
