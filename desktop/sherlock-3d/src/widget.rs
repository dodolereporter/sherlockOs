//! `Sherlock3d`: the mascot in an iced program.
//!
//! On the GPU, an `iced::widget::shader` whose program plays a mood (`Animator`) and whose
//! primitive draws it with `render::Renderer`, at 30 frames per second at most. It asks for
//! frames only while it is in the view and not paused: hidden or paused, it costs nothing. In
//! software (tiny-skia), or without the model file, it shows a still picture of Sherlock.
//!
//! The still picture is cut here, once, and not by iced: tiny-skia draws an image whole, its
//! `crop` and `border_radius` ignored, so the full picture overflowed the widget (lot Q-33). It
//! is the top square of the picture (cap, face and scarf, like the 3D framing), as a disc for
//! a round cut, and its drawing is clipped to the widget.
//!
//! ```ignore
//! let mascot = Sherlock3d::new(model.clone())
//!     .mood(Mood::Peek)
//!     .look_at(Some(Point::new(85.0, 300.0)))
//!     .width(size)
//!     .height(size);
//! ```

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use glam::{Mat4, Vec2};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self as advanced_renderer, Renderer as _};
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Clipboard, Shell, mouse};
use iced::widget::image as still;
use iced::widget::shader::{self, Viewport};
use iced::{Color, Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, window};

use crate::animator::{Animator, Mood};
use crate::model::Model;
use crate::render::{self, Camera, Cut, Frame};

/// At most 30 frames per second.
pub const FRAME: Duration = Duration::from_nanos(1_000_000_000 / 30);
/// Depth of the plane the gaze is aimed at, in front of the face (the model faces +Z).
const GAZE_DEPTH: f32 = 4.0;
/// The still picture of the design boards, shown in software.
static STILL_PNG: &[u8] = include_bytes!("../../../docs/design/images/sherlock-neutre.png");

/// The mascot, animated in a bubble.
#[derive(Debug, Clone)]
pub struct Sherlock3d {
    model: Option<Arc<Model>>,
    mood: Mood,
    gaze: Option<Point>,
    camera: Camera,
    cut: Cut,
    background: Option<Color>,
    paused: bool,
    width: Length,
    height: Length,
}

impl Sherlock3d {
    /// The mascot of `model`, or the still picture without one. Round, upper body, at rest.
    pub fn new(model: Option<Arc<Model>>) -> Self {
        Self {
            model,
            mood: Mood::Idle,
            gaze: None,
            camera: Camera::UPPER_BODY,
            cut: Cut::Circle { radius: 1.0 },
            background: None,
            paused: false,
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    pub fn mood(mut self, mood: Mood) -> Self {
        self.mood = mood;
        self
    }

    /// Where the eyes look, relative to the top left corner of the widget (it may lie
    /// outside, on the password field below for instance); `None` looks ahead.
    pub fn look_at(mut self, point: Option<Point>) -> Self {
        self.gaze = point;
        self
    }

    pub fn camera(mut self, camera: Camera) -> Self {
        self.camera = camera;
        self
    }

    /// The cut: a disc by default, the bubble.
    pub fn cut(mut self, cut: Cut) -> Self {
        self.cut = cut;
        self
    }

    /// Fills the cut behind the mascot.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Stops the animation on the frame shown, and the frames with it.
    pub fn paused(mut self, paused: bool) -> Self {
        self.paused = paused;
        self
    }

    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }
}

impl<'a, Message: 'a> From<Sherlock3d> for Element<'a, Message> {
    fn from(mascot: Sherlock3d) -> Self {
        // A disc stays whole and centered in any frame, as the 3D cut does; a rectangle is
        // covered, the overflow clipped by `Choose::draw`.
        let (handle, fit) = match mascot.cut {
            Cut::Circle { radius } => (still_disc(radius), iced::ContentFit::Contain),
            Cut::None => (still_picture().clone(), iced::ContentFit::Cover),
        };
        let fallback = still::Image::new(handle)
            .width(mascot.width)
            .height(mascot.height)
            .content_fit(fit);
        let scene = mascot.model.clone().map(|model| {
            Element::from(
                iced::widget::shader(Scene {
                    model,
                    mood: mascot.mood,
                    gaze: mascot.gaze,
                    camera: mascot.camera,
                    cut: mascot.cut,
                    background: mascot.background.map_or([0.0; 4], |c| [c.r, c.g, c.b, c.a]),
                    paused: mascot.paused,
                })
                .width(mascot.width)
                .height(mascot.height),
            )
        });
        Element::new(Choose {
            scene,
            fallback: fallback.into(),
            width: mascot.width,
            height: mascot.height,
        })
    }
}

