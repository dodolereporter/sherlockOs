//! The on-screen display of the media keys (lot D-35): a pill of glass in the bottom middle of
//! the screen under the pointer, with the icon of the level and its gauge, in front of
//! everything. It comes and goes on a spring, and goes `osd::VISIBLE` after the last key. Drawn
//! on the processor like the titles of Mission Control, into one texture made again only when
//! what it shows changes. See `docs/media-keys.md`.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::OnceLock,
    time::Duration,
};

use ab_glyph::FontRef;
use resvg::{tiny_skia as skia, usvg};
use sherlock_ui::{
    color::{self, Color},
    motion::{Animation, SCALE_PRECISION},
    osd::{self as ui, Kind},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind as ElementKind,
                texture::{TextureBuffer, TextureRenderElement},
            },
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Physical, Rectangle, Transform},
};
use tiny_skia::{LineCap, Paint, Pixmap, Stroke};

use crate::{
    State,
    damage::{Key, Part},
    decorations::raster,
    output::View,
    render::Item,
};

/// What a media key changed, as the pill shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level {
    pub kind: Kind,
    /// In percent; above 100 the gauge is full.
    pub percent: u16,
    pub muted: bool,
}

/// The pill on a screen, coming, shown or going.
struct Shown {
    level: Level,
    output: Option<Output>,
    /// Its gauge, from 0 to 1.
    gauge: Animation,
    /// 0 out of sight, 1 in place.
    presence: Animation,
}

#[derive(Default)]
pub struct Osd {
    shown: Option<Shown>,
    hide: Option<smithay::reexports::calloop::RegistrationToken>,
}

fn fraction(level: &Level) -> f64 {
    f64::from(level.percent.min(100)) / 100.0
}

impl Osd {
    fn show(&mut self, level: Level, output: Option<Output>, now: Duration) {
        let to = fraction(&level);
        match &mut self.shown {
            // The same level again: its gauge follows, the pill stays where it is.
            Some(shown) if shown.level.kind == level.kind && shown.presence.target() > 0.5 => {
                shown.gauge.retarget(now, to);
                shown.level = level;
            }
            shown => {
                let presence = shown.as_ref().map_or(0.0, |s| s.presence.value(now));
                *shown = Some(Shown {
                    level,
                    output,
                    gauge: Animation::new(ui::GAUGE_SPRING, to, to, now),
                    presence: Animation::new(ui::SPRING, presence, 1.0, now),
                });
            }
        }
    }

    fn hide(&mut self, now: Duration) {
        if let Some(shown) = &mut self.shown {
            shown.presence.retarget(now, 0.0);
        }
    }

    /// What the pill shows, while it is on screen.
    pub fn level(&self) -> Option<Level> {
        self.shown
            .as_ref()
            .filter(|s| s.presence.target() > 0.5)
            .map(|s| s.level)
    }
}

impl State {
    /// Shows `level` on the screen under the pointer, and hides it `osd::VISIBLE` after the
    /// last one.
    pub fn show_osd(&mut self, level: Level) {
        let now = self.now();
        let output = self.pointer_output();
        self.osd.show(level, output, now);
        if let Some(token) = self.osd.hide.take() {
            self.loop_handle.remove(token);
        }
        let timer = Timer::from_duration(ui::VISIBLE);
        let token = self.loop_handle.insert_source(timer, |_, _, state| {
            state.osd.hide = None;
            let now = state.now();
            state.osd.hide(now);
            state.queue_redraw();
            TimeoutAction::Drop
        });
        match token {
            Ok(token) => self.osd.hide = Some(token),
            Err(err) => tracing::warn!("on-screen display timer: {err}"),
        }
        self.queue_redraw();
    }

    /// What the on-screen display shows, while it is on screen.
    pub fn osd_level(&self) -> Option<Level> {
        self.osd.level()
    }
}

/// What the texture of the pill shows: one is made for each look.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    pub kind: Kind,
    pub muted: bool,
    /// Width of the level in the gauge, in pixels.
    pub fill: u32,
    /// In pixels, at `scale`.
    pub size: (u32, u32),
    pub scale: f32,
    /// The step of the palette it was drawn with.
    pub palette: u8,
}

impl Look {
    pub fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        let kind = self.kind as u8;
        (kind, self.muted, self.fill, self.size, self.scale.to_bits()).hash(&mut hasher);
        self.palette.hash(&mut hasher);
        hasher.finish()
    }
}

