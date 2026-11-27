//! The still picture, drawn by tiny-skia (software, or no model): it stays inside the widget,
//! a disc for the round cut, a covered rectangle for none (lot Q-33). Before, tiny-skia drew the
//! whole picture, larger than the widget: only a container with `clip` hid it.

use std::path::PathBuf;

use iced::widget::container;
use iced::{Background, Color, Element, Settings, Theme};
use iced_test::simulator::Simulator;
use sherlock_3d::{Cut, Sherlock3d};

/// Size of the page around the mascot, logical pixels.
const PAGE: f32 = 300.0;
/// The background of the page, which nothing of the mascot may cover outside its frame.
const BACKGROUND: Color = Color::from_rgb(0.0, 1.0, 0.0);

/// The page drawn at scale 2 (the simulator's), as RGBA pixels, and its side in pixels.
fn render(mascot: Sherlock3d, name: &str) -> (image::RgbaImage, u32) {
    let page: Element<'_, ()> = container(mascot)
        .center(PAGE)
        .style(|_| container::Style {
            background: Some(Background::Color(BACKGROUND)),
            ..container::Style::default()
        })
        .into();
    let mut ui = Simulator::with_size(Settings::default(), (PAGE, PAGE), page);
    let Ok(snapshot) = ui.snapshot(&Theme::Dark) else {
        panic!("snapshot");
    };
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("still-{name}.png"));
    let written = path.with_file_name(format!("still-{name}-tiny-skia.png"));
    let _ = std::fs::remove_file(&written);
    assert!(matches!(snapshot.matches_image(&path), Ok(true)));
    let Ok(picture) = image::open(&written) else {
        panic!("{} not written", written.display());
    };
    let picture = picture.to_rgba8();
    let side = picture.width();
    (picture, side)
}

fn is_background(pixel: &image::Rgba<u8>) -> bool {
    pixel.0 == [0, 255, 0, 255]
}

/// Every pixel outside `frame` (x, y, width, height in pixels) is the page's.
fn nothing_outside(picture: &image::RgbaImage, frame: (u32, u32, u32, u32)) {
    let (x0, y0, w, h) = frame;
    for (x, y, pixel) in picture.enumerate_pixels() {
        let inside = (x0..x0 + w).contains(&x) && (y0..y0 + h).contains(&y);
        assert!(
            inside || is_background(pixel),
            "({x}, {y}) covered: {pixel:?}"
        );
    }
}

#[test]
fn the_round_still_picture_is_a_disc_inside_its_frame() {
    // Wider than tall: the disc stays whole, centered, as the 3D cut does.
    let mascot = Sherlock3d::new(None).width(160).height(100);
    let (picture, side) = render(mascot, "round");
    assert_eq!(side, 600);
    // At scale 2: the frame is 320 × 200 in the middle of 600 × 600.
    nothing_outside(&picture, (140, 200, 320, 200));
    // The disc is 200 across, centered: the corners of its square are the page's.
    for (x, y) in [(201, 201), (398, 201), (201, 398), (398, 398), (145, 300)] {
        assert!(is_background(picture.get_pixel(x, y)), "({x}, {y})");
    }
    assert!(
        !is_background(picture.get_pixel(300, 300)),
        "the picture shows"
    );
}

#[test]
fn the_rectangular_still_picture_covers_its_frame_and_no_more() {
    let mascot = Sherlock3d::new(None).cut(Cut::None).width(100).height(140);
    let (picture, _) = render(mascot, "rectangle");
    // Covered, the top square of the picture is wider than the frame and the rest of the
    // picture (the body) used to hang below it.
    nothing_outside(&picture, (200, 160, 200, 280));
    assert!(
        !is_background(picture.get_pixel(300, 300)),
        "the picture shows"
    );
}