/// The top square of the still picture, decoded once, straight-alpha RGBA.
fn still_square() -> &'static Option<image::RgbaImage> {
    static SQUARE: OnceLock<Option<image::RgbaImage>> = OnceLock::new();
    SQUARE.get_or_init(|| {
        match image::load_from_memory_with_format(STILL_PNG, image::ImageFormat::Png) {
            Ok(decoded) => Some(top_square(decoded.to_rgba8())),
            Err(error) => {
                tracing::warn!(%error, "cannot decode the still picture of Sherlock");
                None
            }
        }
    })
}

/// The top square of `picture`: as wide as it is, or as tall if it is wider than tall.
fn top_square(picture: image::RgbaImage) -> image::RgbaImage {
    let side = picture.width().min(picture.height());
    let x = (picture.width() - side) / 2;
    image::imageops::crop_imm(&picture, x, 0, side, side).to_image()
}

/// `square` cut to a disc centered in it, `radius` a share of half its side, antialiased.
fn disc(square: &image::RgbaImage, radius: f32) -> image::RgbaImage {
    let half = square.width() as f32 / 2.0;
    let radius = radius.clamp(0.0, 1.0) * half;
    let mut disc = square.clone();
    for (x, y, pixel) in disc.enumerate_pixels_mut() {
        let distance = (x as f32 + 0.5 - half).hypot(y as f32 + 0.5 - half);
        let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
        pixel[3] = (f32::from(pixel[3]) * coverage).round() as u8;
    }
    disc
}

fn handle_of(picture: &image::RgbaImage) -> still::Handle {
    still::Handle::from_rgba(picture.width(), picture.height(), picture.as_raw().clone())
}

/// A transparent pixel, when the picture cannot be read.
fn nothing() -> still::Handle {
    still::Handle::from_rgba(1, 1, vec![0; 4])
}

/// The still picture, cut to its top square: what a rectangle shows.
fn still_picture() -> &'static still::Handle {
    static PICTURE: OnceLock<still::Handle> = OnceLock::new();
    PICTURE.get_or_init(|| still_square().as_ref().map_or_else(nothing, handle_of))
}

/// The still picture as a disc of `radius`: each radius cut once, the handle shared.
fn still_disc(radius: f32) -> still::Handle {
    /// Radii kept at most: a program uses one or two.
    const KEPT: usize = 8;
    static DISCS: OnceLock<Mutex<HashMap<u32, still::Handle>>> = OnceLock::new();
    let Some(square) = still_square() else {
        return nothing();
    };
    let radius = if radius.is_finite() {
        radius.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut discs = DISCS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if discs.len() >= KEPT && !discs.contains_key(&radius.to_bits()) {
        discs.clear();
    }
    discs
        .entry(radius.to_bits())
        .or_insert_with(|| handle_of(&disc(square, radius)))
        .clone()
}

/// Draws the scene on the GPU, the still picture in software.
struct Choose<'a, Message> {
    scene: Option<Element<'a, Message>>,
    fallback: Element<'a, Message>,
    width: Length,
    height: Length,
}

