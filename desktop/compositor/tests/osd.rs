//! Boards of the on-screen display (lot D-35d), to judge its motion and its place by eye: drawn
//! on the processor as the compositor draws it, without a screen, into Cargo's temporary folder
//! of the integration tests (`target/tmp`):
//!
//! - `osd-stretch.png`: « plus » pressed once more at 100 %, in 6 steps; the guides mark the
//!   ends of the pill at rest.
//! - `osd-gauge.png`: the gauge sliding from 40 to 60 %, in 6 steps.
//! - `osd-enter-leave.png`: the pill coming, then going, in 6 steps.
//! - `osd-screen-<width>x<height>@<scale>.png`: the pill on four screens, above a dock of the
//!   default size.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use compositor::osd::{Edge, Frame, Level, Osd, draw};
    use sherlock_ui::{
        color::{Color, Theme},
        dock, metrics,
        osd::{self as ui, Kind},
    };
    use smithay::utils::Rectangle;
    use tiny_skia::{Paint, Pixmap, PixmapPaint, Rect, Transform};

    /// Pixels per logical pixel of the step boards.
    const BOARD_SCALE: f64 = 2.0;
    /// One step of a board, in logical pixels: the pill on a strip of desktop.
    const CELL: (f64, f64) = (340.0, 110.0);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn volume(percent: u16) -> Level {
        Level {
            kind: Kind::Volume,
            percent,
            muted: false,
        }
    }

    fn paint(color: Color) -> Paint<'static> {
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, (color.a * 255.0).round() as u8);
        paint.anti_alias = true;
        paint
    }

    fn fill(pixmap: &mut Pixmap, (x, y, w, h): (f64, f64, f64, f64), color: Color) {
        if let Some(rect) = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
            pixmap.fill_rect(rect, &paint(color), Transform::identity(), None);
        }
    }

    /// Draws `frame` into `pixmap` at `scale`, moved by `offset` logical pixels, as the
    /// renderer does: the texture at its look's size, stretched over the frame's rectangle.
    fn put(pixmap: &mut Pixmap, frame: &Frame, scale: f64, offset: (f64, f64)) {
        let look = frame.look(scale);
        let pill = draw(&look).unwrap();
        let rect = frame.rect;
        let (x, y) = (
            (rect.loc.x + offset.0) * scale,
            (rect.loc.y + offset.1) * scale,
        );
        let sx = rect.size.w * scale / f64::from(pill.width());
        let sy = rect.size.h * scale / f64::from(pill.height());
        let transform = Transform::from_row(sx as f32, 0.0, 0.0, sy as f32, x as f32, y as f32);
        let style = PixmapPaint {
            opacity: frame.alpha,
            quality: tiny_skia::FilterQuality::Bicubic,
            ..PixmapPaint::default()
        };
        pixmap.draw_pixmap(0, 0, pill.as_ref(), &style, transform, None);
    }

    fn save(pixmap: &Pixmap, name: &str) -> PathBuf {
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let image =
            image::RgbaImage::from_raw(pixmap.width(), pixmap.height(), pixmap.data().to_vec())
                .unwrap();
        image.save(&path).unwrap();
        path
    }

    /// A board of `frames` side by side, each on its strip of desktop, with guides at `guides`
    /// (logical x, inside a cell).
    fn board(name: &str, frames: &[Frame], guides: &[f64]) -> PathBuf {
        let palette = Theme::Dark.palette();
        let (w, h) = (CELL.0 * frames.len() as f64, CELL.1);
        let mut pixmap = Pixmap::new((w * BOARD_SCALE) as u32, (h * BOARD_SCALE) as u32).unwrap();
        let s = BOARD_SCALE;
        fill(&mut pixmap, (0.0, 0.0, w * s, h * s), palette.desktop);
        for (i, frame) in frames.iter().enumerate() {
            let left = CELL.0 * i as f64;
            if i > 0 {
                fill(
                    &mut pixmap,
                    (left * s, 0.0, s, h * s),
                    palette.text_tertiary,
                );
            }
            for x in guides {
                let x = (left + x) * s;
                fill(
                    &mut pixmap,
                    (x, 8.0 * s, 1.0, (h - 16.0) * s),
                    palette.blue_light,
                );
            }
            put(&mut pixmap, frame, s, (left, 0.0));
        }
        save(&pixmap, name)
    }

    /// The frames of `osd` at `times`, placed on a screen of 1920 by 1080, then moved into the
    /// middle of a cell.
    fn frames(osd: &Osd, times: &[u64]) -> Vec<Frame> {
        let screen = Rectangle::new((0.0, 0.0).into(), (1920.0, 1080.0).into());
        let middle = 1080.0 - f64::from(ui::BOTTOM) - f64::from(ui::HEIGHT) / 2.0;
        let shift = ((1920.0 - CELL.0) / 2.0, middle - CELL.1 / 2.0);
        times
            .iter()
            .map(|t| {
                let mut frame = osd.frame(ms(*t), screen, 0.0).unwrap();
                frame.rect.loc.x -= shift.0;
                frame.rect.loc.y -= shift.1;
                frame
            })
            .collect()
    }

    #[test]
    fn the_stretch_in_six_steps() {
        let mut osd = Osd::default();
        osd.show(volume(100), None, None, ms(0));
        osd.show(volume(100), Some(Edge::High), None, ms(1000));
        let times = [1000, 1040, 1080, 1120, 1200, 1360];
        let frames = frames(&osd, &times);
        let rest = &frames[0];
        let ends = [rest.rect.loc.x, rest.rect.loc.x + rest.rect.size.w];
        let path = board("osd-stretch.png", &frames, &ends);
        // The fourth step, at the release, is the widest; the last is back at rest.
        let widths: Vec<f64> = frames.iter().map(|f| f.rect.size.w).collect();
        let widest = widths.iter().copied().fold(0.0, f64::max);
        assert!(widest / widths[0] > 1.04, "{widths:?}");
        assert!((widths[5] - widths[0]).abs() < 1.0, "{widths:?}");
        eprintln!("{}", path.display());
    }

    #[test]
    fn the_gauge_in_six_steps() {
        let mut osd = Osd::default();
        osd.show(volume(40), None, None, ms(0));
        osd.show(volume(60), None, None, ms(1000));
        let times = [1000, 1030, 1060, 1100, 1160, 1300];
        let frames = frames(&osd, &times);
        let gauges: Vec<f64> = frames.iter().map(|f| f.gauge).collect();
        assert!(gauges.windows(2).all(|g| g[1] >= g[0] - 0.01), "{gauges:?}");
        let path = board("osd-gauge.png", &frames, &[]);
        eprintln!("{}", path.display());
    }

    #[test]
    fn the_coming_and_going_in_six_steps() {
        let mut osd = Osd::default();
        osd.show(volume(50), None, None, ms(0));
        osd.hide(ms(1000));
        let times = [0, 60, 250, 1000, 1080, 1150];
        let frames = frames(&osd, &times);
        let path = board("osd-enter-leave.png", &frames, &[]);
        eprintln!("{}", path.display());
    }

    #[test]
    fn the_pill_on_four_screens() {
        let palette = Theme::Dark.palette();
        let screens = [
            (1366.0, 768.0, 1.0),
            (1920.0, 1080.0, 1.0),
            (2560.0, 1600.0, 1.5),
            (3840.0, 2160.0, 2.0),
        ];
        let mut osd = Osd::default();
        osd.show(volume(60), None, None, ms(0));
        for (width, height, scale) in screens {
            let (w, h) = (width / scale, height / scale);
            let screen = Rectangle::new((0.0, 0.0).into(), (w, h).into());
            let frame = osd.frame(ui::ENTER, screen, 0.0).unwrap();
            let mut pixmap = Pixmap::new(width as u32, height as u32).unwrap();
            fill(&mut pixmap, (0.0, 0.0, width, height), palette.desktop);
            // A dock of the default size, to see the gap.
            let dock_w = 9.0 * f64::from(dock::ICON_SIZE_MIN + 24.0);
            let dock_h = f64::from(metrics::DOCK_HEIGHT);
            let bottom = f64::from(dock::MARGIN_BOTTOM);
            let rect = ((w - dock_w) / 2.0, h - bottom - dock_h, dock_w, dock_h);
            let rect = (
                rect.0 * scale,
                rect.1 * scale,
                rect.2 * scale,
                rect.3 * scale,
            );
            fill(&mut pixmap, rect, palette.dock_glass);
            put(&mut pixmap, &frame, scale, (0.0, 0.0));
            // Centered, above the dock, inside its bounds.
            let r = frame.rect;
            assert!((r.loc.x + r.size.w / 2.0 - w / 2.0).abs() < 1e-6);
            assert!(r.loc.y + r.size.h < h - bottom - dock_h - 20.0);
            let pill = r.size.w;
            let bounds = f64::from(ui::WIDTH_MIN) - 1e-3..=f64::from(ui::WIDTH_MAX) + 1e-3;
            assert!(bounds.contains(&pill), "{pill} on {width}x{height}");
            let name = format!("osd-screen-{width}x{height}@{scale}.png");
            eprintln!("{}", save(&pixmap, &name).display());
        }
    }
}
