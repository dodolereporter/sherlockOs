//! Cursor of the real session, drawn by the compositor in the frame (in nested mode, the host
//! window's cursor is used instead). Lot D-07, after the author's cursor boards
//! (`docs/design/cursors-dark.html`, `docs/design/cursors-light.html`).
//!
//! - **Theme**: the house theme, generated from `assets/cursors/` and installed by
//!   `just install-session`: `sherlockos`, or `sherlockos-light` while the desktop is light, followed
//!   live (lot D-05c). Without it, the user's theme: `XCURSOR_THEME`, else the system's
//!   `default`. A shape missing from the theme shows the arrow.
//! - **Size**: `XCURSOR_SIZE` (24 by default) times the screen's scale. The image of the nearest
//!   size above is shown pixel for pixel, never stretched: sharp at a fractional scale too.
//! - **Motion**: the waiting cursors turn, a turn per second, each image shown at its time by a
//!   timer; a new shape fades in with a bounce; shaking the pointer makes it larger, until a
//!   second after the last shake. At rest, nothing asks for a frame.

use std::{
    cell::Cell,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use sherlock_ui::{
    color::Theme,
    cursor::{
        SHAKE_HOLD, SHAKE_SCALE, SHAKE_STEP, SHAKE_TRANSITION, SHAKE_TURNS, SHAKE_WINDOW,
        SHAPE_CHANGE, SHAPE_FROM_OPACITY, SHAPE_FROM_SCALE, bounce,
    },
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            },
            gles::GlesRenderer,
        },
    },
    input::pointer::CursorIcon,
    reexports::calloop::{
        LoopHandle,
        timer::{TimeoutAction, Timer},
    },
    utils::{Physical, Point, Rectangle, Size, Transform},
};

use crate::State;

/// Size of the cursor at scale 1, in pixels, when `XCURSOR_SIZE` does not set it.
const DEFAULT_SIZE: u32 = 24;

/// What the cursor needs to go on moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Nothing: it is still.
    Rest,
    /// A frame at the next vertical blank: a transition runs.
    Frame,
    /// A frame at this time of the compositor's clock.
    At(Duration),
}

impl Next {
    /// What both need: the sooner.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Frame, _) | (_, Self::Frame) => Self::Frame,
            (Self::At(a), Self::At(b)) => Self::At(a.min(b)),
            (Self::At(a), Self::Rest) | (Self::Rest, Self::At(a)) => Self::At(a),
            (Self::Rest, Self::Rest) => Self::Rest,
        }
    }
}

/// Where the cursor files are read.
enum Source {
    /// The `cursors` folder of the house theme.
    House(PathBuf),
    /// The user's theme, or the system's.
    System(xcursor::CursorTheme),
}

impl Source {
    /// The house theme of `theme`, if it is installed; else that of the dark desktop; else the
    /// user's theme.
    fn load(theme: Theme) -> Self {
        let first = house_theme(theme);
        let houses = std::iter::once(first)
            .chain((first != cursor_theme::THEME).then_some(cursor_theme::THEME));
        Self::from_house(houses.into_iter().find_map(|name| {
            // Depth 0: the theme itself has the arrow, not one it inherits from.
            let (arrow, depth) =
                xcursor::CursorTheme::load(name).load_icon_with_depth("default")?;
            (depth == 0).then(|| arrow.parent().map(Path::to_path_buf))?
        }))
    }

    fn from_house(house: Option<PathBuf>) -> Self {
        match house {
            Some(dir) => {
                tracing::info!(theme = ?dir, "cursor theme");
                Self::House(dir)
            }
            None => {
                let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
                tracing::info!(
                    theme = name,
                    "house cursor theme missing, user cursor theme"
                );
                Self::System(xcursor::CursorTheme::load(&name))
            }
        }
    }

    /// The file of `shape`, by its CSS name or one of its legacy names.
    fn find(&self, shape: Shape) -> Option<PathBuf> {
        let names: Vec<&str> = match shape {
            Shape::Icon(icon) => std::iter::once(icon.name())
                .chain(icon.alt_names().iter().copied())
                .collect(),
            Shape::Sherlock => vec![SHERLOCK],
        };
        let mut names = names.into_iter();
        match self {
            Self::House(dir) => names.map(|n| dir.join(n)).find(|p| p.is_file()),
            Self::System(theme) => names.find_map(|n| theme.load_icon(n)),
        }
    }
}

