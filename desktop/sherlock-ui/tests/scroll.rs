//! Headless tests of the scroll area (UI-11): what the wheel, the knob and the rail do, and
//! captures of the overlay scroller at rest, while scrolling and hovered, in each theme, at a
//! laptop size and at 1920 × 1080.

use std::borrow::Cow;
use std::error::Error;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::advanced::mouse;
use iced::event::Status;
use iced::widget::{Column, container, row, text};
use iced::{Element, Event, Length, Pixels, Point, Rectangle, Settings, Size};
use iced_test::simulator::{self, Simulator};
use sherlock_ui::scroll::{self, scroll};
use sherlock_ui::{color, theme, typography};

#[derive(Debug, Clone, PartialEq)]
enum Msg {
    /// The offset and the height shown, as `on_scroll` gives them.
    Scrolled(f32, f32),
}

fn settings() -> Settings {
    Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    }
}

/// Height of a row of the lists below.
const ROW: f32 = 40.0;

fn rows(count: usize, label: &'static str) -> Column<'static, Msg> {
    Column::with_children((1..=count).map(|i| {
        container(text(format!("{label} {i}")).size(typography::UI_SIZE))
            .padding([10.0, 16.0])
            .height(ROW)
            .width(Length::Fill)
            .into()
    }))
}

/// One list of `count` rows filling the window.
fn list(count: usize) -> Element<'static, Msg> {
    scroll(rows(count, "Élément"))
        .on_scroll(|v| Msg::Scrolled(v.absolute_offset().y, v.bounds().height))
        .height(Length::Fill)
        .into()
}

fn ui(size: (f32, f32), view: Element<'static, Msg>) -> Simulator<'static, Msg> {
    Simulator::with_size(settings(), size, view)
}

fn wheel(lines: f32) -> Event {
    Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines { x: 0.0, y: lines },
    })
}

fn moved(at: Point) -> Event {
    Event::Mouse(mouse::Event::CursorMoved { position: at })
}

fn frame() -> Event {
    Event::Window(iced::window::Event::RedrawRequested(Instant::now()))
}

fn last_offset(ui: Simulator<'static, Msg>) -> Option<f32> {
    ui.into_messages()
        .map(|Msg::Scrolled(offset, _)| offset)
        .last()
}

const SIZE: (f32, f32) = (400.0, 400.0);

#[test]
fn content_that_fits_neither_scrolls_nor_stretches() {
    let mut ui = ui(SIZE, list(5));
    let at = Point::new(200.0, 100.0);
    ui.point_at(at);
    let statuses = ui.simulate([moved(at), wheel(3.0), wheel(-3.0)]);
    assert!(
        statuses[1..].iter().all(|s| *s == Status::Ignored),
        "the wheel goes on to whatever holds the area: {statuses:?}"
    );
}

#[test]
fn the_wheel_scrolls_and_stretches_at_the_limit() {
    let mut ui = ui(SIZE, list(40));
    let at = Point::new(200.0, 100.0);
    ui.point_at(at);
    // Up at the top: nothing to scroll, the area stretches and keeps the wheel.
    let statuses = ui.simulate([moved(at), wheel(2.0)]);
    assert_eq!(statuses[1], Status::Captured);
    // Down: the stretch first, then the content.
    let _ = ui.simulate([wheel(-2.0), wheel(-2.0), frame()]);
    let offset = last_offset(ui).expect("the area scrolled");
    assert!((offset - 2.0 * scroll::WHEEL_LINE).abs() < 0.5, "{offset}");
}

#[test]
fn a_click_in_the_rail_jumps_by_a_page() {
    let mut ui = ui(SIZE, list(40));
    let below_knob = Point::new(SIZE.0 - scroll::ZONE / 2.0, SIZE.1 - 20.0);
    ui.point_at(below_knob);
    let _ = ui.simulate([moved(below_knob)]);
    let _ = ui.simulate(simulator::click());
    let _ = ui.simulate([frame()]);
    let offset = last_offset(ui).expect("the area scrolled");
    assert!((offset - scroll::page(SIZE.1)).abs() < 0.5, "{offset}");
}

