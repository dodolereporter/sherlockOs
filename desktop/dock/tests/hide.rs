//! `AutoHide` without a screen: the dock gives up its zone and slides out while a maximized
//! window is on its screen, comes back at the edge, hides again, comes back when the window is
//! restored; on the four edges. And the rectangle of each icon on its screen, for the minimize
//! animation.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use dock::app::{Dock, Message, Pointer, Screen, Sources};
use dock::entry::Catalog;
use dock::icons::IconLookup;
use dock::windows::{Placed, Placeholder, Recorder};
use dock_config::{AutoHide, DockConfig, Position};
use iced::window::Id;
use iced::{Point, Rectangle};
use sherlock_ui::dock as tokens;

const SCREEN: &str = "eDP-1";
const SIZE: (i32, i32) = (1440, 900);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data")
}

/// The dock on `SCREEN`, at `position`, with the terminal open, acting through `recorder`.
fn dock(position: Position, mode: AutoHide, recorder: &Recorder) -> Dock {
    let data = fixtures();
    let mut config = DockConfig::default();
    config.position = position;
    config.auto_hide = mode;
    let sources = Sources {
        catalog: Catalog::load(std::slice::from_ref(&data)),
        lookup: IconLookup::new(std::slice::from_ref(&data), None, Some("Exemple")),
        config,
        config_path: None,
        pinned: ["org.example.Files", "org.example.Terminal"]
            .map(String::from)
            .to_vec(),
        data_dirs: vec![data],
        home: None,
        gtk_theme: None,
        control: Arc::new(recorder.clone()),
        applications: false,
    };
    let mut dock = Dock::new(sources);
    let id = Id::unique();
    let _ = dock.update(Message::Surface(id));
    let _ = dock.update(Message::Screen(
        id,
        Some(Screen {
            name: Some(String::from(SCREEN)),
            size: Some(SIZE),
        }),
    ));
    let _ = dock.update(windows(&[]));
    settle(&mut dock);
    dock
}

/// The terminal open, and maximized on the screens of `covered`.
fn windows(covered: &[&str]) -> Message {
    let Placeholder(mut windows) = Placeholder::new("terminal", Some("terminal"));
    windows.controllable = true;
    windows.covered = covered.iter().map(|s| String::from(*s)).collect();
    Message::Windows(windows)
}

/// Lets everything end: a wait that ends (the pointer resting on the band, lot D-21d) may
/// start a slide, which the next tick lets end.
fn settle(dock: &mut Dock) {
    for _ in 0..2 {
        let end = dock.now() + Duration::from_secs(3);
        let _ = dock.tick(end);
    }
    assert!(!dock.animating(), "at rest");
}

/// Depth of the surface, away from its edge.
fn depth(dock: &Dock, position: Position) -> f32 {
    if position.is_vertical() {
        dock.surface_size.width
    } else {
        dock.surface_size.height
    }
}

/// Distance from the edge given to the surface.
fn margin(dock: &Dock, position: Position) -> i32 {
    let margin = dock.layer_settings().margin;
    match position {
        Position::Bottom => margin.bottom,
        Position::Top => margin.top,
        Position::Left => margin.left,
        Position::Right => margin.right,
    }
}

/// What the compositor keeps clear along the edge: the zone and the margin (`docs/compositor.md`).
fn reserved(dock: &Dock, position: Position) -> i32 {
    let zone = dock.layer_settings().exclusive_zone;
    if zone > 0 {
        zone + margin(dock, position)
    } else {
        0
    }
}

const EDGES: [Position; 4] = [
    Position::Bottom,
    Position::Top,
    Position::Left,
    Position::Right,
];

#[test]
fn a_maximized_window_takes_the_room_of_the_dock_on_every_edge() {
    for position in EDGES {
        let mut dock = dock(position, AutoHide::Smart, &Recorder::default());
        // Shown, over the windows (lot D-21g): nothing kept clear; the surface is against the
        // edge, the gap inside it.
        assert_eq!(reserved(&dock, position), 0, "{position:?}");
        assert_eq!(margin(&dock, position), 0);
        let rest = dock.input_area();

        // A window is maximized on the dock's screen: the dock slides out.
        let _ = dock.update(windows(&[SCREEN]));
        assert_eq!(reserved(&dock, position), 0, "{position:?}");
        // -1: placed against the screen edge itself, not below the top bar's zone.
        assert_eq!(dock.layer_settings().exclusive_zone, -1);
        assert!(dock.animating(), "it slides");
        settle(&mut dock);
        let hidden = tokens::HIDE_STRIP as i32 - depth(&dock, position).round() as i32;
        assert_eq!(margin(&dock, position), hidden, "{position:?}");
        // Only the band that stays on the screen takes the pointer: the side away from the
        // edge, over the whole surface.
        let strip = dock.input_area();
        let size = dock.surface_size;
        let expected = match position {
            Position::Bottom => Rectangle::new(Point::ORIGIN, iced::Size::new(size.width, 2.0)),
            Position::Top => Rectangle::new(
                Point::new(0.0, size.height - 2.0),
                iced::Size::new(size.width, 2.0),
            ),
            Position::Left => Rectangle::new(
                Point::new(size.width - 2.0, 0.0),
                iced::Size::new(2.0, size.height),
            ),
            Position::Right => Rectangle::new(Point::ORIGIN, iced::Size::new(2.0, size.height)),
        };
        assert_eq!(strip, expected, "{position:?}");

        // Restored: back, still over the windows.
        let _ = dock.update(windows(&[]));
        assert_eq!(reserved(&dock, position), 0);
        settle(&mut dock);
        assert_eq!(reserved(&dock, position), 0);
        assert_eq!(margin(&dock, position), 0);
        assert_eq!(dock.input_area(), rest);
    }
}

