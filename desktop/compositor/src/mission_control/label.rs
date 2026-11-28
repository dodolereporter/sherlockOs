//! The titles of Mission Control: a pill of glass with a line of Inter, under each window and
//! each workspace thumbnail. Drawn on the processor like the title bars (`decorations::raster`),
//! into one texture per label, made again only when its text, size or colors change.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::OnceLock,
};

use ab_glyph::{Font as _, FontRef, PxScale, ScaleFont as _};
use sherlock_ui::{color::Color, decorations as ui, mission_control as mc};
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
use tiny_skia::Pixmap;

use crate::{damage::Key, decorations::raster};

/// What a label shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    /// In pixels, at `scale`.
    pub size: (u32, u32),
    pub scale: f32,
    pub background: Color,
    pub color: Color,
    /// The « × » of a window under the pointer (lot D-33c): a disc and the close icon of the
    /// title bars, no text.
    pub close: bool,
}

impl Label {
    pub fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (&self.text, self.size, self.scale.to_bits(), self.close).hash(&mut hasher);
        for c in [self.background, self.color] {
            (c.r, c.g, c.b, c.a.to_bits()).hash(&mut hasher);
        }
        hasher.finish()
    }
}

fn font() -> Option<&'static FontRef<'static>> {
    static FONT: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    FONT.get_or_init(raster::title_font).as_ref()
}

/// Width of the pill of `text`, in logical pixels: the text and its padding.
pub fn width(text: &str) -> f32 {
    let Some((font, units)) = font().and_then(|f| Some((f, f.units_per_em()?))) else {
        return 2.0 * mc::LABEL_PADDING;
    };
    let font = font.as_scaled(PxScale::from(
        ui::TITLE_SIZE * font.height_unscaled() / units,
    ));
    let glyphs = raster::layout(&font, text, f32::INFINITY);
    let text = glyphs.last().map_or(0.0, |(id, x)| x + font.h_advance(*id));
    text.ceil() + 2.0 * mc::LABEL_PADDING
}

/// Draws a label: premultiplied RGBA pixels.
pub fn draw(label: &Label) -> Option<Pixmap> {
    let (w, h) = label.size;
    let mut pixmap = Pixmap::new(w, h)?;
    let (wf, hf, s) = (w as f32, h as f32, label.scale);
    let radius = (mc::LABEL_RADIUS * s).min(hf / 2.0);
    if label.close {
        let side = wf.min(hf);
        let disc = raster::rounded((0.0, 0.0, side, side), side / 2.0, side / 2.0);
        raster::fill(&mut pixmap, disc, label.background);
        let icon = ui::Button::Close.icon(false);
        raster::stroke_icon(
            &mut pixmap,
            icon,
            (side / 2.0, side / 2.0),
            s * 0.75,
            label.color,
        );
        return Some(pixmap);
    }
    let pill = raster::rounded((0.0, 0.0, wf, hf), radius, radius);
    raster::fill(&mut pixmap, pill, label.background);
    if let Some(font) = font() {
        let padding = mc::LABEL_PADDING * s;
        let span = (padding, wf - padding, hf / 2.0);
        raster::title(&mut pixmap, font, &label.text, span, s, label.color);
    }
    Some(pixmap)
}

/// Frames a texture stays without being drawn, before it is freed.
const KEEP_FRAMES: u64 = 600;

/// The textures of the labels, one per key and scale (a thumbnail's name is drawn on every
/// screen, at each one's scale): the one of its last look.
#[derive(Default)]
pub struct Labels {
    textures: HashMap<(Key, u32), (u64, TextureBuffer<GlesTexture>, u64)>,
    frame: u64,
}

impl Labels {
    /// The element of the label of `key` drawn in `rect`, its texture made if it is new.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        key: Key,
        label: &Label,
        rect: Rectangle<i32, Physical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let hash = label.hash();
        let key = (key, label.scale.to_bits());
        let fresh = self.textures.get(&key).filter(|(h, ..)| *h == hash);
        let buffer = match fresh {
            Some((_, buffer, _)) => buffer.clone(),
            None => {
                let pixmap = draw(label)?;
                let size = (pixmap.width() as i32, pixmap.height() as i32);
                let format = Fourcc::Abgr8888;
                let normal = Transform::Normal;
                let made = TextureBuffer::from_memory(
                    renderer,
                    pixmap.data(),
                    format,
                    size,
                    false,
                    1,
                    normal,
                    None,
                );
                match made {
                    Ok(buffer) => buffer,
                    Err(err) => {
                        tracing::warn!("label texture: {err}");
                        return None;
                    }
                }
            }
        };
        self.textures
            .insert(key, (hash, buffer.clone(), self.frame));
        Some(TextureRenderElement::from_texture_buffer(
            rect.loc.to_f64(),
            &buffer,
            Some(alpha),
            None,
            Some((rect.size.w, rect.size.h).into()),
            Kind::Unspecified,
        ))
    }

    /// Frees at once the textures of Mission Control, no longer shown (D-33 review): the
    /// others (lock screen, screenshot tool) stay.
    pub fn forget_view(&mut self) {
        use crate::damage::Part;
        self.textures.retain(|(key, _), _| {
            !matches!(
                key.part,
                Part::Label | Part::CloseButton | Part::ThumbnailLabel(_)
            )
        });
    }

    /// End of a frame: the labels not drawn for a while are freed.
    pub fn sweep(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        self.textures
            .retain(|_, (.., used)| frame.saturating_sub(*used) < KEEP_FRAMES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_fits_its_text_and_draws_it() {
        let short = width("Konsole");
        assert!(
            short > 2.0 * mc::LABEL_PADDING + 20.0 && short < 120.0,
            "{short}"
        );
        assert!(width("Konsole — un titre bien plus long") > short);
        let label = Label {
            text: "Konsole".into(),
            size: (short as u32, mc::LABEL_HEIGHT as u32),
            scale: 1.0,
            background: Color::rgb(0, 0, 0),
            color: Color::rgb(255, 255, 255),
            close: false,
        };
        let pixmap = draw(&label).unwrap();
        // Some white of the text in the middle of the black pill.
        let lit = pixmap.pixels().iter().filter(|p| p.red() > 200).count();
        assert!(lit > 20, "{lit}");
    }
}
