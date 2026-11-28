//! Pictures the compositor draws on the processor from an image of the screen (lot D-35e): the
//! thumbnail of a screenshot just taken, and the loupe of the screenshot tool. Each is made
//! into a texture once, kept by its key while its picture stays the same.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

use sherlock_ui::color::Color;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                texture::{TextureBuffer, TextureRenderElement},
            },
            gles::{GlesRenderer, GlesTexture},
        },
    },
    utils::{Physical, Rectangle, Transform},
};
use tiny_skia::{
    FillRule, Mask, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform as Affine,
};

use crate::{capture::Image, damage::Key};

/// Premultiplied RGBA pixels, rows from the top, and what tells them apart.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub id: u64,
    pub size: (u32, u32),
    pub rgba: Arc<[u8]>,
}

impl Picture {
    fn of(pixmap: Pixmap, what: impl Hash) -> Self {
        let mut hasher = DefaultHasher::new();
        what.hash(&mut hasher);
        (pixmap.width(), pixmap.height()).hash(&mut hasher);
        Self {
            id: hasher.finish(),
            size: (pixmap.width(), pixmap.height()),
            rgba: Arc::from(pixmap.take()),
        }
    }
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, (color.a * 255.0).round() as u8);
    paint.anti_alias = true;
    paint
}

/// `image` as a pixmap, its straight alpha made premultiplied.
fn pixmap_of(image: &Image) -> Option<Pixmap> {
    let mut rgba = image.rgba.clone();
    for p in rgba.chunks_mut(4) {
        let a = u32::from(p[3]);
        for c in &mut p[..3] {
            *c = (u32::from(*c) * a / 255) as u8;
        }
    }
    Pixmap::from_vec(
        rgba,
        tiny_skia::IntSize::from_wh(image.width, image.height)?,
    )
}

/// The thumbnail of `image`, `size` pixels, its corners rounded by `radius` pixels and bordered
/// by `border` (color, width in pixels). `serial` tells two screenshots apart.
pub fn thumbnail(
    image: &Image,
    size: (u32, u32),
    radius: f32,
    border: (Color, f32),
    serial: u64,
) -> Option<Picture> {
    let source = pixmap_of(image)?;
    let mut pixmap = Pixmap::new(size.0.max(1), size.1.max(1))?;
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)?;
    let path = PathBuilder::from_rect(rect);
    let rounded = rounded(w, h, radius).unwrap_or(path);
    let mut mask = Mask::new(pixmap.width(), pixmap.height())?;
    mask.fill_path(&rounded, FillRule::Winding, true, Affine::identity());
    let sx = w / image.width.max(1) as f32;
    let sy = h / image.height.max(1) as f32;
    let quality = PixmapPaint {
        quality: tiny_skia::FilterQuality::Bilinear,
        ..PixmapPaint::default()
    };
    let scale = Affine::from_scale(sx, sy);
    pixmap.draw_pixmap(0, 0, source.as_ref(), &quality, scale, Some(&mask));
    let stroke = Stroke {
        width: border.1,
        ..Stroke::default()
    };
    pixmap.stroke_path(
        &rounded,
        &paint(border.0),
        &stroke,
        Affine::identity(),
        None,
    );
    Some(Picture::of(pixmap, ("thumbnail", serial)))
}

fn rounded(w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut path = PathBuilder::new();
    // A rounded rectangle with quadratic corners, close enough to circles at these sizes.
    path.move_to(r, 0.0);
    path.line_to(w - r, 0.0);
    path.quad_to(w, 0.0, w, r);
    path.line_to(w, h - r);
    path.quad_to(w, h, w - r, h);
    path.line_to(r, h);
    path.quad_to(0.0, h, 0.0, h - r);
    path.line_to(0.0, r);
    path.quad_to(0.0, 0.0, r, 0.0);
    path.close();
    path.finish()
}

