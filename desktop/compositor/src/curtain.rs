//! The curtain (lot D-24e): the picture of the lock screen (`desktop_config::lock_background`,
//! the one the lock and login screens draw), drawn by the compositor itself, so that the login,
//! the session and the lock follow each other on one picture, without a plain color nor a jump.
//!
//! - **Login screen** (`--greeter`): the curtain is the background from the first frame, behind
//!   the greeter. When the greeter has left, it fades to black (`CURTAIN_BLACK`), then the
//!   compositor stops and greetd starts the session.
//! - **Session**: from its first frame, the curtain comes back from black, then fades out over
//!   the desktop (`CURTAIN_REVEAL`); the bar and the dock arrive once it is half gone
//!   (`arrival.rs`).
//! - **Lock** (ADR 0023, amended): the last frame of each screen is kept as it was, frozen
//!   (`Freezer`), and the curtain comes down over it in `CURTAIN`. Nothing of the desktop is
//!   drawn but that image, which is dropped once the curtain is down; `locked` is sent only
//!   after a frame with the curtain down. Then the lock surface shows over the curtain, once the
//!   lock program has drawn it; until then, the curtain alone.
//! - **Unlock**: the curtain goes up off the live desktop, which is drawn again only once the
//!   unlock was accepted.
//!
//! The pictures are read once, at start (`State::load_curtain`), for the largest screen.
//! Without them, the plain background of the system screens takes their place.

use std::{ffi::OsString, sync::Arc, time::Duration};

use desktop_config::lock_background::{self, Pictures};
use sherlock_ui::{
    color,
    lock::{self as ui, CURTAIN, CURTAIN_BLACK, CURTAIN_FADE_EASE, CURTAIN_REVEAL},
    screen_off,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem as _,
            element::texture::TextureBuffer,
            gles::{GlesRenderer, GlesTexture},
        },
    },
    output::Output,
    utils::{Rectangle, Transform},
};

use crate::{
    State,
    damage::{Key, Part},
    output::View,
    render::Item,
    wallpaper,
};

/// Draws the screen as it is into a texture: given by the backend, which has the renderer.
pub type Freezer = Box<dyn FnMut(&mut State, &Output) -> Option<GlesTexture>>;

/// Where the curtain stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// The session opens: from black to the picture, then the picture fades out, from the
    /// first frame (`None` until it is drawn).
    Opening(Option<Duration>),
    /// Up: nothing drawn.
    Up,
    /// Coming down over the frozen desktop since then: the session locks.
    Lowering(Duration),
    /// Down: behind the lock surfaces, or the login screen.
    Down,
    /// Going up off the desktop since then: unlocked.
    Raising(Duration),
    /// The login screen leaves: fading to black since then.
    Closing(Duration),
}

/// The curtain, held by the state.
pub struct Curtain {
    pictures: Option<Arc<Pictures>>,
    /// Changes with the pictures: the scene uploads them again.
    generation: u64,
    motion: Motion,
    /// The last frame of each screen when the session locked, while the curtain comes down.
    frozen: Vec<(Output, GlesTexture)>,
    freezer: Option<Freezer>,
    /// The pictures were looked for.
    tried: bool,
}

impl Default for Curtain {
    fn default() -> Self {
        Self {
            pictures: None,
            generation: 0,
            motion: Motion::Opening(None),
            frozen: Vec::new(),
            freezer: None,
            tried: false,
        }
    }
}

/// How far a timed step has gone `elapsed` into `length`, from 0 to 1.
fn share(elapsed: Duration, length: Duration) -> f32 {
    (elapsed.as_secs_f32() / length.as_secs_f32().max(f32::EPSILON)).clamp(0.0, 1.0)
}

impl State {
    /// Reads the pictures of the curtain for screens of at most `pixels`, as the lock screen
    /// does (`var` reads the environment).
    pub fn load_curtain(&mut self, var: impl Fn(&str) -> Option<OsString>, pixels: (u32, u32)) {
        let Some(source) = lock_background::source(var) else {
            tracing::warn!("curtain: no picture of the lock, the plain background takes its place");
            return;
        };
        match lock_background::pictures(&source, pixels) {
            Ok(pictures) => self.set_curtain_pictures(pictures),
            Err(err) => tracing::warn!(?source, "curtain: {err}"),
        }
    }

    /// Reads the pictures once the screens are known, before their first frame: for the
    /// largest one, from the environment of the compositor.
    pub(crate) fn ensure_curtain(&mut self) {
        let pixels = self.views().iter().fold((0, 0), |(w, h), (_, view)| {
            let size = view.frame.size;
            (w.max(size.w.max(0) as u32), h.max(size.h.max(0) as u32))
        });
        if self.curtain.tried || pixels.0 == 0 || pixels.1 == 0 {
            return;
        }
        self.curtain.tried = true;
        self.load_curtain(|key| std::env::var_os(key), pixels);
    }

