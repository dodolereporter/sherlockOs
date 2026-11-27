//! The picture under the pointer when files are dragged out of Evidence (lot E-03): the cards
//! of the first files stacked and fanned, the front one showing its icon or, for a photo, its
//! thumbnail, and the count of files in a pill. Drawn as an SVG document by resvg, the
//! thumbnail laid over it, at `drag::SCALE` pixels per logical pixel.

use std::{fmt::Write as _, path::Path};

use sherlock_ui::{color::Color, drag, icon::Icon as Glyph, metrics};

use crate::dnd::Icon;

/// One file of the drag: its icon, and its path if it may be a photo.
#[derive(Debug, Clone)]
pub struct Item {
    pub glyph: Glyph,
    pub tint: Color,
    pub photo: Option<std::path::PathBuf>,
}

fn css(color: Color) -> String {
    format!(
        "fill=\"rgb({},{},{})\" fill-opacity=\"{}\"",
        color.r, color.g, color.b, color.a
    )
}

fn stroke(color: Color) -> String {
    format!(
        "stroke=\"rgb({},{},{})\" stroke-opacity=\"{}\"",
        color.r, color.g, color.b, color.a
    )
}

/// The segments of a seven-segment digit, `a` to `g`, as lines in a box 6 wide and 10 high.
const SEGMENTS: [((f32, f32), (f32, f32)); 7] = [
    ((0.0, 0.0), (6.0, 0.0)),
    ((6.0, 0.0), (6.0, 5.0)),
    ((6.0, 5.0), (6.0, 10.0)),
    ((0.0, 10.0), (6.0, 10.0)),
    ((0.0, 5.0), (0.0, 10.0)),
    ((0.0, 0.0), (0.0, 5.0)),
    ((0.0, 5.0), (6.0, 5.0)),
];

/// Which segments light up for each digit.
const DIGITS: [&str; 10] = [
    "abcdef", "bc", "abged", "abgcd", "fgbc", "afgcd", "afgedc", "abc", "abcdefg", "abcdfg",
];

/// The path of `text` (digits and `+`) drawn in segments, each digit `height` high, from `x`,
/// `y`, and its width.
fn digits_path(text: &str, (x, y): (f32, f32), height: f32) -> (String, f32) {
    let unit = height / 10.0;
    let (advance, mut d, mut at) = (9.0 * unit, String::new(), x);
    for c in text.chars() {
        let segments = match c {
            '+' => {
                let _ = write!(
                    d,
                    "M{} {}h{}M{} {}v{}",
                    at,
                    y + 5.0 * unit,
                    6.0 * unit,
                    at + 3.0 * unit,
                    y + 2.0 * unit,
                    6.0 * unit
                );
                at += advance;
                continue;
            }
            c => c.to_digit(10).map_or("", |n| DIGITS[n as usize]),
        };
        for s in segments.bytes() {
            let ((x0, y0), (x1, y1)) = SEGMENTS[usize::from(s - b'a')];
            let _ = write!(
                d,
                "M{} {}L{} {}",
                at + x0 * unit,
                y + y0 * unit,
                at + x1 * unit,
                y + y1 * unit
            );
        }
        at += advance;
    }
    (d, at - x - 3.0 * unit)
}

/// Logical side of the picture.
fn side() -> f32 {
    drag::CARD + drag::FAN * (drag::STACK as f32 - 1.0) + 2.0 * drag::MARGIN
}

/// Top left corner of the card `depth` behind the front one.
fn card_at(depth: usize) -> (f32, f32) {
    let shift = drag::FAN * depth as f32;
    let behind = drag::FAN * (drag::STACK as f32 - 1.0);
    (drag::MARGIN + shift, drag::MARGIN + behind - shift)
}

