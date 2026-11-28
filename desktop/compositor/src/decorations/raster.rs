//! The bar of a decoration, drawn on the processor into one texture: its background with the
//! top corners of the window, the divider, the title in Inter, the pill and its three buttons.
//! A texture is made again only when what it shows changes (its `Look`): at rest, nothing is
//! drawn nor uploaded, and its element keeps its identity (see `damage.rs`).
//!
//! The title is laid out by ab_glyph, glyph after glyph with the kerning of the font, without
//! shaping: enough for the scripts Inter covers. tiny-skia fills the shapes and strokes the
//! icons of `sherlock_ui::icon`, with their round caps and joins.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, VariableFont, point};
use sherlock_ui::{
    color::Color,
    decorations::{self as ui, Button, Colors},
    metrics, typography,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{Kind, texture::TextureBuffer, texture::TextureRenderElement},
            gles::{GlesRenderer, GlesTexture},
        },
    },
    utils::{Buffer, Physical, Rectangle, Transform},
};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke};

use crate::damage::Key;

/// What a bar shows, besides its size and colors.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Bar {
    pub title: String,
    /// The three buttons, from left to right, as their motion has them now.
    pub buttons: [ButtonLook; 3],
    /// Share of the shown color of the icons, out of 255: the pointer is on the pill.
    pub pill: u8,
    /// The maximize button shows the restore icon.
    pub maximized: bool,
}

/// A button as drawn: rounded so that a settled button gives the same texture every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ButtonLook {
    /// Share of its hover color, out of 255.
    pub lit: u8,
    /// Its scale, in thousandths.
    pub scale: u16,
}

impl Default for ButtonLook {
    fn default() -> Self {
        Self {
            lit: 0,
            scale: 1000,
        }
    }
}

/// Everything a bar texture depends on.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub bar: Bar,
    /// In pixels: the width of the window and the height of the decoration, at `scale`.
    pub size: (u32, u32),
    pub scale: f32,
    /// Of the top corners, in pixels: the window's, none when it is maximized.
    pub radius: f32,
    pub colors: Colors,
}

impl Look {
    /// A hash of the look, for `Ids::stamp` and to find its texture again.
    pub fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.bar.hash(&mut hasher);
        self.size.hash(&mut hasher);
        let c = &self.colors;
        for color in [
            c.bar,
            c.divider,
            c.title,
            c.controls,
            c.hover,
            c.pressed,
            c.icon,
            c.icon_shown,
            c.close_hover,
            c.close_icon,
        ] {
            (color.r, color.g, color.b, color.a.to_bits()).hash(&mut hasher);
        }
        (self.scale.to_bits(), self.radius.to_bits()).hash(&mut hasher);
        hasher.finish()
    }
}

/// Frames a texture stays without being drawn, before it is freed.
const KEEP_FRAMES: u64 = 600;

/// A bar texture: its window, its scale, and whether it is stretched.
type Slot = (Key, u32, bool);

/// The textures of the bars: one per window, scale and use (desktop or stretched), the one of
/// its last look.
pub struct Textures {
    font: Option<FontRef<'static>>,
    bars: HashMap<Slot, (u64, TextureBuffer<GlesTexture>, u64)>,
    frame: u64,
}

impl Default for Textures {
    fn default() -> Self {
        Self {
            font: title_font(),
            bars: HashMap::new(),
            frame: 0,
        }
    }
}

impl Textures {
    /// The element of the bar of `key` drawn with `look` (of hash `hash`) in `rect`, its
    /// texture made if the look is new.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        key: Key,
        (look, hash): (&Look, u64),
        rect: Rectangle<i32, Physical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        // Drawn smaller than its pixels (Mission Control): a texture of its own, without an
        // opaque region, which smithay rounds up past the element once stretched.
        let stretched = (rect.size.w as u32, rect.size.h as u32) != look.size;
        let slot = (key, look.scale.to_bits(), stretched);
        let fresh = self.bars.get(&slot).filter(|(h, ..)| *h == hash);
        let buffer = match fresh {
            Some((_, buffer, _)) => buffer.clone(),
            None => {
                let pixmap = draw(look, self.font.as_ref())?;
                let size = (pixmap.width() as i32, pixmap.height() as i32);
                let solid = look.colors.bar.a >= 1.0 && !stretched;
                let opaque = solid.then(|| opaque(size, look.radius));
                let made = TextureBuffer::from_memory(
                    renderer,
                    pixmap.data(),
                    Fourcc::Abgr8888,
                    size,
                    false,
                    1,
                    Transform::Normal,
                    opaque,
                );
                match made {
                    Ok(buffer) => buffer,
                    Err(err) => {
                        tracing::warn!("title bar texture: {err}");
                        return None;
                    }
                }
            }
        };
        self.bars.insert(slot, (hash, buffer.clone(), self.frame));
        let size = (rect.size.w, rect.size.h).into();
        // The whole texture, stretched into `rect`: Mission Control draws the bar smaller.
        let (w, h) = look.size;
        let src = Rectangle::from_size((f64::from(w), f64::from(h)).into());
        Some(TextureRenderElement::from_texture_buffer(
            rect.loc.to_f64(),
            &buffer,
            Some(alpha),
            Some(src),
            Some(size),
            Kind::Unspecified,
        ))
    }

    /// End of a frame: the textures of windows no longer drawn for a while are freed.
    pub fn sweep(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.bars
            .retain(|_, (.., used)| frame.saturating_sub(*used) < KEEP_FRAMES);
    }
}