    pub fn set_curtain_pictures(&mut self, pictures: Pictures) {
        self.curtain.pictures = Some(Arc::new(pictures));
        self.curtain.generation += 1;
        self.queue_redraw();
    }

    /// The pictures and their generation, for the scene to upload.
    pub(crate) fn curtain_pictures(&self) -> Option<(u64, &Pictures)> {
        let pictures = self.curtain.pictures.as_deref()?;
        Some((self.curtain.generation, pictures))
    }

    /// Plugs in the backend's way to draw a screen into a texture.
    pub fn set_freezer(&mut self, freezer: Freezer) {
        self.curtain.freezer = Some(freezer);
    }

    pub fn curtain_motion(&self) -> Motion {
        self.curtain.motion
    }

    /// A frame is drawn at `now`: the opening of the session starts with the first one. Without
    /// the pictures (none could be read, or the tests), there is no opening.
    pub fn curtain_frame(&mut self, now: Duration) {
        if self.curtain.motion == Motion::Opening(None) {
            self.curtain.motion = match (self.greeter, self.curtain.pictures.is_some()) {
                (true, _) => Motion::Down,
                (false, true) => Motion::Opening(Some(now)),
                (false, false) => Motion::Up,
            };
        }
    }

    /// When the bar and the dock may start to arrive: once the curtain of the opening is half
    /// gone. Zero without an opening.
    pub(crate) fn curtain_revealed(&self) -> Duration {
        match self.curtain.motion {
            Motion::Opening(Some(start)) => start + CURTAIN_BLACK + CURTAIN_REVEAL / 2,
            _ => Duration::ZERO,
        }
    }

    /// A key or a click while the session opens: the curtain goes at once.
    pub fn end_curtain_opening(&mut self) {
        if matches!(self.curtain.motion, Motion::Opening(_)) && !self.greeter {
            self.curtain.motion = Motion::Up;
        }
    }

    /// How far the curtain is down while it comes down over the frozen desktop at `now`.
    pub fn curtain_lowering(&self, now: Duration) -> Option<f32> {
        match self.curtain.motion {
            Motion::Lowering(since) if now < since + CURTAIN => {
                Some(ui::curtain(now.saturating_sub(since), true))
            }
            _ => None,
        }
    }

    /// The session locks: the last frame of each screen is frozen, and the curtain comes down
    /// over it. With the screens dark, there is nothing to see: it is down at once.
    pub(crate) fn lower_curtain(&mut self) {
        self.curtain.frozen.clear();
        if self.screens_dark() {
            self.curtain.motion = Motion::Down;
            return;
        }
        if let Some(mut freezer) = self.curtain.freezer.take() {
            let frozen: Vec<_> = self
                .outputs()
                .into_iter()
                .filter_map(|output| freezer(self, &output).map(|texture| (output, texture)))
                .collect();
            self.curtain.freezer = Some(freezer);
            self.curtain.frozen = frozen;
        }
        self.curtain.motion = Motion::Lowering(self.now());
    }

    /// Unlocked: the curtain goes up off the desktop.
    pub(crate) fn raise_curtain(&mut self) {
        self.curtain.frozen.clear();
        self.curtain.motion = Motion::Raising(self.now());
    }

    /// The login screen left: the curtain fades to black.
    pub fn close_curtain(&mut self) {
        self.curtain.motion = Motion::Closing(self.now());
        self.queue_redraw();
    }

    /// Each loop iteration: a motion that ended settles, and the frozen frames go once the
    /// curtain is down: the desktop is kept no longer than the curtain takes.
    pub(crate) fn refresh_curtain(&mut self) {
        let now = self.now();
        let motion = &mut self.curtain.motion;
        match *motion {
            Motion::Lowering(since) if now >= since + CURTAIN => {
                *motion = Motion::Down;
                self.curtain.frozen.clear();
            }
            Motion::Raising(since) if now >= since + CURTAIN => *motion = Motion::Up,
            Motion::Opening(Some(since)) if now >= since + CURTAIN_BLACK + CURTAIN_REVEAL => {
                *motion = Motion::Up;
            }
            _ => {}
        }
    }

    /// Frozen frames held now (the tests).
    pub fn frozen_frames(&self) -> usize {
        self.curtain.frozen.len()
    }
}