impl<Message> Choose<'_, Message> {
    /// The child that draws with `renderer`, and its index among the children.
    fn pick(&self, renderer: &Renderer) -> (usize, &Element<'_, Message>) {
        let gpu = matches!(renderer, iced_renderer::fallback::Renderer::Primary(_));
        match &self.scene {
            Some(scene) if gpu => (1, scene),
            _ => (0, &self.fallback),
        }
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Choose<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        let mut children = vec![Tree::new(&self.fallback)];
        children.extend(self.scene.iter().map(Tree::new));
        children
    }

    fn diff(&self, tree: &mut Tree) {
        let mut elements = vec![&self.fallback];
        elements.extend(self.scene.iter());
        tree.diff_children(&elements);
    }

    fn size(&self) -> Size<Length> {
        Size::new(self.width, self.height)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let node = layout::atomic(limits, self.width, self.height);
        let size = node.size();
        let exact = layout::Limits::new(size, size);
        let mut children =
            vec![
                self.fallback
                    .as_widget_mut()
                    .layout(&mut tree.children[0], renderer, &exact),
            ];
        if let Some(scene) = &mut self.scene {
            children.push(
                scene
                    .as_widget_mut()
                    .layout(&mut tree.children[1], renderer, &exact),
            );
        }
        layout::Node::with_children(size, children)
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let (index, _) = self.pick(renderer);
        // Only the scene animates: in software, nothing asks for frames.
        if index == 1
            && let (Some(scene), Some(child)) = (&mut self.scene, layout.children().nth(1))
        {
            scene.as_widget_mut().update(
                &mut tree.children[1],
                event,
                child,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &advanced_renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let (index, element) = self.pick(renderer);
        let Some(child) = layout.children().nth(index) else {
            return;
        };
        let draw = |renderer: &mut Renderer| {
            element.as_widget().draw(
                &tree.children[index],
                renderer,
                theme,
                style,
                child,
                cursor,
                viewport,
            );
        };
        if index == 0 {
            // A covered picture is larger than the widget: only the widget's part shows.
            renderer.with_layer(layout.bounds(), draw);
        } else {
            draw(renderer);
        }
    }
}

/// The shader program: a mood played on the model.
#[derive(Debug, Clone)]
struct Scene {
    model: Arc<Model>,
    mood: Mood,
    gaze: Option<Point>,
    camera: Camera,
    cut: Cut,
    background: [f32; 4],
    paused: bool,
}

/// What the widget keeps between frames.
#[derive(Debug)]
pub struct State {
    id: u64,
    animator: Option<Animator>,
    last: Option<Instant>,
}

impl Default for State {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            animator: None,
            last: None,
        }
    }
}

impl<Message> shader::Program<Message> for Scene {
    type State = State;
    type Primitive = Primitive;

    fn update(
        &self,
        state: &mut State,
        event: &Event,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Option<shader::Action<Message>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else {
            return None;
        };
        let animator = state
            .animator
            .get_or_insert_with(|| Animator::new(self.model.clone()));
        animator.set_mood(self.mood);
        animator.look_at(self.gaze.map(|point| {
            let ndc = Vec2::new(
                point.x / bounds.width.max(1.0) * 2.0 - 1.0,
                1.0 - point.y / bounds.height.max(1.0) * 2.0,
            );
            let aspect = bounds.width / bounds.height.max(1.0);
            self.camera.unproject(ndc, aspect, GAZE_DEPTH)
        }));
        if self.paused {
            state.last = Some(*now);
            return None;
        }
        let dt = state.last.map_or(0.0, |last| {
            now.saturating_duration_since(last).as_secs_f32()
        });
        animator.advance(dt);
        state.last = Some(*now);
        Some(shader::Action::request_redraw_at(*now + FRAME))
    }

    fn draw(&self, state: &State, _cursor: mouse::Cursor, _bounds: Rectangle) -> Primitive {
        let joints = match &state.animator {
            Some(animator) => animator.joints(),
            None => Animator::new(self.model.clone()).joints(),
        };
        Primitive {
            id: state.id,
            model: self.model.clone(),
            joints,
            camera: self.camera,
            cut: self.cut,
            background: self.background,
        }
    }
}

/// One frame of one mascot.
#[derive(Debug)]
pub struct Primitive {
    id: u64,
    model: Arc<Model>,
    joints: Vec<Mat4>,
    camera: Camera,
    cut: Cut,
    background: [f32; 4],
}