#[test]
fn a_window_on_another_screen_changes_nothing() {
    let mut dock = dock(Position::Bottom, AutoHide::Smart, &Recorder::default());
    let _ = dock.update(windows(&["HDMI-1"]));
    assert!(!dock.covered());
    assert!(!dock.animating(), "it stays");
}

/// Lot D-21g: as on macOS, a dock that hides by itself never reserves room. In the `smart`
/// and `always` modes, the zone stays -1 through every state: shown, covered, hidden, brought
/// back by the pointer, hidden again, uncovered, and every frame of the slides in between. So
/// a maximized window is never resized by the dock. Only `never` keeps its room.
#[test]
fn a_dock_that_hides_never_reserves_room() {
    for mode in [AutoHide::Smart, AutoHide::Always] {
        let mut dock = dock(Position::Bottom, mode, &Recorder::default());
        let mut zones = vec![dock.layer_settings().exclusive_zone];
        let mut step = |dock: &mut Dock, message: Option<Message>| {
            if let Some(message) = message {
                let _ = dock.update(message);
            }
            // Every frame of the slide, not only its end.
            let start = dock.now();
            for ms in (0..2000).step_by(16) {
                let _ = dock.tick(start + Duration::from_millis(ms));
                zones.push(dock.layer_settings().exclusive_zone);
            }
        };
        step(&mut dock, Some(windows(&[SCREEN])));
        step(
            &mut dock,
            Some(Message::Pointer(Pointer::Moved(Point::new(300.0, 1.0)))),
        );
        step(&mut dock, Some(Message::Pointer(Pointer::Left)));
        step(&mut dock, Some(windows(&[])));
        step(&mut dock, None);
        assert!(zones.iter().all(|z| *z == -1), "{mode:?}: {zones:?}");
    }
    let never = dock(Position::Bottom, AutoHide::Never, &Recorder::default());
    assert_eq!(reserved(&never, Position::Bottom), 84);
}

#[test]
fn the_edge_brings_the_dock_back_over_the_window() {
    let mut dock = dock(Position::Bottom, AutoHide::Smart, &Recorder::default());
    let _ = dock.update(windows(&[SCREEN]));
    settle(&mut dock);
    // The pointer touches the band.
    let _ = dock.update(Message::Pointer(Pointer::Moved(Point::new(300.0, 1.0))));
    settle(&mut dock);
    assert_eq!(margin(&dock, Position::Bottom), 0, "back");
    assert_eq!(reserved(&dock, Position::Bottom), 0, "over the window");
    assert_ne!(
        dock.input_area().height,
        2.0,
        "the whole bar takes the pointer"
    );

    // It leaves: the dock stays for the delay, then hides.
    let _ = dock.update(Message::Pointer(Pointer::Left));
    let left = dock.now();
    let _ = dock.tick(left + tokens::HIDE_DELAY - Duration::from_millis(20));
    assert_eq!(margin(&dock, Position::Bottom), 0);
    let _ = dock.tick(left + tokens::HIDE_DELAY);
    assert!(dock.animating());
    settle(&mut dock);
    assert!(margin(&dock, Position::Bottom) < 0, "hidden again");
    assert_eq!(dock.input_area().height, 2.0);
}

#[test]
fn always_and_never() {
    let dock_always = dock(Position::Bottom, AutoHide::Always, &Recorder::default());
    assert_eq!(reserved(&dock_always, Position::Bottom), 0);
    assert!(
        margin(&dock_always, Position::Bottom) < 0,
        "hidden from the start"
    );
    let mut never = dock(Position::Bottom, AutoHide::Never, &Recorder::default());
    let _ = never.update(windows(&[SCREEN]));
    settle(&mut never);
    assert_eq!(reserved(&never, Position::Bottom), 84);
}