/// A shape the compositor shows: a CSS name of `wp_cursor_shape_v1`, or the Sherlock cursor,
/// which no client names (Super + click, lot C-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    Icon(CursorIcon),
    Sherlock,
}

/// The file of the Sherlock cursor in the house themes (`cursor_theme::SHAPES`).
const SHERLOCK: &str = "sherlock";

/// The house theme of the desktop's `theme`. At startup, it is the one the session names in
/// `XCURSOR_THEME` for every program (`sherlockos-session`), read from the same appearance
/// file; after a change of theme, the compositor follows it and XWayland keeps the first one
/// (see "Cursors" in `docs/cursor.md`).
fn house_theme(theme: Theme) -> &'static str {
    match theme {
        Theme::Light => cursor_theme::LIGHT_THEME,
        Theme::Dark => cursor_theme::THEME,
    }
}

/// One image of a cursor file.
struct Frame {
    buffer: MemoryRenderBuffer,
    hotspot: Point<f64, Physical>,
    size: Size<i32, Physical>,
    delay: Duration,
}

/// The images of a cursor at one size: one, or a turn of them.
struct Image {
    nominal: u32,
    frames: Vec<Frame>,
    turn: Duration,
}

impl Image {
    /// The image shown at `now`, and when the next one is due.
    fn frame(&self, now: Duration) -> Option<(&Frame, Next)> {
        let first = self.frames.first()?;
        if self.turn.is_zero() {
            return Some((first, Next::Rest));
        }
        let mut t = Duration::from_nanos((now.as_nanos() % self.turn.as_nanos()) as u64);
        for frame in &self.frames {
            if t < frame.delay {
                return Some((frame, Next::At(now + (frame.delay - t))));
            }
            t -= frame.delay;
        }
        Some((first, Next::Rest))
    }
}

/// The nominal size to show for `target` pixels: the smallest at least as large, else the
/// largest.
fn pick_size(sizes: impl Iterator<Item = u32> + Clone, target: u32) -> Option<u32> {
    sizes.clone().filter(|&s| s >= target).min().or(sizes.max())
}

/// Reads the images of the file at `path` at the size nearest above `target`.
fn read(path: &Path, target: u32) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    let images = xcursor::parser::parse_xcursor(&bytes)?;
    let nominal = pick_size(images.iter().map(|i| i.size), target)?;
    let frames: Vec<Frame> = images
        .iter()
        .filter(|i| i.size == nominal)
        .map(|i| Frame {
            // Xcursor pixels are premultiplied ARGB words in the machine's order, as `Argb8888`.
            buffer: MemoryRenderBuffer::from_slice(
                &i.pixels_rgba,
                Fourcc::Argb8888,
                (i.width as i32, i.height as i32),
                1,
                Transform::Normal,
                None,
            ),
            hotspot: (f64::from(i.xhot), f64::from(i.yhot)).into(),
            size: (i.width as i32, i.height as i32).into(),
            delay: Duration::from_millis(u64::from(i.delay)),
        })
        .collect();
    // One image is still, whatever its delay says.
    let turn = match frames.len() {
        0 | 1 => Duration::ZERO,
        _ => frames.iter().map(|f| f.delay).sum(),
    };
    (!frames.is_empty()).then_some(Image {
        nominal,
        frames,
        turn,
    })
}

/// Shake to find: changes of direction of the pointer, and the growth they started.
#[derive(Default)]
struct Shake {
    trace: VecDeque<(Duration, f64)>,
    /// When the growth started, while the cursor is larger.
    grown: Option<Duration>,
    last: Duration,
}

impl Shake {
    /// The pointer is at `x` (logical pixels) at `now`.
    fn feed(&mut self, x: f64, now: Duration) {
        while self
            .trace
            .front()
            .is_some_and(|&(t, _)| now.saturating_sub(t) >= SHAKE_WINDOW)
        {
            self.trace.pop_front();
        }
        if self.trace.back().is_some_and(|&(_, last)| last == x) {
            return;
        }
        self.trace.push_back((now, x));
        let mut turns = 0;
        let mut direction = 0.0;
        for (a, b) in self.trace.iter().zip(self.trace.iter().skip(1)) {
            let d = b.1 - a.1;
            if d.abs() < SHAKE_STEP {
                continue;
            }
            if direction != 0.0 && d.signum() != direction {
                turns += 1;
            }
            direction = d.signum();
        }
        if turns >= SHAKE_TURNS {
            self.trace.clear();
            self.grown.get_or_insert(now);
            self.last = now;
        }
    }