/// The GPU side of every mascot of a window.
pub struct Pipeline {
    format: wgpu::TextureFormat,
    renderers: HashMap<u64, Area>,
    used: HashSet<u64>,
}

struct Area {
    renderer: render::Renderer,
    /// Where the mascot lies in the target, in physical pixels.
    bounds: Rectangle<u32>,
}

impl shader::Pipeline for Pipeline {
    fn new(_device: &wgpu::Device, _queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            renderers: HashMap::new(),
            used: HashSet::new(),
        }
    }

    /// Drops the mascots no longer drawn: a hidden one keeps nothing on the GPU.
    fn trim(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.renderers.retain(|id, _| used.contains(id));
    }
}

impl shader::Primitive for Primitive {
    type Pipeline = Pipeline;

    fn prepare(
        &self,
        pipeline: &mut Pipeline,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let scale = viewport.scale_factor();
        let physical = Rectangle {
            x: (bounds.x * scale).round().max(0.0) as u32,
            y: (bounds.y * scale).round().max(0.0) as u32,
            width: (bounds.width * scale).round().max(1.0) as u32,
            height: (bounds.height * scale).round().max(1.0) as u32,
        };
        let format = pipeline.format;
        let area = pipeline.renderers.entry(self.id).or_insert_with(|| Area {
            renderer: render::Renderer::new(device, queue, &self.model, format),
            bounds: physical,
        });
        area.bounds = physical;
        area.renderer.prepare(
            device,
            queue,
            &Frame {
                size: [physical.width, physical.height],
                joints: &self.joints,
                camera: self.camera,
                cut: self.cut,
                background: self.background,
            },
            [bounds.x * scale, bounds.y * scale],
        );
        pipeline.used.insert(self.id);
    }

    fn render(
        &self,
        pipeline: &Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip_bounds: &Rectangle<u32>,
    ) {
        let Some(area) = pipeline.renderers.get(&self.id) else {
            return;
        };
        let Some(scissor) = intersection(&area.bounds, clip_bounds) else {
            return;
        };
        area.renderer.draw_scene(encoder);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sherlock-3d widget"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        area.renderer.composite(&mut pass);
    }
}

/// The common part of two rectangles, if not empty.
fn intersection(a: &Rectangle<u32>, b: &Rectangle<u32>) -> Option<Rectangle<u32>> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then(|| Rectangle {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_still_picture_is_its_top_square() {
        let mut tall = image::RgbaImage::from_pixel(4, 10, image::Rgba([0, 0, 0, 255]));
        tall.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let square = top_square(tall);
        assert_eq!(square.dimensions(), (4, 4));
        assert_eq!(square.get_pixel(0, 0)[0], 255, "the top is kept");
        let wide = image::RgbaImage::new(10, 4);
        assert_eq!(top_square(wide).dimensions(), (4, 4));
        let picture = still_square().as_ref().map(image::RgbaImage::dimensions);
        assert!(picture.is_some_and(|(w, h)| w == h && w > 1), "{picture:?}");
    }

    #[test]
    fn the_disc_is_transparent_outside_its_radius() {
        let square = image::RgbaImage::from_pixel(100, 100, image::Rgba([10, 20, 30, 255]));
        let full = disc(&square, 1.0);
        assert_eq!(full.get_pixel(0, 0)[3], 0, "corner");
        assert_eq!(full.get_pixel(99, 99)[3], 0, "corner");
        assert_eq!(full.get_pixel(50, 50)[3], 255, "center");
        assert_eq!(full.get_pixel(50, 1)[3], 255, "edge, inside");
        assert_eq!(full.get_pixel(50, 50).0[..3], [10, 20, 30]);
        let half = disc(&square, 0.5);
        assert_eq!(half.get_pixel(50, 10)[3], 0, "outside half the radius");
        // Out of range: clamped, never a panic.
        assert_eq!(disc(&square, f32::NAN).get_pixel(0, 0)[3], 0);
        assert_eq!(disc(&square, 3.0).get_pixel(0, 0)[3], 0);
    }
}
