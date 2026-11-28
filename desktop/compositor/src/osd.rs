//! The on-screen display of the media keys (lots D-35, D-35d): a pill of glass in the bottom
//! middle of the screen under the pointer, above the dock, with the icon of the level and its
//! gauge, in front of everything. Its size follows the screen (`osd::fit`). It grows in and fades
//! in, then the other way `osd::VISIBLE` after the last key; its gauge slides on a spring; a key
//! that insists at the end stretches it, as on macOS and iOS. Drawn on the processor like the
//! titles of Mission Control, into one texture made again only when what it shows changes. See
//! `docs/media-keys.md`.

use std::{
    hash::{DefaultHasher, Hash, Hasher},
    sync::OnceLock,
    time::Duration,
};

use ab_glyph::FontRef;
use resvg::{tiny_skia as skia, usvg};
use sherlock_ui::{
    color::{self, Color},
    motion::{Animation, Fade, SCALE_PRECISION},
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
    desktop::layer_map_for_output,
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::{Logical, Physical, Rectangle, Transform},
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

/// The end of the gauge a key pushed against: « moins » at the bottom, « plus » at the top.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Low,
    High,
}

/// A key that insisted at an end: the pill stretches towards it, then comes back.
#[derive(Debug, Clone, Copy)]
struct Push {
    edge: Edge,
    /// Towards the full stretch, from the press on.
    rise: Animation,
    at: Duration,
}

impl Push {
    /// The stretch at `now`: towards 1 until `STRETCH_HOLD` after the press, then back to 0 on
    /// the same spring, with its speed.
    fn animation(&self, now: Duration) -> Animation {
        let release = self.at + ui::STRETCH_HOLD;
        let mut stretch = self.rise;
        if now >= release {
            stretch.retarget(release, 0.0);
        }
        stretch
    }

    fn done(&self, now: Duration) -> bool {
        now >= self.at + ui::STRETCH_HOLD && self.animation(now).is_done(now, SCALE_PRECISION)
    }
}

/// The pill on a screen, coming, shown or going.
struct Shown {
    level: Level,
    output: Option<Output>,
    /// Its gauge, from 0 to 1.
    gauge: Animation,
    /// 0 out of sight, 1 in place.
    presence: Fade,
    push: Option<Push>,
}

impl Shown {
    fn stretch(&self, now: Duration) -> f64 {
        self.push.map_or(0.0, |push| push.animation(now).value(now))
    }

    /// A key insisted at `edge`: the stretch aims at its full length again, from where it is,
    /// never past it.
    fn push(&mut self, edge: Edge, now: Duration) {
        let rise = match self.push {
            Some(push) if push.edge == edge => {
                let mut rise = push.animation(now);
                rise.retarget(now, 1.0);
                rise
            }
            _ => Animation::new(ui::STRETCH_SPRING, 0.0, 1.0, now),
        };
        self.push = Some(Push {
            edge,
            rise,
            at: now,
        });
    }
}

#[derive(Default)]
pub struct Osd {
    shown: Option<Shown>,
    hide: Option<smithay::reexports::calloop::RegistrationToken>,
}

fn fraction(level: &Level) -> f64 {
    f64::from(level.percent.min(100)) / 100.0
}

/// The pill at an instant, on its screen: what the renderer needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub level: Level,
    /// Where it is drawn, in logical pixels, scaled while it comes or goes.
    pub rect: Rectangle<f64, Logical>,
    /// Its size before that scale: stretched, never scaled.
    pub size: (f64, f64),
    /// The scale of the pill and of its content on its screen (`osd::fit`).
    pub fit: f64,
    pub alpha: f32,
    /// Its gauge, from 0 to 1.
    pub gauge: f64,
    /// Something still moves: another frame is needed.
    pub moving: bool,
}

impl Osd {
    /// Shows `level`, `against` an end when a key insisted there. The same kind of level keeps
    /// the pill, still there or going, and slides its gauge; another kind replaces it.
    pub fn show(
        &mut self,
        level: Level,
        against: Option<Edge>,
        output: Option<Output>,
        now: Duration,
    ) {
        let to = fraction(&level);
        let presence = self.shown.as_ref().map_or(0.0, |s| s.presence.value(now));
        let enter = Fade::eased(
            presence,
            1.0,
            now,
            ui::ENTER.mul_f32(1.0 - presence),
            ui::ENTER_EASE,
        );
        match &mut self.shown {
            Some(shown) if shown.level.kind == level.kind && presence > 0.0 => {
                shown.gauge.retarget(now, to);
                shown.level = level;
                if shown.presence.target() < 1.0 {
                    shown.presence = enter;
                }
            }
            shown => {
                *shown = Some(Shown {
                    level,
                    output,
                    gauge: Animation::new(ui::GAUGE_SPRING, to, to, now),
                    presence: enter,
                    push: None,
                });
            }
        }
        if let (Some(shown), Some(edge)) = (&mut self.shown, against) {
            shown.push(edge, now);
        }
    }