    /// How much larger the cursor is at `now`, and what it needs next.
    fn factor(&mut self, now: Duration) -> (f32, Next) {
        let Some(grown) = self.grown else {
            return (1.0, Next::Rest);
        };
        let progress = |since: Duration| {
            now.saturating_sub(since).as_secs_f32() / SHAKE_TRANSITION.as_secs_f32()
        };
        let shrink = (self.last + SHAKE_HOLD).max(grown + SHAKE_TRANSITION);
        if now < shrink {
            return match progress(grown) {
                p if p < 1.0 => (1.0 + (SHAKE_SCALE - 1.0) * bounce(p), Next::Frame),
                _ => (SHAKE_SCALE, Next::At(shrink)),
            };
        }
        match progress(shrink) {
            p if p < 1.0 => (SHAKE_SCALE + (1.0 - SHAKE_SCALE) * bounce(p), Next::Frame),
            _ => {
                self.grown = None;
                (1.0, Next::Rest)
            }
        }
    }
}

/// How the cursor is drawn at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    /// The growth of a shake.
    shake: f32,
    /// The scale of a new shape, while it fades in.
    change: f32,
    alpha: f32,
    next: Next,
}

/// The theme, the images already read, and the state of the cursor's motion.
pub struct Cursors {
    /// The desktop's theme, whose house cursors `source` reads.
    theme: Theme,
    source: Source,
    size: u32,
    images: HashMap<(Shape, u32), Option<Rc<Image>>>,
    /// The shape shown, since when.
    shown: Option<(Shape, Duration)>,
    shake: Shake,
    /// The time the wake-up timer is set for, if one is.
    wake: Rc<Cell<Option<Duration>>>,
}

impl Cursors {
    /// The cursors of the desktop's `theme`.
    pub fn load(theme: Theme) -> Self {
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|s| (1..=256).contains(s))
            .unwrap_or(DEFAULT_SIZE);
        tracing::info!(size, "cursor size");
        Self::new(theme, Source::load(theme), size)
    }

    fn new(theme: Theme, source: Source, size: u32) -> Self {
        Self {
            theme,
            source,
            size,
            images: HashMap::new(),
            shown: None,
            shake: Shake::default(),
            wake: Rc::default(),
        }
    }

    /// Follows the desktop's `theme`: on a change, the house cursors of the new one, read again
    /// from their files. The shape shown and a shake go on.
    pub fn follow(&mut self, theme: Theme) {
        if theme != self.theme {
            self.theme = theme;
            self.source = Source::load(theme);
            self.images.clear();
        }
    }

    /// The images of `icon` for `target` pixels, the arrow's if the theme has none.
    fn image(&mut self, icon: Shape, target: u32) -> Option<Rc<Image>> {
        let source = &self.source;
        self.images
            .entry((icon, target))
            .or_insert_with(|| {
                let image = [icon, Shape::Icon(CursorIcon::Default)]
                    .into_iter()
                    .find_map(|i| read(&source.find(i)?, target));
                if image.is_none() {
                    tracing::warn!(?icon, "no cursor image in the theme");
                }
                image.map(Rc::new)
            })
            .clone()
    }

    /// The motion at `now` of `icon`, the pointer being at `x` (logical pixels).
    fn look(&mut self, icon: Shape, x: f64, now: Duration) -> Look {
        if self.shown.is_none_or(|(shown, _)| shown != icon) {
            self.shown = Some((icon, now));
        }
        self.shake.feed(x, now);
        let (shake, next) = self.shake.factor(now);
        let since = self
            .shown
            .map_or(SHAPE_CHANGE, |(_, t)| now.saturating_sub(t));
        let p = since.as_secs_f32() / SHAPE_CHANGE.as_secs_f32();
        if p >= 1.0 {
            return Look {
                shake,
                change: 1.0,
                alpha: 1.0,
                next,
            };
        }
        let e = bounce(p);
        Look {
            shake,
            change: SHAPE_FROM_SCALE + (1.0 - SHAPE_FROM_SCALE) * e,
            alpha: (SHAPE_FROM_OPACITY + (1.0 - SHAPE_FROM_OPACITY) * e).min(1.0),
            next: next.and(Next::Frame),
        }
    }

    /// The cursor `icon` with its tip at `at`, in frame pixels, on a screen at `scale`, at
    /// `now`, `press` times its size (it sinks at a click shown, lot C-01); and what it needs
    /// next. `None` if the theme has no image for it, not even the arrow.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        icon: Shape,
        at: Point<f64, Physical>,
        (scale, press): (f64, f32),
        now: Duration,
    ) -> Option<(MemoryRenderBufferRenderElement<GlesRenderer>, Next)> {
        let look = self.look(icon, at.x / scale, now);
        let base = (f64::from(self.size) * scale).round().max(1.0) as u32;
        // While shaken, the image of the largest size, scaled down: sharp at its peak.
        let grown = self.shake.grown.is_some();
        let target = match grown {
            true => (base as f32 * SHAKE_SCALE).round() as u32,
            false => base,
        };
        let image = self.image(icon, target)?;
        let (frame, next) = image.frame(now)?;
        let ratio = match grown {
            true => f64::from(base) * f64::from(look.shake) / f64::from(image.nominal),
            false => 1.0,
        } * f64::from(look.change)
            * f64::from(press);
        let size = frame.size.to_f64().upscale(ratio).to_i32_round();
        let element = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            at - frame.hotspot.upscale(ratio),
            &frame.buffer,
            (look.alpha < 1.0).then_some(look.alpha),
            Some(Rectangle::from_size(frame.size.to_f64().to_logical(1.0))),
            Some(size.to_logical(1)),
            Kind::Cursor,
        );
        let element = element
            .inspect_err(|err| tracing::warn!("cursor image: {err}"))
            .ok()?;
        Some((element, look.next.and(next)))
    }

    /// A frame at `at` (the compositor's clock, now being `now`), for a still cursor that will
    /// move then: one timer at a time, the soonest.
    pub fn wake_at(&self, handle: &LoopHandle<'static, State>, at: Duration, now: Duration) {
        if self.wake.get().is_some_and(|set| set <= at) {
            return;
        }
        self.wake.set(Some(at));
        let wake = self.wake.clone();
        let timer = Timer::from_duration(at.saturating_sub(now));
        let inserted = handle.insert_source(timer, move |_, _, state| {
            if wake.get() == Some(at) {
                wake.set(None);
            }
            state.queue_redraw();
            TimeoutAction::Drop
        });
        if let Err(err) = inserted {
            self.wake.set(None);
            tracing::warn!("cursor timer: {}", err.error);
        }
    }
}

