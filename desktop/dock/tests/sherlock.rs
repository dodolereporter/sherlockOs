//! Sherlock as a pinned icon (lot D-21h): moved anywhere among the pinned applications, its
//! place written in the pinned list, taken out of the dock from its menu or by a drag, put back
//! from the dock's own menu; an old file without its place keeps it in the middle, unwritten.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use dock::app::{Dock, Message, Pointer, Sources};
use dock::drag::Gesture;
use dock::entry::Catalog;
use dock::icons::IconLookup;
use dock::menu::Entry;
use dock::windows::Recorder;
use dock_config::DockConfig;
use iced::window::Id;
use iced::{Point, mouse};

/// The four pinned applications of the fixtures, Magnification off: the places do not depend
/// on where the pointer is.
const OLD: &str = "[Dock]\nPinned=org.example.Files;org.example.Terminal;example-editor;\
                   example-browser;\nMagnification=false\n";

const SHERLOCK: usize = 2;

/// The file `text` in a folder of its own, and the dock that read it.
fn dock(name: &str, text: &str) -> (Dock, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-sherlock"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(std::fs::create_dir_all(&dir).is_ok(), "scratch folder");
    let path = dir.join("dock.conf");
    assert!(std::fs::write(&path, text).is_ok(), "file written");
    let (config, _) = DockConfig::parse(text);
    let data = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data")];
    let sources = Sources {
        catalog: Catalog::load(&data),
        lookup: IconLookup::new(&data, None, Some("Exemple")),
        pinned: config.pinned.clone().unwrap_or_default(),
        config,
        config_path: Some(path.clone()),
        data_dirs: data,
        home: None,
        gtk_theme: None,
        control: Arc::new(Recorder::default()),
        applications: false,
    };
    let mut dock = Dock::new(sources);
    let _ = dock.update(Message::Surface(Id::unique()));
    settle(&mut dock);
    (dock, path)
}

fn settle(dock: &mut Dock) {
    let _ = dock.tick(dock.now() + Duration::from_secs(3));
    assert!(!dock.animating(), "no frame is asked for at rest");
}

fn names(dock: &Dock) -> Vec<&str> {
    dock.items.iter().map(|i| i.name.as_str()).collect()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The file once the dock's thread that saves wrote it: its text when it is no longer `before`.
fn changed(path: &Path, before: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let text = read(path);
        if text != before {
            return text;
        }
        assert!(std::time::Instant::now() < deadline, "never written");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn center(dock: &Dock, index: usize) -> Point {
    dock.layout(dock.now()).icons[index].center()
}

/// Presses the left button on `from` and drags to `to` in small steps, without letting go.
fn drag(dock: &mut Dock, from: Point, to: Point) {
    let _ = dock.update(Message::Pointer(Pointer::Moved(from)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Left)));
    steps(dock, from, to);
}

/// The pointer moves from `from` to `to` in small steps, the button held.
fn steps(dock: &mut Dock, from: Point, to: Point) {
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        let point = Point::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        let _ = dock.update(Message::Gesture(Gesture::Moved(point)));
    }
}