    /// The pill goes: it shrinks and fades out.
    pub fn hide(&mut self, now: Duration) {
        if let Some(shown) = &mut self.shown {
            let presence = shown.presence.value(now);
            let duration = ui::LEAVE.mul_f32(presence);
            shown.presence = Fade::eased(presence, 0.0, now, duration, ui::LEAVE_EASE);
        }
    }

    /// What the pill shows, while it is on screen.
    pub fn level(&self) -> Option<Level> {
        self.shown
            .as_ref()
            .filter(|s| s.presence.target() > 0.5)
            .map(|s| s.level)
    }

    /// How far the pill is stretched at `now`, 1 for `osd::STRETCH` of its length.
    pub fn stretch(&self, now: Duration) -> f64 {
        self.shown.as_ref().map_or(0.0, |s| s.stretch(now))
    }

    /// The pill at `now` on `screen`, whose dock reserves `reserved` logical pixels at the
    /// bottom; `None` once it is gone.
    pub fn frame(
        &self,
        now: Duration,
        screen: Rectangle<f64, Logical>,
        reserved: f64,
    ) -> Option<Frame> {
        let shown = self.shown.as_ref()?;
        let presence = f64::from(shown.presence.value(now).clamp(0.0, 1.0));
        let moving = !shown.presence.is_done(now)
            || !shown.gauge.is_done(now, SCALE_PRECISION)
            || shown.push.is_some_and(|push| !push.done(now));
        if presence <= 0.0 && !moving {
            return None;
        }
        let fit = f64::from(ui::fit(screen.size.w as f32));
        let (w, h) = (f64::from(ui::WIDTH) * fit, f64::from(ui::HEIGHT) * fit);
        let bottom = ui::bottom(screen.size.h as f32, reserved as f32, fit as f32);
        let middle = screen.loc.y + screen.size.h - f64::from(bottom) - h / 2.0;
        let center = screen.loc.x + screen.size.w / 2.0;
        let stretch = shown.stretch(now);
        let (sw, sh) = (
            w * (1.0 + ui::STRETCH * stretch),
            h * (1.0 - ui::STRETCH * ui::STRETCH_THIN * stretch),
        );
        // The end away from the key stays; the other one goes out.
        let left = match shown.push.map(|push| push.edge) {
            Some(Edge::High) => center - w / 2.0,
            Some(Edge::Low) => center + w / 2.0 - sw,
            None => center - sw / 2.0,
        };
        let scale = ui::SCALE + (1.0 - ui::SCALE) * presence;
        let (dw, dh) = (sw * scale, sh * scale);
        let x = left + (sw - dw) / 2.0;
        Some(Frame {
            level: shown.level,
            rect: Rectangle::new((x, middle - dh / 2.0).into(), (dw, dh).into()),
            size: (sw, sh),
            fit,
            alpha: presence as f32,
            gauge: shown.gauge.value(now).clamp(0.0, 1.0),
            moving,
        })
    }
}

impl Frame {
    /// The look of the pill drawn at `scale` (the screen's).
    pub fn look(&self, scale: f64) -> Look {
        let size = (
            (self.size.0 * scale).round().max(1.0) as u32,
            (self.size.1 * scale).round().max(1.0) as u32,
        );
        let s = (scale * self.fit) as f32;
        let track = gauge_track(size.0 as f32, size.1 as f32, s);
        Look {
            kind: self.level.kind,
            muted: self.level.muted,
            fill: (track.2 * self.gauge as f32).round().max(0.0) as u32,
            size,
            scale: s,
            palette: color::mix_step(),
        }
    }
}