#[cfg(test)]
mod tests {
    use cursor_theme::xcursor::{Image as XImage, encode};

    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// A house theme in a fresh folder: every shape with an image per size of `sizes`,
    /// `frames` images each, 40 ms apart, and every alias as a link.
    fn theme(test: &str, sizes: &[u32], frames: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cursor-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for shape in cursor_theme::SHAPES {
            let images: Vec<XImage> = sizes
                .iter()
                .flat_map(|&size| {
                    (0..frames).map(move |f| XImage {
                        size,
                        width: size,
                        height: size,
                        hotspot: (size / 4, f),
                        delay: 40,
                        pixels: vec![0xFF00_0000; (size * size) as usize],
                    })
                })
                .collect();
            std::fs::write(dir.join(shape.name), encode(&images)).unwrap();
            for alias in shape.aliases {
                std::os::unix::fs::symlink(shape.name, dir.join(alias)).unwrap();
            }
        }
        dir
    }

    const ICONS: [CursorIcon; 36] = [
        CursorIcon::Default,
        CursorIcon::ContextMenu,
        CursorIcon::Help,
        CursorIcon::Pointer,
        CursorIcon::Progress,
        CursorIcon::Wait,
        CursorIcon::Cell,
        CursorIcon::Crosshair,
        CursorIcon::Text,
        CursorIcon::VerticalText,
        CursorIcon::Alias,
        CursorIcon::Copy,
        CursorIcon::Move,
        CursorIcon::NoDrop,
        CursorIcon::NotAllowed,
        CursorIcon::Grab,
        CursorIcon::Grabbing,
        CursorIcon::EResize,
        CursorIcon::NResize,
        CursorIcon::NeResize,
        CursorIcon::NwResize,
        CursorIcon::SResize,
        CursorIcon::SeResize,
        CursorIcon::SwResize,
        CursorIcon::WResize,
        CursorIcon::EwResize,
        CursorIcon::NsResize,
        CursorIcon::NeswResize,
        CursorIcon::NwseResize,
        CursorIcon::ColResize,
        CursorIcon::RowResize,
        CursorIcon::AllScroll,
        CursorIcon::ZoomIn,
        CursorIcon::ZoomOut,
        CursorIcon::DndAsk,
        CursorIcon::AllResize,
    ];