/// The pill on `output`, in front of everything, and whether it moves.
pub fn push_item(items: &mut Vec<Item>, state: &State, (output, view): (&Output, &View)) -> bool {
    let Some(shown) = &state.osd.shown else {
        return false;
    };
    let now = state.now();
    let presence = shown.presence.value(now).clamp(0.0, 1.0);
    let animating = !shown.presence.is_done(now, SCALE_PRECISION)
        || !shown.gauge.is_done(now, SCALE_PRECISION);
    if shown.output.as_ref().is_some_and(|o| o != output) || presence < 0.01 && !animating {
        return animating;
    }
    let Some(screen) = state.space.output_geometry(output) else {
        return animating;
    };
    let (w, h) = (f64::from(ui::WIDTH), f64::from(ui::HEIGHT));
    let rise = f64::from(ui::RISE) * (1.0 - presence);
    let x = f64::from(screen.loc.x) + (f64::from(screen.size.w) - w) / 2.0;
    let y = f64::from(screen.loc.y + screen.size.h) - f64::from(ui::BOTTOM) - h + rise;
    let rect = view.rect(Rectangle::new((x, y).into(), (w, h).into()));
    let scale = view.scale as f32;
    let track = gauge_track(rect.size.w as f32, rect.size.h as f32, scale);
    let gauge = shown.gauge.value(now).clamp(0.0, 1.0) as f32;
    let fill = (track.2 * gauge).round().max(0.0) as u32;
    let look = Look {
        kind: shown.level.kind,
        muted: shown.level.muted,
        fill,
        size: (rect.size.w.max(1) as u32, rect.size.h.max(1) as u32),
        scale,
        palette: color::mix_step(),
    };
    items.push(Item::Osd {
        key: Key::of(None, Part::Osd),
        rect,
        look,
        alpha: presence as f32,
    });
    animating
}

/// Where the gauge goes in a pill of `w` by `h` pixels at scale `s`: left, middle, width.
fn gauge_track(w: f32, h: f32, s: f32) -> (f32, f32, f32) {
    let left = (ui::PADDING + ui::ICON_SIZE + ui::GAP) * s;
    (left, h / 2.0, (w - ui::PADDING * s - left).max(0.0))
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, (color.a * 255.0).round() as u8);
    paint.anti_alias = true;
    paint
}

/// A line of `width` pixels with round ends, from `x0` to `x1` at height `y`.
fn line(pixmap: &mut Pixmap, (x0, x1, y): (f32, f32, f32), width: f32, color: Color) {
    let mut path = tiny_skia::PathBuilder::new();
    path.move_to(x0, y);
    path.line_to(x1.max(x0 + 0.01), y);
    let Some(path) = path.finish() else {
        return;
    };
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    let identity = tiny_skia::Transform::identity();
    pixmap.stroke_path(&path, &paint(color), &stroke, identity, None);
}

/// Strokes `icon` of `size` pixels with its top left corner at `at`, through resvg: the icons
/// are SVG paths, arcs included.
fn icon(pixmap: &mut Pixmap, icon: sherlock_ui::icon::Icon, at: (f32, f32), size: f32, c: Color) {
    let rgb = format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
    let svg = icon
        .svg()
        .replace("stroke=\"#000\"", &format!("stroke=\"{rgb}\""));
    let tree = match usvg::Tree::from_str(&svg, &usvg::Options::default()) {
        Ok(tree) => tree,
        Err(err) => {
            tracing::warn!(?icon, "icon not understood: {err}");
            return;
        }
    };
    let k = size / tree.size().width();
    let transform = skia::Transform::from_row(k, 0.0, 0.0, k, at.0, at.1);
    let mut layer = pixmap.as_mut();
    resvg::render(&tree, transform, &mut layer);
}

fn font() -> Option<&'static FontRef<'static>> {
    static FONT: OnceLock<Option<FontRef<'static>>> = OnceLock::new();
    FONT.get_or_init(raster::title_font).as_ref()
}

/// Draws the pill: premultiplied RGBA pixels.
pub fn draw(look: &Look) -> Option<Pixmap> {
    let (w, h) = look.size;
    let mut pixmap = Pixmap::new(w, h)?;
    let (wf, hf, s) = (w as f32, h as f32, look.scale);
    let colors = ui::Colors::of(color::palette());
    let radius = (ui::RADIUS * s).min(hf / 2.0);
    let pill = raster::rounded((0.0, 0.0, wf, hf), radius, radius);
    raster::fill(&mut pixmap, pill, colors.border);
    let inset = sherlock_ui::metrics::BORDER_WIDTH * s;
    let inner = (inset, inset, wf - 2.0 * inset, hf - 2.0 * inset);
    raster::fill(
        &mut pixmap,
        raster::rounded(inner, radius - inset, radius - inset),
        colors.glass,
    );
    let size = ui::ICON_SIZE * s;
    let at = (ui::PADDING * s, (hf - size) / 2.0);
    icon(&mut pixmap, look.kind.icon(look.muted), at, size, colors.icon);
    let (left, middle, width) = gauge_track(wf, hf, s);
    let thickness = ui::GAUGE_HEIGHT * s;
    // The round ends stick out of the line by half its thickness.
    let (start, end) = (left + thickness / 2.0, left + width - thickness / 2.0);
    if look.kind.has_gauge() {
        line(&mut pixmap, (start, end, middle), thickness, colors.track);
        if look.fill > 0 {
            let fill = if look.muted {
                colors.fill_muted
            } else {
                colors.fill
            };
            let to = (start + look.fill as f32 - thickness).max(start);
            line(&mut pixmap, (start, to, middle), thickness, fill);
        }
    } else if let Some(font) = font() {
        let text = if look.muted {
            ui::MICROPHONE_OFF
        } else {
            ui::MICROPHONE_ON
        };
        raster::title(&mut pixmap, font, text, (left, wf, middle), s, colors.icon);
    }
    Some(pixmap)
}

