//! The thumbnail of a screenshot just taken (lot D-35e), as on macOS: it slides in at the
//! bottom right corner of the screen of the image and stays `THUMBNAIL_VISIBLE`. A click opens
//! the file; a drag to the right throws it away past `THUMBNAIL_DISMISS` of its width, and
//! springs back short of it; a drag in any other direction carries the image into an
//! application (a drag and drop the compositor starts, `image/png` and the file's
//! `text/uri-list`). See `docs/screenshots.md`.

use std::{os::fd::OwnedFd, path::PathBuf, sync::Arc, time::Duration};

use sherlock_ui::{
    color,
    motion::{Animation, Fade, GEOMETRY_PRECISION},
    screenshot_tool::{self as ui, Rect},
};
use smithay::{
    input::pointer::{CursorIcon, GrabStartData},
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        wayland_server::protocol::wl_data_device_manager::DndAction,
    },
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::selection::data_device::{SourceMetadata, start_dnd},
};

use crate::{
    State,
    damage::{Key, Part},
    output::View,
    picture::Picture,
    render::Item,
};

/// The image just taken, as the worker hands it back.
pub struct Taken {
    pub png: Arc<[u8]>,
    pub path: Option<PathBuf>,
    pub size: (u32, u32),
    /// The thumbnail, drawn at the scale of its screen.
    pub picture: Option<Picture>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    from: (f64, f64),
    start: f64,
    sliding: bool,
}

pub(crate) struct Thumbnail {
    output: Output,
    taken: Taken,
    presence: Fade,
    /// How far right it is slid: by the pointer, or by its spring.
    slide: f64,
    spring: Option<Animation>,
    press: Option<Press>,
    leaving: bool,
    serial: u64,
}

const TRANSPARENT: [f32; 4] = [0.0; 4];

impl Thumbnail {
    fn bounds(&self, state: &State) -> Option<Rect> {
        let g = state.space.output_geometry(&self.output)?.to_f64();
        Some(Rect::new(g.loc.x, g.loc.y, g.size.w, g.size.h))
    }

    fn slide_at(&self, now: Duration) -> f64 {
        match (&self.press, &self.spring) {
            (Some(_), _) | (None, None) => self.slide,
            (None, Some(spring)) => spring.value(now),
        }
    }

    /// Where it is at `now`, in the space.
    fn rect(&self, state: &State, now: Duration) -> Option<Rect> {
        let bounds = self.bounds(state)?;
        let enter = f64::from(1.0 - self.presence.value(now).clamp(0.0, 1.0));
        let home = ui::thumbnail(&bounds, self.taken.size, 0.0);
        let out = ui::thumbnail_out(&home, &bounds);
        let slide = self.slide_at(now) + if self.leaving { 0.0 } else { enter * out };
        Some(ui::thumbnail(&bounds, self.taken.size, slide))
    }
}

impl State {
    /// Shows the thumbnail of `taken` on `output`, in place of the last one.
    pub(crate) fn show_thumbnail(&mut self, output: Output, taken: Taken) {
        let now = self.now();
        let serial = self.capture.thumbnail.as_ref().map_or(1, |t| t.serial + 1);
        self.capture.thumbnail = Some(Thumbnail {
            output,
            taken,
            presence: Fade::eased(0.0, 1.0, now, ui::ENTER, ui::ENTER_EASE),
            slide: 0.0,
            spring: None,
            press: None,
            leaving: false,
            serial,
        });
        self.hide_thumbnail_after(serial);
        self.queue_redraw();
    }

    fn hide_thumbnail_after(&mut self, serial: u64) {
        let timer = Timer::from_duration(ui::THUMBNAIL_VISIBLE);
        let inserted = self.loop_handle.insert_source(timer, move |_, _, state| {
            let held = state
                .capture
                .thumbnail
                .as_ref()
                .is_some_and(|t| t.serial == serial && t.press.is_some());
            if held {
                return TimeoutAction::ToDuration(ui::THUMBNAIL_VISIBLE);
            }
            if state
                .capture
                .thumbnail
                .as_ref()
                .is_some_and(|t| t.serial == serial)
            {
                state.dismiss_thumbnail(false);
            }
            TimeoutAction::Drop
        });
        if let Err(err) = inserted {
            tracing::warn!("screenshot thumbnail timer: {err}");
        }
    }