impl State {
    /// Shows `level` on the screen under the pointer, `against` an end when a key insisted
    /// there, and hides it `osd::VISIBLE` after the last one.
    pub fn show_osd(&mut self, level: Level, against: Option<Edge>) {
        let now = self.now();
        let output = self.pointer_output();
        self.osd.show(level, against, output, now);
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

    /// How far it is stretched now (`Osd::stretch`).
    pub fn osd_stretch(&self) -> f64 {
        self.osd.stretch(self.now())
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
    if shown.output.as_ref().is_some_and(|o| o != output) {
        return false;
    }
    let Some(screen) = state.space.output_geometry(output) else {
        return false;
    };
    // What the dock, or any surface anchored at the bottom, keeps from the windows.
    let zone = layer_map_for_output(output).non_exclusive_zone();
    let reserved = (screen.size.h - zone.loc.y - zone.size.h).max(0);
    let Some(frame) = state
        .osd
        .frame(state.now(), screen.to_f64(), f64::from(reserved))
    else {
        return false;
    };
    let (rect, look) = (view.rect(frame.rect), frame.look(view.scale));
    // The blur behind its glass, over the pill as `draw` rounds it (lot D-37b).
    let radius = (ui::RADIUS * look.scale).min(rect.size.h as f32 / 2.0);
    let rects = crate::render::rounded_rects(rect, radius);
    items.push(Item::Osd {
        key: Key::of(None, Part::Osd),
        rect,
        look,
        alpha: frame.alpha,
    });
    items.push(Item::Blur {
        key: Key::of(None, Part::Blur),
        rects,
        alpha: frame.alpha,
    });
    frame.moving
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
    icon(
        &mut pixmap,
        look.kind.icon(look.muted),
        at,
        size,
        colors.icon,
    );
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
        osd.show(volume(50), None, None, t);
        osd.show(volume(55), None, None, t + Duration::from_millis(40));
        let shown = osd.shown.as_ref().unwrap();
        assert!((shown.gauge.target() - 0.55).abs() < 1e-9);
        assert_eq!(osd.level(), Some(volume(55)));
        let light = Level {
            kind: Kind::Brightness,
            percent: 120,
            muted: false,
        };
        osd.show(light, None, None, t + Duration::from_millis(80));
        assert!((osd.shown.as_ref().unwrap().gauge.target() - 1.0).abs() < 1e-9);
        osd.hide(t + Duration::from_secs(2));
        assert_eq!(osd.level(), None);
    }

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

    /// A screen of 1920 by 1080 logical pixels, its dock reserving nothing.
    fn screen() -> Rectangle<f64, Logical> {
        Rectangle::new((0.0, 0.0).into(), (1920.0, 1080.0).into())
    }

    /// The pill settled at 100 %, then « plus » pressed again at `at`, against `edge`.
    fn pushed(edge: Edge, at: Duration) -> Osd {
        let mut osd = Osd::default();
        osd.show(volume(100), None, None, ms(0));
        osd.show(volume(100), Some(edge), None, at);
        osd
    }

    /// The largest stretch from `from` for `span`, every millisecond.
    fn peak(osd: &Osd, from: Duration, span: u64) -> f64 {
        (0..span)
            .map(|t| osd.stretch(from + ms(t)))
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_key_that_insists_at_the_top_stretches_the_pill_to_the_right() {
        let osd = pushed(Edge::High, ms(1000));
        let rest = osd.frame(ms(999), screen(), 0.0).unwrap();
        let top = peak(&osd, ms(1000), 400);
        // About 5 % of its length, then back to rest on the spring.
        assert!(top > 0.9 && top < 1.1, "{top}");
        let at = (0..400)
            .map(|t| ms(1000 + t))
            .find(|t| osd.stretch(*t) >= top - 1e-9);
        let frame = osd.frame(at.unwrap(), screen(), 0.0).unwrap();
        let grown = frame.size.0 / rest.size.0 - 1.0;
        assert!(grown > 0.04 && grown < 0.06, "{grown}");
        assert!(frame.size.1 < rest.size.1, "it gets thinner");
        // Towards the key: the left end stays, the right one goes out.
        assert!((frame.rect.loc.x - rest.rect.loc.x).abs() < 0.5);
        let right = |f: &Frame| f.rect.loc.x + f.rect.size.w;
        assert!(right(&frame) - right(&rest) > 9.0);
        // The gauge, full, stretches with it.
        assert!(frame.look(1.0).fill > rest.look(1.0).fill);
        assert!(osd.stretch(ms(1600)).abs() < 1e-3);
    }

    #[test]
    fn a_key_that_insists_at_the_bottom_stretches_it_to_the_left() {
        let osd = pushed(Edge::Low, ms(1000));
        let rest = osd.frame(ms(999), screen(), 0.0).unwrap();
        let frame = osd
            .frame(ms(1000) + ui::STRETCH_HOLD, screen(), 0.0)
            .unwrap();
        let right = |f: &Frame| f.rect.loc.x + f.rect.size.w;
        assert!((right(&frame) - right(&rest)).abs() < 0.5);
        assert!(rest.rect.loc.x - frame.rect.loc.x > 6.0);
    }

    #[test]
    fn presses_that_follow_each_other_do_not_add_up() {
        let single = peak(&pushed(Edge::High, ms(1000)), ms(1000), 600);
        // The held key: a press every 40 ms for a second.
        let mut osd = pushed(Edge::High, ms(1000));
        let mut highest: f64 = 0.0;
        for press in 1..25 {
            let at = ms(1000 + press * 40);
            highest = highest.max(peak(&osd, at - ms(40), 40));
            osd.show(volume(100), Some(Edge::High), None, at);
        }
        // Never past the full stretch but for the spring's slight overshoot.
        assert!(highest < 1.05, "{highest} against {single} for one press");
        assert!(highest > single * 0.9, "it keeps stretching: {highest}");
        // Let go, it comes back.
        assert!(osd.stretch(ms(2000) + ms(700)).abs() < 1e-3);
    }

    #[test]
    fn the_gauge_slides_to_its_new_level() {
        let mut osd = Osd::default();
        osd.show(volume(40), None, None, ms(0));
        osd.show(volume(60), None, None, ms(500));
        let gauge = |t| osd.frame(ms(t), screen(), 0.0).unwrap().gauge;
        assert!((gauge(500) - 0.4).abs() < 1e-9, "no jump");
        let values: Vec<f64> = (500..800).map(gauge).collect();
        assert!(
            values.windows(2).all(|w| (w[1] - w[0]).abs() < 0.02),
            "small moves"
        );
        assert!((gauge(900) - 0.6).abs() < 1e-3);
        assert_eq!(osd.stretch(ms(600)), 0.0, "no stretch inside the range");
    }

    #[test]
    fn it_grows_in_and_shrinks_out_and_draws_nothing_at_rest() {
        let mut osd = Osd::default();
        osd.show(volume(50), None, None, ms(0));
        let first = osd.frame(ms(0), screen(), 0.0).unwrap();
        let shown = osd.frame(ui::ENTER, screen(), 0.0).unwrap();
        assert!((first.rect.size.w / shown.rect.size.w - ui::SCALE).abs() < 1e-6);
        assert_eq!((first.alpha, shown.alpha), (0.0, 1.0));
        // Around its center.
        let center = |f: &Frame| f.rect.loc.x + f.rect.size.w / 2.0;
        assert!((center(&first) - center(&shown)).abs() < 1e-6);
        assert!(osd.frame(ms(100), screen(), 0.0).unwrap().moving);
        // In place and still: drawn, but nothing moves, so no new frame is asked for.
        assert!(
            !osd.frame(ui::ENTER + ms(300), screen(), 0.0)
                .unwrap()
                .moving
        );
        osd.hide(ms(2000));
        let going = osd.frame(ms(2000) + ui::LEAVE / 2, screen(), 0.0).unwrap();
        assert!(going.moving && going.alpha < 1.0);
        assert!(going.rect.size.w < shown.rect.size.w);
        assert_eq!(osd.frame(ms(2000) + ui::LEAVE, screen(), 0.0), None);
    }

    #[test]
    fn it_sits_above_the_dock_in_the_middle_of_its_screen() {
        let mut osd = Osd::default();
        osd.show(volume(50), None, None, ms(0));
        let at = ui::ENTER;
        let on = |w: f64, h: f64, reserved: f64| {
            let screen = Rectangle::new((100.0, 0.0).into(), (w, h).into());
            osd.frame(at, screen, reserved).unwrap().rect
        };
        let rect = on(1920.0, 1080.0, 0.0);
        assert!((rect.loc.x + rect.size.w / 2.0 - 1060.0).abs() < 1e-6);
        assert!((1080.0 - rect.loc.y - rect.size.h - f64::from(ui::BOTTOM)).abs() < 1e-6);
        // A dock that reserves more pushes it up.
        let higher = on(1920.0, 1080.0, 150.0);
        assert!((rect.loc.y - higher.loc.y - (150.0 - f64::from(ui::DOCK_BAND))).abs() < 1e-6);
        // Bounded on a small and on a wide screen (the scale is an f32).
        assert!((on(1366.0, 768.0, 0.0).size.w - f64::from(ui::WIDTH_MIN)).abs() < 1e-3);
        assert!((on(5120.0, 2880.0, 0.0).size.w - f64::from(ui::WIDTH_MAX)).abs() < 1e-3);
    }
}