/// The texture of the pill, made again when its look changes.
#[derive(Default)]
pub struct Texture {
    made: Option<(u64, TextureBuffer<GlesTexture>)>,
}

impl Texture {
    /// The element of the pill drawn in `rect`.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        look: &Look,
        rect: Rectangle<i32, Physical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let hash = look.hash();
        let buffer = match &self.made {
            Some((made, buffer)) if *made == hash => buffer.clone(),
            _ => {
                let pixmap = draw(look)?;
                let size = (pixmap.width() as i32, pixmap.height() as i32);
                let made = TextureBuffer::from_memory(
                    renderer,
                    pixmap.data(),
                    Fourcc::Abgr8888,
                    size,
                    false,
                    1,
                    Transform::Normal,
                    None,
                );
                match made {
                    Ok(buffer) => {
                        self.made = Some((hash, buffer.clone()));
                        buffer
                    }
                    Err(err) => {
                        tracing::warn!("on-screen display texture: {err}");
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
            ElementKind::Unspecified,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look(kind: Kind, muted: bool, fill: u32) -> Look {
        Look {
            kind,
            muted,
            fill,
            size: (ui::WIDTH as u32, ui::HEIGHT as u32),
            scale: 1.0,
            palette: 0,
        }
    }

    /// Pixels of `pixmap` close to `color`.
    fn count(pixmap: &Pixmap, color: Color) -> usize {
        let near = |a: u8, b: u8| a.abs_diff(b) < 12;
        pixmap
            .pixels()
            .iter()
            .map(|p| p.demultiply())
            .filter(|p| {
                p.alpha() > 240
                    && near(p.red(), color.r)
                    && near(p.green(), color.g)
                    && near(p.blue(), color.b)
            })
            .count()
    }

    #[test]
    fn the_pill_draws_its_icon_and_its_level() {
        let colors = ui::Colors::of(color::Theme::Dark.palette());
        let empty = draw(&look(Kind::Volume, false, 0)).unwrap();
        let half = draw(&look(Kind::Volume, false, 80)).unwrap();
        assert_eq!(count(&empty, colors.fill), 0);
        let lit = count(&half, colors.fill);
        assert!(lit > 300, "{lit}");
        // Muted, the level takes the muted color.
        let muted = draw(&look(Kind::Volume, true, 80)).unwrap();
        assert_eq!(count(&muted, colors.fill), 0);
        // The icon, in the text color, on the left.
        let strokes = (0..ui::HEIGHT as u32)
            .flat_map(|y| (0..(ui::PADDING + ui::ICON_SIZE) as u32).map(move |x| (x, y)))
            .filter_map(|(x, y)| half.pixel(x, y))
            .filter(|p| p.demultiply().red() > 200)
            .count();
        assert!(strokes > 30, "{strokes}");
    }

    #[test]
    fn the_same_level_keeps_the_pill_and_another_replaces_it() {
        let mut osd = Osd::default();
        let volume = |percent| Level {
            kind: Kind::Volume,
            percent,
            muted: false,
        };
        let t = Duration::from_secs(1);
        osd.show(volume(50), None, t);
        osd.show(volume(55), None, t + Duration::from_millis(40));
        let shown = osd.shown.as_ref().unwrap();
        assert!((shown.gauge.target() - 0.55).abs() < 1e-9);
        assert_eq!(osd.level(), Some(volume(55)));
        let light = Level {
            kind: Kind::Brightness,
            percent: 120,
            muted: false,
        };
        osd.show(light, None, t + Duration::from_millis(80));
        assert!((osd.shown.as_ref().unwrap().gauge.target() - 1.0).abs() < 1e-9);
        osd.hide(t + Duration::from_secs(2));
        assert_eq!(osd.level(), None);
    }
}