    /// The thumbnail goes: it fades out where it is, or slides out of the screen when thrown.
    fn dismiss_thumbnail(&mut self, thrown: bool) {
        let now = self.now();
        let Some(bounds) = self.capture.thumbnail.as_ref().and_then(|t| t.bounds(self)) else {
            return;
        };
        let Some(thumb) = &mut self.capture.thumbnail else {
            return;
        };
        let from = thumb.slide_at(now);
        let home = ui::thumbnail(&bounds, thumb.taken.size, 0.0);
        let to = if thrown {
            ui::thumbnail_out(&home, &bounds)
        } else {
            from
        };
        thumb.spring = Some(Animation::new(ui::SETTLE_SPRING, from, to, now));
        thumb.slide = to;
        let presence = thumb.presence.value(now);
        thumb.presence = Fade::eased(presence, 0.0, now, ui::LEAVE, ui::LEAVE_EASE);
        thumb.leaving = true;
        thumb.press = None;
        self.queue_redraw();
    }

    /// The thumbnail is under `location` and takes the pointer.
    pub(crate) fn over_thumbnail(&self, location: Point<f64, Logical>) -> bool {
        let Some(thumb) = &self.capture.thumbnail else {
            return false;
        };
        if thumb.press.is_some() {
            return true;
        }
        !thumb.leaving
            && thumb
                .rect(self, self.now())
                .is_some_and(|r| r.contains((location.x, location.y)))
    }

    pub(crate) fn thumbnail_motion(&mut self, location: Point<f64, Logical>) {
        let p = (location.x, location.y);
        let Some(thumb) = &mut self.capture.thumbnail else {
            return;
        };
        let Some(press) = &mut thumb.press else {
            return;
        };
        let (dx, dy) = (p.0 - press.from.0, p.1 - press.from.1);
        if !press.sliding && dx.hypot(dy) > ui::CLICK_SLOP {
            if dx > dy.abs() {
                press.sliding = true;
            } else {
                self.drag_thumbnail(location);
                return;
            }
        }
        if press.sliding {
            // To the left, past its place, it resists as a rubber band.
            let width = f64::from(ui::THUMBNAIL);
            thumb.slide = ui::rubber(press.start + dx, 0.0, f64::INFINITY, width);
        }
        self.queue_redraw();
    }

    pub(crate) fn thumbnail_button(&mut self, pressed: bool, location: Point<f64, Logical>) {
        let now = self.now();
        let p = (location.x, location.y);
        let Some(thumb) = &mut self.capture.thumbnail else {
            return;
        };
        if pressed {
            thumb.spring = None;
            thumb.press = Some(Press {
                from: p,
                start: thumb.slide,
                sliding: false,
            });
            return;
        }
        let Some(press) = thumb.press.take() else {
            return;
        };
        if !press.sliding {
            let path = thumb.taken.path.clone();
            self.dismiss_thumbnail(false);
            if let Some(path) = path {
                open(path);
            }
            return;
        }
        let width = f64::from(ui::THUMBNAIL);
        if thumb.slide > ui::THUMBNAIL_DISMISS * width {
            self.dismiss_thumbnail(true);
        } else {
            thumb.spring = Some(Animation::new(ui::SETTLE_SPRING, thumb.slide, 0.0, now));
            thumb.slide = 0.0;
            self.queue_redraw();
        }
    }

    /// Carries the image into the application under the pointer: a drag and drop the
    /// compositor starts. The thumbnail goes.
    fn drag_thumbnail(&mut self, location: Point<f64, Logical>) {
        let Some(thumb) = &mut self.capture.thumbnail else {
            return;
        };
        let mut mime_types = vec![String::from("image/png")];
        if thumb.taken.path.is_some() {
            mime_types.push(String::from("text/uri-list"));
        }
        self.capture.dragged = Some((thumb.taken.png.clone(), thumb.taken.path.clone()));
        self.dismiss_thumbnail(false);
        let start = GrabStartData {
            focus: None,
            button: crate::input::BTN_LEFT,
            location,
        };
        let metadata = SourceMetadata {
            mime_types,
            dnd_action: DndAction::Copy,
        };
        let (dh, seat) = (self.display_handle.clone(), self.seat.clone());
        let serial = SERIAL_COUNTER.next_serial();
        start_dnd(&dh, &seat, self, serial, Some(start), None, metadata);
    }