fn right_click(dock: &mut Dock, at: Point) {
    let _ = dock.update(Message::Pointer(Pointer::Moved(at)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
}

#[test]
fn sherlock_moves_among_the_pinned_applications_and_its_place_is_written() {
    let (mut dock, path) = dock("move", OLD);
    let rest = dock.layout(dock.now());
    let files = rest.icons[0];
    // To the left end, past the files.
    let sherlock = center(&dock, SHERLOCK);
    drag(&mut dock, sherlock, Point::new(files.x - 4.0, sherlock.y));
    assert_eq!(dock.held(), Some((0, false)), "it follows the pointer");
    assert_eq!(
        names(&dock),
        [
            "Sherlock",
            "Fichiers",
            "Terminal d'exemple",
            "Éditeur",
            "Example Browser"
        ]
    );
    // The others move aside with the spring, from where they were drawn.
    assert!(dock.animating());
    let now = dock.layout(dock.now());
    assert!(
        (now.icons[1].x - files.x).abs() < 0.5,
        "the files start in place"
    );
    assert_eq!(read(&path), OLD, "nothing written before it is let go");
    let _ = dock.update(Message::Gesture(Gesture::Released));
    let order = [
        "sherlock",
        "org.example.Files",
        "org.example.Terminal",
        "example-editor",
        "example-browser",
    ];
    assert_eq!(dock.pinned, order);
    let (written, _) = DockConfig::parse(&changed(&path, OLD));
    assert_eq!(written.pinned, Some(order.map(String::from).to_vec()));
    assert!(written.sherlock && !written.magnification);
    settle(&mut dock);
    // Read again, the dock keeps it there.
    let _ = dock.update(Message::Config(Box::new(written)));
    assert_eq!(names(&dock)[0], "Sherlock");
}

#[test]
fn an_old_file_keeps_sherlock_in_the_middle_without_being_written() {
    let (mut dock, path) = dock("old", OLD);
    assert_eq!(names(&dock)[SHERLOCK], "Sherlock");
    assert!(!dock.pinned.iter().any(|p| p == "sherlock"));
    // A click, a drag back to its place, a menu opened and closed, the file read again:
    // nothing written.
    let sherlock = center(&dock, SHERLOCK);
    drag(
        &mut dock,
        sherlock,
        Point::new(sherlock.x + 2.0, sherlock.y),
    );
    let _ = dock.update(Message::Gesture(Gesture::Released));
    let editor = center(&dock, 3);
    drag(&mut dock, editor, Point::new(editor.x + 30.0, editor.y));
    steps(&mut dock, Point::new(editor.x + 30.0, editor.y), editor);
    let _ = dock.update(Message::Gesture(Gesture::Released));
    right_click(&mut dock, sherlock);
    assert!(dock.menu_open());
    let _ = dock.update(Message::Key(dock::menu::Key::Close));
    let (config, _) = DockConfig::parse(OLD);
    let _ = dock.update(Message::Config(Box::new(config)));
    settle(&mut dock);
    assert_eq!(read(&path), OLD);
    assert_eq!(names(&dock)[SHERLOCK], "Sherlock");
    assert!(!dock.pinned.iter().any(|p| p == "sherlock"));
}

#[test]
fn sherlock_leaves_the_dock_and_comes_back() {
    let (mut dock, path) = dock("leave", OLD);
    // Its menu now has « Retirer du Dock ».
    let lines = dock.menu_lines(SHERLOCK);
    assert!(lines.iter().any(|l| l.entry == Entry::Unpin && l.enabled));
    let sherlock = center(&dock, SHERLOCK);
    right_click(&mut dock, sherlock);
    let _ = dock.update(Message::MenuPick(Entry::Unpin));
    assert!(!dock.sherlock);
    assert!(!names(&dock).contains(&"Sherlock"));
    assert_eq!(
        dock.leaving.len(),
        1,
        "it leaves with the others closing up"
    );
    let left = changed(&path, OLD);
    let (written, _) = DockConfig::parse(&left);
    assert!(!written.sherlock);
    assert!(
        !written
            .pinned
            .unwrap_or_default()
            .iter()
            .any(|p| p == "sherlock")
    );
    settle(&mut dock);
    // The dock's own menu, from its bar between two icons, puts it back in the middle.
    let layout = dock.layout(dock.now());
    let gap = Point::new(
        (layout.icons[1].x + layout.icons[1].width + layout.icons[2].x) / 2.0,
        layout.bar.center_y(),
    );
    assert!(layout.icon_at(gap).is_none() && layout.bar.contains(gap));
    right_click(&mut dock, gap);
    assert!(dock.menu_open(), "the dock's menu");
    let _ = dock.update(Message::MenuPick(Entry::ShowSherlock));
    assert!(dock.sherlock);
    assert_eq!(names(&dock)[SHERLOCK], "Sherlock");
    let back = changed(&path, &left);
    let (written, _) = DockConfig::parse(&back);
    assert!(written.sherlock);
    assert_eq!(
        written.pinned.as_deref().map(|p| p[SHERLOCK].as_str()),
        Some("sherlock")
    );
    settle(&mut dock);
    // Dragged out of the dock and let go: it vanishes where it was let go.
    let sherlock = center(&dock, SHERLOCK);
    drag(&mut dock, sherlock, Point::new(sherlock.x, 2.0));
    assert_eq!(dock.held(), Some((SHERLOCK, true)));
    let _ = dock.update(Message::Gesture(Gesture::Released));
    assert!(!dock.sherlock && dock.items.len() == 4);
    assert!(dock.leaving.iter().any(|l| l.dropped.is_some()));
    assert!(!DockConfig::parse(&changed(&path, &back)).0.sherlock);
    settle(&mut dock);
}

#[test]
fn a_change_made_by_another_program_meanwhile_is_kept() {
    let (mut dock, path) = dock("meanwhile", OLD);
    // Control Room changes the icon size before the dock reads the file again.
    let changed_by_hand = "[Dock]\n# by hand\nPinned=org.example.Files;org.example.Terminal;\
                   example-editor;example-browser;\nMagnification=false\nIconSize=64\n";
    assert!(std::fs::write(&path, changed_by_hand).is_ok());
    let sherlock = center(&dock, SHERLOCK);
    let end = dock.layout(dock.now()).icons[4];
    drag(
        &mut dock,
        sherlock,
        Point::new(end.x + end.width + 4.0, sherlock.y),
    );
    let _ = dock.update(Message::Gesture(Gesture::Released));
    let text = changed(&path, changed_by_hand);
    assert!(
        text.contains("# by hand") && text.contains("IconSize=64"),
        "{text}"
    );
    let (written, _) = DockConfig::parse(&text);
    assert_eq!(written.icon_size, 64.0);
    assert_eq!(
        written
            .pinned
            .as_deref()
            .and_then(<[String]>::last)
            .map(String::as_str),
        Some("sherlock")
    );
}