/// The SVG of the stack, without the thumbnail; `badge`: the pill alone, drawn after it.
fn document(items: &[Item], count: usize, badge: bool, photo: bool) -> String {
    let side = side();
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{side}\" height=\"{side}\" \
         viewBox=\"0 0 {side} {side}\">"
    );
    if !badge {
        let shown = items.len().min(drag::STACK);
        for depth in (0..shown).rev() {
            let (x, y) = card_at(depth);
            let (c, r) = (drag::CARD, drag::CARD_RADIUS);
            let turn = drag::TURN * depth as f32;
            let _ = write!(
                svg,
                "<g transform=\"rotate({turn} {cx} {cy})\"><rect x=\"{x}\" y=\"{y}\" \
                 width=\"{c}\" height=\"{c}\" rx=\"{r}\" {} {} stroke-width=\"{}\"/>",
                css(drag::card()),
                stroke(drag::card_border()),
                drag::CARD_BORDER,
                cx = x + c / 2.0,
                cy = y + c / 2.0,
            );
            let item = &items[depth];
            if !(depth == 0 && photo) {
                let scale = drag::GLYPH / metrics::ICON_GRID;
                let (gx, gy) = (x + (c - drag::GLYPH) / 2.0, y + (c - drag::GLYPH) / 2.0);
                let _ = write!(
                    svg,
                    "<path transform=\"translate({gx} {gy}) scale({scale})\" d=\"{}\" \
                     fill=\"none\" {} stroke-width=\"{}\" stroke-linecap=\"round\" \
                     stroke-linejoin=\"round\"/>",
                    item.glyph.path(),
                    stroke(item.tint),
                    metrics::ICON_STROKE,
                );
            }
            svg.push_str("</g>");
        }
    } else if count > 1 {
        let text = if count > 99 {
            String::from("99+")
        } else {
            count.to_string()
        };
        let (h, digit) = (drag::BADGE, drag::DIGIT);
        let (_, width) = digits_path(&text, (0.0, 0.0), digit);
        let w = (width + h - digit).max(h);
        let (front_x, front_y) = card_at(0);
        let (x, y) = (
            front_x + drag::CARD - w / 2.0 - 2.0,
            front_y - h / 2.0 + 2.0,
        );
        let (path, _) = digits_path(&text, (x + (w - width) / 2.0, y + (h - digit) / 2.0), digit);
        let _ = write!(
            svg,
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"{}\" {}/>\
             <path d=\"{path}\" fill=\"none\" {} stroke-width=\"{}\" stroke-linecap=\"round\" \
             stroke-linejoin=\"round\"/>",
            h / 2.0,
            css(drag::badge()),
            stroke(drag::badge_text()),
            digit / 6.0,
        );
    }
    svg.push_str("</svg>");
    svg
}

/// The thumbnail of a photo, `side` pixels square, cropped to fill, in straight RGBA.
fn thumbnail(path: &Path, side: u32) -> Option<Vec<u8>> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > drag::THUMBNAIL_MAX_FILE {
        return None;
    }
    let picture = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let filled = picture.resize_to_fill(side, side, image::imageops::FilterType::Triangle);
    Some(filled.to_rgba8().into_raw())
}

/// Lays `top` (straight RGBA, `w` wide) over `base` at `(x, y)`, through a rounded mask of
/// `radius` pixels.
fn lay(base: &mut [u8], base_w: usize, top: &[u8], w: usize, (x, y): (usize, usize), radius: f32) {
    let h = top.len() / 4 / w.max(1);
    for row in 0..h {
        for col in 0..w {
            // Coverage of the rounded square at this pixel's center.
            let (cx, cy) = (col as f32 + 0.5, row as f32 + 0.5);
            let dx = (radius - cx).max(cx - (w as f32 - radius)).max(0.0);
            let dy = (radius - cy).max(cy - (h as f32 - radius)).max(0.0);
            let cover = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
            let t = &top[(row * w + col) * 4..][..4];
            let i = ((y + row) * base_w + x + col) * 4;
            let Some(b) = base.get_mut(i..i + 4) else {
                continue;
            };
            let ta = f32::from(t[3]) / 255.0 * cover;
            let ba = f32::from(b[3]) / 255.0;
            let out = ta + ba * (1.0 - ta);
            if out <= 0.0 {
                continue;
            }
            for k in 0..3 {
                let c = (f32::from(t[k]) * ta + f32::from(b[k]) * ba * (1.0 - ta)) / out;
                b[k] = c.round().clamp(0.0, 255.0) as u8;
            }
            b[3] = (out * 255.0).round() as u8;
        }
    }
}