    #[test]
    fn every_shape_a_client_names_finds_its_own_drawing() {
        let dir = theme("names", &[24], 1);
        let source = Source::House(dir.clone());
        for icon in ICONS {
            let path = source
                .find(Shape::Icon(icon))
                .unwrap_or_else(|| panic!("{icon:?}"));
            // The file reached through the link is the shape that lists this CSS name.
            let shape = cursor_theme::shape(icon.name()).unwrap_or_else(|| panic!("{icon:?}"));
            assert_eq!(
                path.canonicalize().unwrap(),
                dir.join(shape.name).canonicalize().unwrap(),
                "{icon:?}"
            );
        }
    }

    /// The shapes the components of the shell ask for (lot UI-01), and the Sherlock cursor of
    /// Super + click, show their drawings: the hand that points, the closed hand, the I-beam,
    /// the forbidden sign.
    #[test]
    fn the_shapes_of_the_shell_find_their_drawings() {
        let dir = theme("shell", &[24], 1);
        let source = Source::House(dir.clone());
        let drawing = |shape: Shape| {
            let path = source.find(shape).unwrap_or_else(|| panic!("{shape:?}"));
            let path = path.canonicalize().unwrap();
            path.file_name().unwrap().to_string_lossy().into_owned()
        };
        assert_eq!(drawing(Shape::Icon(CursorIcon::Pointer)), "pointer");
        assert_eq!(drawing(Shape::Icon(CursorIcon::Grab)), "grabbing");
        assert_eq!(drawing(Shape::Icon(CursorIcon::Grabbing)), "grabbing");
        assert_eq!(drawing(Shape::Icon(CursorIcon::Text)), "text");
        assert_eq!(drawing(Shape::Icon(CursorIcon::NotAllowed)), "not-allowed");
        assert_eq!(drawing(Shape::Icon(CursorIcon::Move)), "move");
        assert_eq!(drawing(Shape::Icon(CursorIcon::NwseResize)), "nwse-resize");
        assert_eq!(drawing(Shape::Sherlock), "sherlock");
    }

    #[test]
    fn the_nearest_size_above_is_shown() {
        let sizes = [24, 32, 48, 64, 96].into_iter();
        assert_eq!(pick_size(sizes.clone(), 24), Some(24));
        // Scale 1.25: 30 pixels, the image of 32, not a blurred 24.
        assert_eq!(pick_size(sizes.clone(), 30), Some(32));
        assert_eq!(pick_size(sizes.clone(), 36), Some(48));
        assert_eq!(pick_size(sizes.clone(), 200), Some(96));
        assert_eq!(pick_size(std::iter::empty(), 24), None);

        let dir = theme("sizes", &[24, 32, 48], 1);
        let mut cursors = Cursors::new(Theme::Dark, Source::House(dir), 24);
        let image = cursors.image(Shape::Icon(CursorIcon::Text), 30).unwrap();
        assert_eq!(image.nominal, 32);
        assert_eq!(image.frames[0].hotspot, (8.0, 0.0).into());
        assert_eq!(image.frames[0].size, (32, 32).into());
    }

    #[test]
    fn a_missing_theme_or_shape_falls_back() {
        // No house theme: the user's theme takes over.
        assert!(matches!(Source::from_house(None), Source::System(_)));
        // The text cursor is missing from the house theme: the arrow.
        let dir = theme("fallback", &[24], 1);
        std::fs::remove_file(dir.join("text")).unwrap();
        let mut cursors = Cursors::new(Theme::Dark, Source::House(dir), 24);
        assert!(cursors.image(Shape::Icon(CursorIcon::Text), 24).is_some());
        // Nothing at all: no cursor, rather than a wrong one.
        let empty = std::env::temp_dir().join(format!("cursor-empty-{}", std::process::id()));
        let mut none = Cursors::new(Theme::Dark, Source::House(empty), 24);
        assert!(none.image(Shape::Icon(CursorIcon::Default), 24).is_none());
    }