#[test]
fn the_knob_drags_the_content() {
    let mut ui = ui(SIZE, list(40));
    let area = Rectangle::new(Point::ORIGIN, Size::new(SIZE.0, SIZE.1));
    let content = 40.0 * ROW;
    let knob = scroll::knob(area, content, 0.0, 0.0, scroll::WIDE).expect("a knob");
    let grab = Point::new(knob.center_x(), knob.y + 5.0);
    let to = Point::new(grab.x - 60.0, grab.y + 100.0);
    ui.point_at(grab);
    let _ = ui.simulate([moved(grab)]);
    let press = ui.simulate(simulator::click().take(1));
    assert_eq!(press[0], Status::Captured, "the knob takes the press");
    // Off the zone, the held knob still follows the pointer.
    ui.point_at(to);
    let _ = ui.simulate([moved(to)]);
    let _ = ui.simulate(simulator::click().skip(1));
    let _ = ui.simulate([frame()]);
    let expected = scroll::offset_at(area, content, knob.y + 100.0);
    let offset = last_offset(ui).expect("the area scrolled");
    assert!((offset - expected).abs() < 0.5, "{offset} instead of {expected}");
}

/// A window like those of the desktop: a sidebar and a page, each its own scroll area.
fn window() -> Element<'static, Msg> {
    let sidebar = container(scroll(rows(40, "Section")).height(Length::Fill))
        .width(240.0)
        .height(Length::Fill)
        .style(theme::sidebar);
    let page = container(list(120))
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::window);
    container(row![sidebar, page])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Writes a snapshot of `ui` into `target/tmp/scroll/<name>-tiny-skia.png` and returns that
/// path.
fn capture(ui: &mut Simulator<'static, Msg>, name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let snapshot = ui.snapshot(&theme::window_theme())?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scroll");
    std::fs::create_dir_all(&dir)?;
    let written = dir.join(format!("{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    if !snapshot.matches_image(dir.join(format!("{name}.png")))? {
        return Err(format!("{name}: not written").into());
    }
    Ok(written)
}

/// Captures into `target/tmp/scroll/<theme>-<size>-<state>-tiny-skia.png`, at scale 2: at
/// rest (no scroller), while scrolling (thin knob), and hovered (wide knob on its rail). The
/// waits let the fades end; they stay well inside the linger.
#[test]
fn the_scroller_renders_at_rest_while_scrolling_and_hovered() {
    for (which, name) in [(color::Theme::Dark, "dark"), (color::Theme::Light, "light")] {
        color::set_active(which);
        // The small one, a list alone, is the one the documentation keeps.
        for size in [(360.0, 280.0), (1280.0, 800.0), (1920.0, 1080.0)] {
            let tag = format!("{name}-{}x{}", size.0, size.1);
            let view = if size.0 < 400.0 {
                container(list(40)).style(theme::window).into()
            } else {
                window()
            };
            let mut ui = ui(size, view);
            let written = capture(&mut ui, &format!("{tag}-rest")).expect("capture");
            println!("snapshot: {}", written.display());

            let middle = Point::new(size.0 / 2.0, size.1 / 2.0);
            ui.point_at(middle);
            let _ = ui.simulate([moved(middle), wheel(-6.0)]);
            std::thread::sleep(scroll::SHOW + Duration::from_millis(50));
            let written = capture(&mut ui, &format!("{tag}-scrolling")).expect("capture");
            println!("snapshot: {}", written.display());

            let edge = Point::new(size.0 - scroll::ZONE / 2.0, size.1 * 0.8);
            ui.point_at(edge);
            let _ = ui.simulate([moved(edge)]);
            std::thread::sleep(scroll::EXPAND + Duration::from_millis(50));
            let written = capture(&mut ui, &format!("{tag}-hovered")).expect("capture");
            println!("snapshot: {}", written.display());
        }
    }
    color::set_active(color::Theme::Dark);
}