/// The opaque part of a bar of `size` pixels: all of it but its top corners.
fn opaque((w, h): (i32, i32), radius: f32) -> Vec<Rectangle<i32, Buffer>> {
    let r = (radius.ceil() as i32 + 1).min(h);
    vec![
        Rectangle::new((r, 0).into(), ((w - 2 * r).max(0), r).into()),
        Rectangle::new((0, r).into(), (w, h - r).into()),
    ]
}

/// Inter, at the weight of the title. `None` if the font cannot be read: the bars go without
/// their title.
pub(crate) fn title_font() -> Option<FontRef<'static>> {
    match FontRef::try_from_slice(typography::FONTS[0]) {
        Ok(mut font) => {
            font.set_variation(b"wght", f32::from(ui::TITLE_WEIGHT));
            Some(font)
        }
        Err(err) => {
            tracing::warn!("title font: {err}");
            None
        }
    }
}

/// Draws a bar: premultiplied RGBA pixels, as tiny-skia keeps them.
pub fn draw(look: &Look, font: Option<&FontRef<'_>>) -> Option<Pixmap> {
    let (w, h) = look.size;
    let mut pixmap = Pixmap::new(w, h)?;
    let (s, c) = (look.scale, &look.colors);
    let (wf, hf) = (w as f32, h as f32);
    fill(
        &mut pixmap,
        rounded((0.0, 0.0, wf, hf), look.radius, 0.0),
        c.bar,
    );
    let divider = ui::DIVIDER_WIDTH * s;
    fill(
        &mut pixmap,
        rounded((0.0, hf - divider, wf, divider), 0.0, 0.0),
        c.divider,
    );
    let width = f64::from(wf / s);
    let px = |r: Rectangle<f64, smithay::utils::Logical>| {
        let (s, r) = (f64::from(s), r);
        let px = [r.loc.x * s, r.loc.y * s, r.size.w * s, r.size.h * s].map(|v| v as f32);
        (px[0], px[1], px[2], px[3])
    };
    let pill = px(super::pill(width));
    fill(
        &mut pixmap,
        rounded(pill, ui::CONTROLS_RADIUS * s, ui::CONTROLS_RADIUS * s),
        c.controls,
    );
    let shown = f32::from(look.bar.pill) / 255.0;
    for ((button, rect), state) in super::buttons(width).into_iter().zip(look.bar.buttons) {
        let (x, y, w, h) = px(rect);
        let center = (x + w / 2.0, y + h / 2.0);
        // Grown or sunk around its center.
        let k = f32::from(state.scale) / 1000.0;
        let rect = (center.0 - w * k / 2.0, center.1 - h * k / 2.0, w * k, h * k);
        let lit = f32::from(state.lit) / 255.0;
        // Sunk below its rest size: held down.
        let down = k < 1.0;
        let (veil, icon) = match button {
            Button::Close => (c.close_hover, c.close_icon),
            _ if down => (c.pressed, c.icon_shown),
            _ => (c.hover, c.icon_shown),
        };
        if lit > 0.0 {
            let r = ui::BUTTON_RADIUS * s * k;
            let veil = Color {
                a: veil.a * lit,
                ..veil
            };
            fill(&mut pixmap, rounded(rect, r, r), veil);
        }
        // The icons at rest, all three shown on the pill, then the button's own.
        let color = c.icon.lerp(c.icon_shown, shown).lerp(icon, lit);
        stroke_icon(
            &mut pixmap,
            button.icon(look.bar.maximized),
            center,
            s * k,
            color,
        );
    }
    if let Some(font) = font {
        let start = (metrics::BORDER_WIDTH + ui::PADDING.0) * s;
        let end = pill.0 - ui::TITLE_GAP * s;
        let middle = (metrics::BORDER_WIDTH + ui::BAR_HEIGHT / 2.0) * s;
        title(
            &mut pixmap,
            font,
            &look.bar.title,
            (start, end, middle),
            s,
            c.title,
        );
    }
    Some(pixmap)
}