/// The loupe: the `pixels` × `pixels` pixels of `image` around `(cx, cy)` (pixels of the
/// image), each a square of the same color, in a disc of `diameter` pixels with a ring of
/// `ring` (color, width) and the middle pixel outlined in `mark`.
pub fn loupe(
    image: &Image,
    (cx, cy): (i64, i64),
    (pixels, diameter): (u32, u32),
    ring: (Color, f32),
    mark: Color,
) -> Option<Picture> {
    let mut pixmap = Pixmap::new(diameter.max(1), diameter.max(1))?;
    let d = diameter as f32;
    let cell = d / pixels.max(1) as f32;
    let half = i64::from(pixels / 2);
    for row in 0..pixels {
        for col in 0..pixels {
            let (x, y) = (cx - half + i64::from(col), cy - half + i64::from(row));
            let inside =
                x >= 0 && y >= 0 && x < i64::from(image.width) && y < i64::from(image.height);
            let color = if inside {
                let i = ((y as usize) * image.width as usize + x as usize) * 4;
                let p = &image.rgba[i..i + 4];
                Color::rgba(p[0], p[1], p[2], 1.0)
            } else {
                Color::rgba(0, 0, 0, 1.0)
            };
            let r = tiny_skia::Rect::from_xywh(
                col as f32 * cell,
                row as f32 * cell,
                cell + 0.5,
                cell + 0.5,
            )?;
            let mut p = paint(color);
            p.anti_alias = false;
            pixmap.fill_rect(r, &p, Affine::identity(), None);
        }
    }
    let middle = tiny_skia::Rect::from_xywh(half as f32 * cell, half as f32 * cell, cell, cell)?;
    let stroke = Stroke {
        width: (cell / 6.0).max(1.0),
        ..Stroke::default()
    };
    pixmap.stroke_path(
        &PathBuilder::from_rect(middle),
        &paint(mark),
        &stroke,
        Affine::identity(),
        None,
    );
    // Cut to the disc, then its ring.
    let circle = PathBuilder::from_circle(d / 2.0, d / 2.0, d / 2.0 - ring.1 / 2.0)?;
    let mut mask = Mask::new(pixmap.width(), pixmap.height())?;
    mask.fill_path(&circle, FillRule::Winding, true, Affine::identity());
    let mut cut = Pixmap::new(pixmap.width(), pixmap.height())?;
    cut.draw_pixmap(
        0,
        0,
        pixmap.as_ref(),
        &PixmapPaint::default(),
        Affine::identity(),
        Some(&mask),
    );
    let stroke = Stroke {
        width: ring.1,
        ..Stroke::default()
    };
    cut.stroke_path(&circle, &paint(ring.0), &stroke, Affine::identity(), None);
    Some(Picture::of(cut, ("loupe", cx, cy, pixels, diameter)))
}

/// The arrow of the house cursor theme, as the boards draw it.
const ARROW: &str = include_str!("../../../assets/cursors/default.svg");

/// Draws the arrow cursor into `image` with its point at `at` (pixels of the image), at the
/// size 24 times `scale`, in the colors of the dark or `light` desktop (lot D-35e).
pub fn stamp_cursor(image: &mut Image, at: (f64, f64), scale: f64, light: bool) {
    let variant = if light {
        cursor_theme::Variant::Light
    } else {
        cursor_theme::Variant::Dark
    };
    let Ok(drawing) = cursor_theme::Drawing::read(ARROW) else {
        return;
    };
    let svg = variant.paint(ARROW, &drawing, 0.0);
    let Ok(tree) = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()) else {
        return;
    };
    let k = (24.0 * scale) as f32 / tree.size().width();
    let side = (tree.size().width() * k).ceil() as u32 + 2;
    let Some(mut cursor) = Pixmap::new(side, side) else {
        return;
    };
    resvg::render(&tree, Affine::from_scale(k, k), &mut cursor.as_mut());
    let (hx, hy) = (
        f64::from(drawing.hotspot.0 * k),
        f64::from(drawing.hotspot.1 * k),
    );
    let (ox, oy) = ((at.0 - hx).round() as i64, (at.1 - hy).round() as i64);
    let (w, h) = (i64::from(image.width), i64::from(image.height));
    for (i, p) in cursor.pixels().iter().enumerate() {
        let a = u32::from(p.alpha());
        if a == 0 {
            continue;
        }
        let (x, y) = (
            ox + (i as i64 % i64::from(side)),
            oy + (i as i64 / i64::from(side)),
        );
        if x < 0 || y < 0 || x >= w || y >= h {
            continue;
        }
        let at = ((y * w + x) * 4) as usize;
        let dst = &mut image.rgba[at..at + 4];
        // Premultiplied source over a straight destination.
        let src = [p.red(), p.green(), p.blue()];
        let da = u32::from(dst[3]);
        let out_a = a + da * (255 - a) / 255;
        for c in 0..3 {
            let below = u32::from(dst[c]) * da / 255 * (255 - a) / 255;
            let value = (u32::from(src[c]) + below) * 255 / out_a.max(1);
            dst[c] = value.min(255) as u8;
        }
        dst[3] = out_a.min(255) as u8;
    }
}