    /// An application asks for the image dropped on it.
    pub(crate) fn send_dragged(&mut self, mime_type: &str, fd: OwnedFd) {
        let Some((png, path)) = self.capture.dragged.clone() else {
            return;
        };
        match (mime_type, path) {
            ("image/png", _) => self.paste_png(fd, png),
            ("text/uri-list", Some(path)) => {
                let uri = format!("file://{}\r\n", uri_path(&path));
                self.paste_png(fd, Arc::from(uri.into_bytes()));
            }
            _ => {}
        }
    }

    /// The cursor over the thumbnail.
    pub(crate) fn thumbnail_cursor(&self) -> Option<CursorIcon> {
        let thumb = self.capture.thumbnail.as_ref()?;
        Some(match thumb.press {
            Some(Press { sliding: true, .. }) => CursorIcon::Grabbing,
            _ => CursorIcon::Default,
        })
    }

    /// The thumbnail's place, while it is shown: for the tests.
    pub fn thumbnail_rect(&self) -> Option<Rectangle<f64, Logical>> {
        let thumb = self.capture.thumbnail.as_ref()?;
        if thumb.leaving {
            return None;
        }
        let r = thumb.rect(self, self.now())?;
        Some(Rectangle::new((r.x, r.y).into(), (r.w, r.h).into()))
    }
}

/// `path` with the characters a URI cannot carry escaped.
fn uri_path(path: &std::path::Path) -> String {
    let mut out = String::new();
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Opens the file with the application that opens images (`xdg-open`), never waiting for it.
fn open(path: PathBuf) {
    let spawned = std::process::Command::new("xdg-open").arg(&path).spawn();
    match spawned {
        Ok(mut child) => {
            let waited = std::thread::Builder::new()
                .name(String::from("screenshot-open"))
                .spawn(move || {
                    let _ = child.wait();
                });
            if let Err(err) = waited {
                tracing::warn!("screenshot opened, not waited for: {err}");
            }
        }
        Err(err) => tracing::warn!(path = %path.display(), "screenshot not opened: {err}"),
    }
}

/// The thumbnail on `output`, in front of the windows, and whether it moves.
pub fn push_item(items: &mut Vec<Item>, state: &State, (output, view): (&Output, &View)) -> bool {
    let now = state.now();
    let Some(thumb) = &state.capture.thumbnail else {
        return false;
    };
    if &thumb.output != output {
        return false;
    }
    let alpha = thumb.presence.value(now).clamp(0.0, 1.0);
    let springing = thumb
        .spring
        .is_some_and(|s| thumb.press.is_none() && !s.is_done(now, GEOMETRY_PRECISION));
    let moving = !thumb.presence.is_done(now) || springing;
    if alpha <= 0.0 && thumb.leaving && !moving {
        return false;
    }
    let Some(r) = thumb.rect(state, now) else {
        return moving;
    };
    let rect = view.rect(Rectangle::new((r.x, r.y).into(), (r.w, r.h).into()));
    if let Some(picture) = &thumb.taken.picture {
        items.push(Item::Picture {
            key: Key::of(None, Part::CaptureThumbnail),
            rect,
            picture: picture.clone(),
            alpha,
        });
    }
    // Its shadow, a floating panel's.
    let palette = color::palette();
    let scale = view.scale as f32;
    items.push(Item::Shape {
        key: Key::of(None, Part::CaptureThumbnailShadow),
        rect: view.rect(Rectangle::new(
            (r.x - 1.0, r.y - 1.0).into(),
            (r.w + 2.0, r.h + 2.0).into(),
        )),
        fill: TRANSPARENT,
        border: palette.shadow.premultiplied(),
        border_width: scale,
        radius: (ui::THUMBNAIL_RADIUS + 1.0) * scale,
        alpha,
    });
    moving
}