/// A rectangle `(x, y, w, h)` with its top corners of radius `top` and its bottom ones of
/// radius `bottom`.
pub(crate) fn rounded((x, y, w, h): (f32, f32, f32, f32), top: f32, bottom: f32) -> Option<Path> {
    let (top, bottom) = (top.min(w / 2.0).min(h), bottom.min(w / 2.0).min(h - top));
    // Distance of the control points of a quarter circle from its corner, as a share of the
    // radius: 1 - 0.5523.
    let k = 0.447_7;
    let (r, b) = (x + w, y + h);
    let mut path = PathBuilder::new();
    path.move_to(x + top, y);
    path.line_to(r - top, y);
    path.cubic_to(r - top * k, y, r, y + top * k, r, y + top);
    path.line_to(r, b - bottom);
    path.cubic_to(r, b - bottom * k, r - bottom * k, b, r - bottom, b);
    path.line_to(x + bottom, b);
    path.cubic_to(x + bottom * k, b, x, b - bottom * k, x, b - bottom);
    path.line_to(x, y + top);
    path.cubic_to(x, y + top * k, x + top * k, y, x + top, y);
    path.close();
    path.finish()
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    let alpha = (color.a * 255.0).round() as u8;
    paint.set_color_rgba8(color.r, color.g, color.b, alpha);
    paint.anti_alias = true;
    paint
}

pub(crate) fn fill(pixmap: &mut Pixmap, path: Option<Path>, color: Color) {
    if let Some(path) = path {
        let identity = tiny_skia::Transform::identity();
        pixmap.fill_path(&path, &paint(color), FillRule::Winding, identity, None);
    }
}

/// Strokes the icon `d`, an SVG path on the grid of `icon`, centered on `center`.
pub(crate) fn stroke_icon(pixmap: &mut Pixmap, d: &str, center: (f32, f32), s: f32, color: Color) {
    let Some(path) = icon_path(d) else {
        tracing::warn!(d, "icon path not understood");
        return;
    };
    let size = ui::ICON_SIZE * s;
    let k = size / metrics::ICON_GRID;
    let (x, y) = (center.0 - size / 2.0, center.1 - size / 2.0);
    let transform = tiny_skia::Transform::from_row(k, 0.0, 0.0, k, x, y);
    let stroke = Stroke {
        width: ui::ICON_STROKE,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint(color), &stroke, transform, None);
}

/// The SVG path `d` of an icon: only the commands of the window icons, `M`, `L`, `H`, `V` and
/// `Z`, absolute or relative.
fn icon_path(d: &str) -> Option<Path> {
    let mut path = PathBuilder::new();
    let (mut command, mut args, mut at) = ('M', Vec::new(), (0.0f32, 0.0f32));
    let mut rest = d;
    while let Some(c) = rest.chars().next() {
        if c.is_ascii_alphabetic() {
            command = c;
            rest = &rest[1..];
            if c.eq_ignore_ascii_case(&'z') {
                path.close();
            }
            continue;
        }
        if c == ' ' || c == ',' {
            rest = &rest[1..];
            continue;
        }
        // A number: a sign, digits and one point; a second sign or point starts the next one.
        let len = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| !c.is_ascii_digit() && *c != '.')
            .map_or(rest.len(), |(i, _)| i);
        args.push(rest[..len].parse::<f32>().ok()?);
        rest = &rest[len..];
        let relative = command.is_ascii_lowercase();
        let base = if relative { at } else { (0.0, 0.0) };
        at = match (command.to_ascii_uppercase(), args.as_slice()) {
            ('M' | 'L', [x, y]) => (base.0 + x, base.1 + y),
            ('H', [x]) => (base.0 + x, at.1),
            ('V', [y]) => (at.0, base.1 + y),
            ('M' | 'L', [_]) => continue,
            _ => return None,
        };
        if command.eq_ignore_ascii_case(&'m') {
            path.move_to(at.0, at.1);
        } else {
            path.line_to(at.0, at.1);
        }
        args.clear();
    }
    path.finish()
}