/// The textures of the pictures, one per key: the one of its last picture.
#[derive(Default)]
pub struct Pictures {
    textures: HashMap<Key, (u64, TextureBuffer<GlesTexture>)>,
}

impl Pictures {
    /// The element of `picture` drawn in `rect`, its texture made if it is new.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        key: Key,
        picture: &Picture,
        rect: Rectangle<i32, Physical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let fresh = self.textures.get(&key).filter(|(id, _)| *id == picture.id);
        let buffer = match fresh {
            Some((_, buffer)) => buffer.clone(),
            None => {
                let size = (picture.size.0 as i32, picture.size.1 as i32);
                let made = TextureBuffer::from_memory(
                    renderer,
                    &picture.rgba,
                    Fourcc::Abgr8888,
                    size,
                    false,
                    1,
                    Transform::Normal,
                    None,
                );
                match made {
                    Ok(buffer) => {
                        self.textures.insert(key, (picture.id, buffer.clone()));
                        buffer
                    }
                    Err(err) => {
                        tracing::warn!("picture texture: {err}");
                        return None;
                    }
                }
            }
        };
        Some(TextureRenderElement::from_texture_buffer(
            rect.loc.to_f64(),
            &buffer,
            Some(alpha),
            None,
            Some((rect.size.w, rect.size.h).into()),
            Kind::Unspecified,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32) -> Image {
        Image {
            width: w,
            height: h,
            rgba: (0..w * h)
                .flat_map(|i| [(i % w) as u8 * 10, (i / w) as u8 * 10, 200, 255])
                .collect(),
        }
    }

    #[test]
    fn the_thumbnail_is_rounded_and_shows_the_image() {
        let border = (Color::rgba(255, 255, 255, 0.5), 1.0);
        let t = thumbnail(&image(20, 10), (100, 50), 10.0, border, 1).unwrap();
        assert_eq!(t.size, (100, 50));
        // The corner is cut, the middle is the image, opaque.
        assert_eq!(t.rgba[3], 0);
        let mid = ((25 * 100 + 50) * 4) as usize;
        assert_eq!(t.rgba[mid + 3], 255);
        assert!(t.rgba[mid + 2] > 150);
        let other = thumbnail(&image(20, 10), (100, 50), 10.0, border, 2).unwrap();
        assert_ne!(t.id, other.id);
    }

    #[test]
    fn the_cursor_is_drawn_with_its_point_where_asked() {
        let mut img = Image {
            width: 60,
            height: 60,
            rgba: vec![0; 60 * 60 * 4],
        };
        stamp_cursor(&mut img, (20.0, 20.0), 1.0, false);
        let alpha = |x: usize, y: usize| img.rgba[(y * 60 + x) * 4 + 3];
        // Drawn below and right of its point, nothing far above or left of it.
        assert!(alpha(22, 28) > 200);
        assert_eq!(alpha(5, 5), 0);
        assert_eq!(alpha(50, 10), 0);
    }

    #[test]
    fn the_loupe_shows_each_pixel_large() {
        let ring = (Color::rgba(255, 255, 255, 1.0), 2.0);
        let mark = Color::rgba(133, 197, 149, 1.0);
        let l = loupe(&image(20, 20), (10, 10), (5, 100), ring, mark).unwrap();
        assert_eq!(l.size, (100, 100));
        // Inside the cell of the pixel (9, 10): its color, red 90, green 100.
        let at = ((50 * 100 + 30) * 4) as usize;
        assert_eq!(&l.rgba[at..at + 4], &[90, 100, 200, 255]);
        // Outside the disc: nothing.
        assert_eq!(l.rgba[3], 0);
        // Off the image: black.
        let edge = loupe(&image(20, 20), (0, 0), (5, 100), ring, mark).unwrap();
        let at = ((30 * 100 + 30) * 4) as usize;
        assert_eq!(&edge.rgba[at..at + 3], &[0, 0, 0]);
    }
}
