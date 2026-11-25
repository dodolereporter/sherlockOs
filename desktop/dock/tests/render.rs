//! Rendering the dock and an icon's menu without a screen, from the neutral fixtures.

use std::path::PathBuf;
use std::time::Duration;

use dock::app::{Dock, Message, Pointer};
use dock::config::Config;
use dock::entry::Catalog;
use dock::icons::IconLookup;
use dock::windows::Placeholder;
use iced::widget::container;
use iced::window::Id;
use iced::{Element, Event, Length, Point, mouse};
use iced_test::simulator::Simulator;
use sherlock_ui::theme;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Four applications of the fixtures, Sherlock in the middle; the terminal open and active,
/// the browser open.
fn sample() -> Dock {
    let data = fixtures().join("data");
    let catalog = Catalog::load(std::slice::from_ref(&data));
    let lookup = IconLookup::new(&[data], None, Some("Exemple"));
    let pinned: Vec<String> = [
        "org.example.Files",
        "org.example.Terminal",
        "example-editor",
        "example-browser",
    ]
    .map(String::from)
    .to_vec();
    let items = dock::app::items(&catalog, &pinned, &lookup);
    let mut dock = Dock::new(items, pinned, Config::default(), None);
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
    let dock = sample();
    assert_eq!(dock.items.len(), 5);
    assert_eq!(dock.items[2].name, "Sherlock", "Sherlock in the middle");
    let ui = Simulator::with_size(dock::settings(), size(&dock), on_desktop(dock.view_dock()));
    let bytes = capture(ui, "dock-rest").expect("screenshot written");
    assert!(bytes > 2_000, "empty screenshot");
}

#[test]
fn the_dock_renders_magnified_with_its_label() {
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
    let mut dock = sample();
    let _ = dock.update(Message::Surface(Id::unique()));
    let point = icon_center(&dock, 1);
    let _ = dock.update(Message::Pointer(Pointer::Moved(point)));
    let _ = dock.update(Message::Pointer(Pointer::Pressed(mouse::Button::Right)));
    assert!(dock.menu_open());
    let entries = dock::menu::entries(menu_abilities());
    let (width, height) = dock::menu::surface_size(&entries);
    let mut ui = Simulator::with_size(
        dock::settings(),
        (width as f32, height as f32),
        dock.view_menu(),
    );
    for text in [
        "Terminal d'exemple",
        "1 fenêtre",
        "Ouvrir",
        "Nouvelle fenêtre",
        "Retirer du dock",
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

/// What the terminal fixture can do: start, new window, unpin; not quit.
fn menu_abilities() -> dock::menu::Abilities {
    dock::menu::Abilities {
        launch: true,
        new_window: true,
        unpin: true,
        quit: false,
    }
}

#[test]
fn unpinning_keeps_sherlock_in_the_middle() {
    let mut dock = sample();
    let _ = dock.unpin(0);
    let names: Vec<&str> = dock.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Terminal d'exemple",
            "Éditeur",
            "Sherlock",
            "Example Browser"
        ]
    );
    assert_eq!(
        dock.pinned,
        ["org.example.Terminal", "example-editor", "example-browser"]
    );
    // Sherlock cannot be unpinned.
    let _ = dock.unpin(2);
    assert_eq!(dock.items.len(), 4);
}