/// Writes `text` from `start`, centered on `middle` in height, at the title size times `s`.
/// What does not fit before `end` ends with an ellipsis.
pub(crate) fn title(
    pixmap: &mut Pixmap,
    font: &FontRef<'_>,
    text: &str,
    (start, end, middle): (f32, f32, f32),
    s: f32,
    color: Color,
) {
    let Some(units) = font.units_per_em() else {
        return;
    };
    // `PxScale` is the height from descent to ascent, not the size of the em.
    let scale = PxScale::from(ui::TITLE_SIZE * s * font.height_unscaled() / units);
    let font = font.as_scaled(scale);
    let glyphs = layout(&font, text, end - start);
    let baseline = middle + (font.ascent() + font.descent()) / 2.0;
    let rgba = color.premultiplied();
    for (id, x) in glyphs {
        let glyph = id.with_scale_and_position(scale, point(start + x, baseline));
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let x = bounds.min.x as i32 + gx as i32;
            let y = bounds.min.y as i32 + gy as i32;
            if (x as f32) < end {
                blend(pixmap, (x, y), rgba, coverage);
            }
        });
    }
}

/// The glyphs of `text` and their place, kerned, cut with an ellipsis to fit in `room`.
pub fn layout<F: ScaleFont<G>, G: Font>(font: &F, text: &str, room: f32) -> Vec<(GlyphId, f32)> {
    let mut glyphs = Vec::new();
    let (mut x, mut previous) = (0.0, None);
    for c in text.chars().filter(|c| !c.is_control()) {
        let id = font.glyph_id(c);
        x += previous.map_or(0.0, |p| font.kern(p, id));
        glyphs.push((id, x, c));
        x += font.h_advance(id);
        previous = Some(id);
    }
    if x <= room {
        return glyphs.into_iter().map(|(id, x, _)| (id, x)).collect();
    }
    let ellipsis = font.glyph_id(ui::ELLIPSIS);
    let room = room - font.h_advance(ellipsis);
    // Whole glyphs that end before the ellipsis, without the spaces before it.
    let mut kept: Vec<_> = glyphs
        .iter()
        .take_while(|(id, x, _)| x + font.h_advance(*id) <= room)
        .collect();
    while kept.last().is_some_and(|(.., c)| c.is_whitespace()) {
        kept.pop();
    }
    let end = kept
        .last()
        .map_or(0.0, |(id, x, _)| x + font.h_advance(*id));
    let mut cut: Vec<_> = kept.into_iter().map(|(id, x, _)| (*id, *x)).collect();
    cut.push((ellipsis, end));
    cut
}

/// Lays the premultiplied `rgba` over the pixel `(x, y)` with `coverage`.
fn blend(pixmap: &mut Pixmap, (x, y): (i32, i32), rgba: [f32; 4], coverage: f32) {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    if !(0..w).contains(&x) || !(0..h).contains(&y) {
        return;
    }
    let i = (y * w + x) as usize * 4;
    let coverage = coverage.clamp(0.0, 1.0);
    let keep = 1.0 - rgba[3] * coverage;
    for (byte, channel) in pixmap.data_mut()[i..i + 4].iter_mut().zip(rgba) {
        let value = channel * coverage * 255.0 + f32::from(*byte) * keep;
        *byte = value.round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_icons_are_understood() {
        for maximized in [false, true] {
            for button in Button::ALL {
                assert!(icon_path(button.icon(maximized)).is_some(), "{button:?}");
            }
        }
        // The close icon: two strokes from corner to corner of the 6 to 18 square.
        let close = icon_path(Button::Close.icon(false)).unwrap();
        let bounds = close.bounds();
        assert_eq!((bounds.left(), bounds.top()), (6.0, 6.0));
        assert_eq!((bounds.right(), bounds.bottom()), (18.0, 18.0));
        // Relative and negative numbers: the restore icon closes its back square at 14.
        let restore = icon_path(ui::RESTORE_ICON).unwrap();
        assert_eq!(restore.bounds().right(), 17.0);
        assert_eq!(restore.bounds().top(), 7.0);
    }

    #[test]
    fn a_long_title_ends_with_an_ellipsis() {
        let font = title_font().unwrap();
        let units = font.units_per_em().unwrap();
        let scale = PxScale::from(ui::TITLE_SIZE * font.height_unscaled() / units);
        let font = font.as_scaled(scale);
        let short = layout(&font, "dock.rs", 300.0);
        assert_eq!(short.len(), 7);
        let long = layout(
            &font,
            "compositor — a title far too long for its bar",
            120.0,
        );
        let (last, x) = *long.last().unwrap();
        assert_eq!(last, font.glyph_id(ui::ELLIPSIS));
        assert!(x + font.h_advance(last) <= 120.0);
        assert!(long.len() < 20);
        // No room at all: the ellipsis alone.
        assert_eq!(layout(&font, "compositor", 1.0).len(), 1);
    }
}