    #[test]
    fn the_waiting_cursor_turns_at_the_pace_of_its_images() {
        let dir = theme("turn", &[24], 3);
        let mut cursors = Cursors::new(Theme::Dark, Source::House(dir), 24);
        let image = cursors.image(Shape::Icon(CursorIcon::Wait), 24).unwrap();
        assert_eq!(image.turn, 120 * MS);
        let (frame, next) = image.frame(50 * MS).unwrap();
        assert_eq!(frame.hotspot.y, 1.0);
        assert_eq!(next, Next::At(80 * MS));
        // A turn later, the same image.
        assert_eq!(image.frame(170 * MS).unwrap().0.hotspot.y, 1.0);
        // A still cursor needs nothing.
        let still = theme("still", &[24], 1);
        let mut cursors = Cursors::new(Theme::Dark, Source::House(still), 24);
        let image = cursors.image(Shape::Icon(CursorIcon::Default), 24).unwrap();
        assert_eq!(image.frame(50 * MS).unwrap().1, Next::Rest);
    }

    #[test]
    fn a_new_shape_fades_in_then_rests() {
        let mut cursors = Cursors::new(Theme::Dark, Source::House(PathBuf::new()), 24);
        let first = cursors.look(Shape::Icon(CursorIcon::Default), 0.0, Duration::ZERO);
        assert!((first.change - SHAPE_FROM_SCALE).abs() < 1e-3);
        assert!((first.alpha - SHAPE_FROM_OPACITY).abs() < 1e-3);
        assert_eq!(first.next, Next::Frame);
        let rest = cursors.look(Shape::Icon(CursorIcon::Default), 0.0, SHAPE_CHANGE);
        assert_eq!((rest.change, rest.alpha, rest.next), (1.0, 1.0, Next::Rest));
        // Another shape: another fade.
        let pointer = cursors.look(
            Shape::Icon(CursorIcon::Pointer),
            0.0,
            SHAPE_CHANGE + 10 * MS,
        );
        assert_eq!(pointer.next, Next::Frame);
    }

    #[test]
    fn shaking_the_pointer_makes_it_larger_for_a_while() {
        let mut shake = Shake::default();
        // A slow move one way: nothing.
        for i in 0..10 {
            shake.feed(f64::from(i) * 10.0, i * 30 * MS);
        }
        assert_eq!(shake.factor(300 * MS), (1.0, Next::Rest));
        // Later, four changes of direction within the window.
        let start = 2000 * MS;
        for i in 0..6 {
            let x = if i % 2 == 0 { 0.0 } else { 60.0 };
            shake.feed(x, start + i * 40 * MS);
        }
        let shaken = start + 200 * MS;
        assert_eq!(shake.grown, Some(shaken));
        assert_eq!(shake.factor(shaken).1, Next::Frame);
        // Grown, held until a second after the last shake: no frame meanwhile.
        let shrink = shaken + SHAKE_HOLD;
        assert_eq!(
            shake.factor(shaken + SHAKE_TRANSITION),
            (SHAKE_SCALE, Next::At(shrink))
        );
        // Then back, with a bounce, and at rest.
        assert_eq!(shake.factor(shrink + 100 * MS).1, Next::Frame);
        assert_eq!(shake.factor(shrink + SHAKE_TRANSITION), (1.0, Next::Rest));
        assert_eq!(shake.grown, None);
    }

    #[test]
    fn the_house_theme_follows_the_desktop() {
        assert_eq!(house_theme(Theme::Dark), cursor_theme::THEME);
        assert_eq!(house_theme(Theme::Light), cursor_theme::LIGHT_THEME);
        let dir = theme("follow", &[24], 1);
        let mut cursors = Cursors::new(Theme::Dark, Source::House(dir), 24);
        assert!(
            cursors
                .image(Shape::Icon(CursorIcon::Default), 24)
                .is_some()
        );
        // The same theme: the images read stay.
        cursors.follow(Theme::Dark);
        assert_eq!(cursors.images.len(), 1);
        // The other one: its cursors, read again.
        cursors.follow(Theme::Light);
        assert_eq!(cursors.theme, Theme::Light);
        assert!(cursors.images.is_empty());
    }

    #[test]
    fn the_sooner_need_wins() {
        assert_eq!(Next::Rest.and(Next::At(MS)), Next::At(MS));
        assert_eq!(Next::At(2 * MS).and(Next::At(MS)), Next::At(MS));
        assert_eq!(Next::At(MS).and(Next::Frame), Next::Frame);
        assert_eq!(Next::Rest.and(Next::Rest), Next::Rest);
    }
}