#[test]
fn each_window_gets_the_rectangle_of_its_icon_on_the_screen() {
    let recorder = Recorder::probing(&["terminal", "files"]);
    let mut dock = dock(Position::Bottom, AutoHide::Smart, &recorder);
    let places = recorder.places();
    let last = places.last().expect("placed");
    // The files, Sherlock, the terminal: the icons at rest, the surface centered at the bottom,
    // 8 above the edge.
    assert_eq!(last.len(), 3);
    let size = dock.surface_size;
    let left = SIZE.0 / 2 - size.width.round() as i32 / 2;
    let top = SIZE.1 - size.height.round() as i32;
    let icon = dock.layout(dock.now()).icons[2];
    assert_eq!(
        last[2],
        Placed {
            app_id: Some(String::from("terminal")),
            output: String::from(SCREEN),
            rectangle: (left + icon.x as i32, top + icon.y as i32, 52, 52),
        }
    );
    assert_eq!(last[0].app_id.as_deref(), Some("files"));
    assert_eq!(
        last[1].app_id, None,
        "Sherlock, in the middle, owns no window"
    );

    // The pointer magnifies the icons, the dock hides: the places at rest do not change.
    let count = places.len();
    let _ = dock.update(Message::Pointer(Pointer::Moved(icon.center())));
    let _ = dock.update(windows(&[SCREEN]));
    settle(&mut dock);
    assert_eq!(recorder.places().len(), count, "nothing sent again");

    // Lot D-21d: a click minimizes the terminal under the pointer: the compositor first gets
    // the icons as drawn, the terminal magnified, so that the genie lands on it.
    let Placeholder(mut open) = Placeholder::new("terminal", Some("terminal"));
    open.controllable = true;
    let _ = dock.update(Message::Windows(open));
    let _ = dock.update(Message::Pointer(Pointer::Moved(icon.center())));
    dock.frame(dock.now() + Duration::from_secs(2));
    let before = recorder.places().len();
    let magnified = dock.layout(dock.now()).icons[2];
    assert!(magnified.width > 80.0, "magnified under the pointer");
    dock.click(2);
    let places = recorder.places();
    assert_eq!(
        places.len(),
        before + 1,
        "sent as drawn, before the request"
    );
    let sent = places[before][2].rectangle;
    assert!(sent.2 > 80 && sent.3 > 80, "{sent:?}: the magnified icon");
    // The pointer leaves: back at rest.
    let _ = dock.update(Message::Pointer(Pointer::Left));
    let places = recorder.places();
    assert_eq!(places.len(), before + 2);
    assert_eq!(places[before + 1][2].rectangle.2, 52);
    let count = places.len();

    // A new open application: the icons move, and are placed again.
    let Placeholder(mut more) = Placeholder::new("terminal,ExampleBrowser", None);
    more.controllable = true;
    let _ = dock.update(Message::Windows(more));
    let places = recorder.places();
    assert_eq!(places.len(), count + 1);
    assert_eq!(places[count].len(), 4);
}

#[test]
fn a_dock_brought_back_keeps_the_pointer_up_to_the_edge() {
    for position in EDGES {
        let mut dock = dock(position, AutoHide::Smart, &Recorder::default());
        let _ = dock.update(windows(&[SCREEN]));
        settle(&mut dock);
        let size = dock.surface_size;
        // A point in the gap of 8 between the bar and the screen edge, in the middle.
        let gap = match position {
            Position::Bottom => Point::new(size.width / 2.0, size.height - 3.0),
            Position::Top => Point::new(size.width / 2.0, 3.0),
            Position::Left => Point::new(3.0, size.height / 2.0),
            Position::Right => Point::new(size.width - 3.0, size.height / 2.0),
        };
        // The pointer touches the band, the dock comes back.
        let band = dock.input_area().center();
        let _ = dock.update(Message::Pointer(Pointer::Moved(band)));
        settle(&mut dock);
        // Shown, the gap is part of the surface and of its input region.
        assert!(dock.input_area().contains(gap), "{position:?}");
        // The pointer rests in the gap: the dock stays, long after the delay.
        let _ = dock.update(Message::Pointer(Pointer::Moved(gap)));
        let now = dock.now();
        let _ = dock.tick(now + Duration::from_secs(2));
        assert_eq!(margin(&dock, position), 0, "{position:?} still shown");
        // A dock that does not hide keeps the gap out of its input region.
        let shown = self::dock(position, AutoHide::Never, &Recorder::default());
        assert!(!shown.input_area().contains(gap), "{position:?}");
    }
}

#[test]
fn an_open_menu_holds_the_dock_without_frames() {
    let mut dock = dock(Position::Bottom, AutoHide::Smart, &Recorder::default());
    let _ = dock.update(windows(&[SCREEN]));
    settle(&mut dock);
    let _ = dock.update(Message::Pointer(Pointer::Moved(Point::new(300.0, 1.0))));
    settle(&mut dock);
    // A right click on the terminal opens its menu; the pointer goes there.
    let icon = dock.layout(dock.now()).icons[2].center();
    let _ = dock.update(Message::Pointer(Pointer::Moved(icon)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(
        iced::mouse::Button::Right,
    )));
    assert!(dock.menu_open());
    let _ = dock.update(Message::Pointer(Pointer::Left));
    settle(&mut dock);
    assert!(!dock.animating(), "no frame while the menu is open");
    assert_eq!(margin(&dock, Position::Bottom), 0, "the dock stays");
}