/// The picture of a drag of `count` files, whose first ones are `items`.
pub fn build(items: &[Item], count: usize) -> Option<Icon> {
    let scale = drag::SCALE as f32;
    let px = (side() * scale).round() as u32;
    let front_photo = items
        .first()
        .and_then(|i| i.photo.as_deref())
        .and_then(|p| thumbnail(p, (drag::CARD * scale) as u32));
    let base = document(items, count, false, front_photo.is_some());
    let mut rgba = sherlock_ui::raster::rasterize(base.as_bytes(), px)?;
    if let Some(photo) = front_photo {
        let (x, y) = card_at(0);
        let at = ((x * scale) as usize, (y * scale) as usize);
        let w = (drag::CARD * scale) as usize;
        lay(
            &mut rgba,
            px as usize,
            &photo,
            w,
            at,
            drag::CARD_RADIUS * scale,
        );
    }
    if count > 1 {
        let pill = document(items, count, true, false);
        let pill = sherlock_ui::raster::rasterize(pill.as_bytes(), px)?;
        lay(&mut rgba, px as usize, &pill, px as usize, (0, 0), 0.0);
    }
    Some(Icon {
        rgba,
        width: px,
        height: px,
        scale: drag::SCALE,
        hotspot: (drag::HOTSPOT.0 as i32, drag::HOTSPOT.1 as i32),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(glyph: Glyph) -> Item {
        Item {
            glyph,
            tint: sherlock_ui::color::palette().text_secondary,
            photo: None,
        }
    }

    #[test]
    fn the_picture_stacks_the_files_and_counts_them() {
        let one = build(&[item(Glyph::Folder)], 1).unwrap();
        let side = (side() * drag::SCALE as f32).round() as u32;
        assert_eq!((one.width, one.height), (side, side));
        assert_eq!(one.rgba.len(), (side * side * 4) as usize);
        let items = [
            item(Glyph::Folder),
            item(Glyph::Document),
            item(Glyph::Document),
        ];
        let many = build(&items, 12).unwrap();
        // The pill adds opaque pixels at the top right that a single file does not have.
        let opaque = |icon: &Icon| icon.rgba.chunks(4).filter(|p| p[3] > 200).count();
        assert!(opaque(&many) > opaque(&one));
    }

    #[test]
    fn digits_are_drawn_in_segments() {
        let (path, width) = digits_path("18", (0.0, 0.0), 10.0);
        // 1: two segments, 8: seven.
        assert_eq!(path.matches('M').count(), 9);
        assert!((width - 15.0).abs() < 0.01, "{width}");
    }

    #[test]
    fn a_photo_shows_as_its_thumbnail() {
        let dir = std::env::temp_dir().join(format!("evidence-drag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("red.png");
        image::RgbaImage::from_pixel(300, 200, image::Rgba([255, 0, 0, 255]))
            .save(&path)
            .unwrap();
        let photo = Item {
            photo: Some(path),
            ..item(Glyph::Document)
        };
        let icon = build(&[photo], 1).unwrap();
        let red = icon
            .rgba
            .chunks(4)
            .filter(|p| p[0] > 240 && p[1] < 20 && p[3] == 255)
            .count();
        let card = (drag::CARD * drag::SCALE as f32) as usize;
        assert!(red > card * card * 3 / 4, "{red}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