/// Pushes the curtain in front of `items`, `down` of the screen down, `alpha` opaque: the light
/// picture over the dark one as the palette mixes them, or the plain background of the system
/// screens without them.
pub fn push(items: &mut Vec<Item>, state: &State, view: &View, down: f32, alpha: f32) {
    let offset = ((1.0 - down.clamp(0.0, 1.0)) * view.frame.size.h as f32).round() as i32;
    let rect = Rectangle::new(
        (view.frame.loc.x, view.frame.loc.y - offset).into(),
        view.frame.size,
    );
    if state.curtain.pictures.is_none() {
        let fill = ui::Colors::of(color::palette()).background.premultiplied();
        items.push(Item::Shape {
            key: Key::of(None, Part::Curtain(false)),
            rect,
            fill,
            border: fill,
            border_width: 0.0,
            radius: 0.0,
            alpha,
        });
        return;
    }
    let light = f32::from(color::mix_step()) / f32::from(color::MIX_STEPS);
    if light > 0.0 {
        items.push(Item::Curtain {
            key: Key::of(None, Part::Curtain(true)),
            rect,
            light: true,
            alpha: alpha * light,
        });
    }
    if light < 1.0 {
        items.push(Item::Curtain {
            key: Key::of(None, Part::Curtain(false)),
            rect,
            light: false,
            alpha,
        });
    }
}

/// Black over the whole screen, `alpha` opaque.
fn push_black(items: &mut Vec<Item>, view: &View, alpha: f32) {
    let black = screen_off::BLACK.premultiplied();
    items.push(Item::Shape {
        key: Key::of(None, Part::CurtainBlack),
        rect: view.frame,
        fill: black,
        border: black,
        border_width: 0.0,
        radius: 0.0,
        alpha,
    });
}

/// While locked and the curtain comes down at `down`: the curtain, the frozen frame of
/// `output` under it, and the plain background under that (a screen without a frozen frame).
pub fn push_lowering(
    items: &mut Vec<Item>,
    state: &State,
    (output, view): (&Output, &View),
    down: f32,
) {
    push(items, state, view, down, 1.0);
    if let Some((_, texture)) = state.curtain.frozen.iter().find(|(o, _)| o == output) {
        items.push(Item::Snapshot {
            key: Key::of(None, Part::Frozen),
            texture: texture.clone(),
            rect: view.frame,
            alpha: 1.0,
        });
    }
}

/// What the curtain adds in front of the desktop at `now`, unlocked: going up, the opening of
/// the session, the closing of the login screen, or the background of the login screen.
/// `true` while it moves.
pub fn push_front(items: &mut Vec<Item>, state: &State, view: &View, now: Duration) -> bool {
    let mut front = Vec::new();
    let moving = match state.curtain.motion {
        Motion::Raising(since) if now < since + CURTAIN => {
            let up = ui::curtain(now.saturating_sub(since), false);
            push(&mut front, state, view, up, 1.0);
            true
        }
        // Before the first frame, nothing: the frames of the backends start it.
        Motion::Opening(Some(start)) if !state.greeter => {
            let t = now.saturating_sub(start);
            if t < CURTAIN_BLACK {
                let fade = CURTAIN_FADE_EASE.apply(share(t, CURTAIN_BLACK));
                push(&mut front, state, view, 1.0, fade);
                push_black(&mut front, view, 1.0);
            } else {
                let reveal = CURTAIN_FADE_EASE.apply(share(t - CURTAIN_BLACK, CURTAIN_REVEAL));
                push(&mut front, state, view, 1.0, 1.0 - reveal);
            }
            t < CURTAIN_BLACK + CURTAIN_REVEAL
        }
        Motion::Closing(since) => {
            let fade = CURTAIN_FADE_EASE.apply(share(now.saturating_sub(since), CURTAIN_BLACK));
            push_black(&mut front, view, fade);
            now < since + CURTAIN_BLACK
        }
        _ => false,
    };
    items.splice(0..0, front);
    moving
}

/// The textures of the curtain, uploaded once per generation of its pictures.
#[derive(Default)]
pub struct Textures {
    generation: u64,
    dark: Option<wallpaper::Texture>,
    light: Option<wallpaper::Texture>,
}

impl Textures {
    /// Uploads the pictures of `state` if they changed.
    pub fn update(&mut self, renderer: &mut GlesRenderer, state: &State) {
        let Some((generation, pictures)) = state.curtain_pictures() else {
            return;
        };
        if generation == self.generation {
            return;
        }
        self.generation = generation;
        let mut upload = |picture: &lock_background::Picture| {
            let size = (picture.width as i32, picture.height as i32).into();
            renderer
                .import_memory(&picture.rgba, Fourcc::Abgr8888, size, false)
                .inspect_err(|err| tracing::warn!("curtain: uploading its picture: {err}"))
                .ok()
                .map(|texture| {
                    let buffer =
                        TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
                    wallpaper::Texture::new(buffer, (picture.width, picture.height))
                })
        };
        self.dark = upload(&pictures.dark);
        self.light = upload(&pictures.light);
    }

    pub fn get(&self, light: bool) -> Option<&wallpaper::Texture> {
        if light {
            self.light.as_ref()
        } else {
            self.dark.as_ref()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_goes_from_zero_to_one() {
        let ms = Duration::from_millis;
        assert_eq!(share(ms(0), ms(200)), 0.0);
        assert_eq!(share(ms(100), ms(200)), 0.5);
        assert_eq!(share(ms(400), ms(200)), 1.0);
    }
}
