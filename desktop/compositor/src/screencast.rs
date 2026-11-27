//! Screen capture for screen sharing (lot D-28b, ADR 0028): `ext-image-copy-capture-v1` with
//! `ext-output-image-capture-source-v1`, written here since Smithay 0.7 has no helper for them.
//!
//! Only the clients of the shell socket see these globals: the portal backend of the session,
//! which asks the user, never an ordinary application. A client makes a source of a screen,
//! opens a session on it and learns the buffer it needs (size, shared memory formats), then asks
//! for frames one at a time: the compositor draws the screen offscreen, as it shows (the cursor
//! aside), with the `Capturer` of the screenshots (lot D-35c), copies it into the client's
//! buffer and says it is ready. A frame waits until the screen showed a new picture since the
//! session's previous one (`screen_changed`, from the backend). While the session is locked the
//! screen is never drawn: every frame is black.

use std::sync::Mutex;

use smithay::{
    output::Output,
    reexports::{
        rustix,
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
                ext_output_image_capture_source_manager_v1::{
                    self, ExtOutputImageCaptureSourceManagerV1,
                },
            },
            image_copy_capture::v1::server::{
                ext_image_copy_capture_cursor_session_v1::{
                    self, ExtImageCopyCaptureCursorSessionV1,
                },
                ext_image_copy_capture_frame_v1::{
                    self, ExtImageCopyCaptureFrameV1, FailureReason,
                },
                ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
                ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
            },
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource, WEnum,
            protocol::{wl_buffer::WlBuffer, wl_output::Transform, wl_shm::Format},
        },
    },
    utils::{Physical, Size},
    wayland::shm::{BufferData, with_buffer_contents_mut},
};

use crate::{
    State,
    capture::{Image, Target},
};

/// The shared memory formats offered: the two every compositor has, blue first in memory. The
/// renderer reads red first: the copy swaps them.
const FORMATS: [Format; 2] = [Format::Xrgb8888, Format::Argb8888];

/// What a capture source shows. `None`: a screen gone or unknown, which can only stop.
struct SourceData(Option<Output>);

/// The sessions open, each with its source, and how many new pictures each screen showed.
#[derive(Default)]
pub struct Screencast {
    sessions: Vec<Session>,
    changes: Vec<(Output, u64)>,
}

struct Session {
    resource: ExtImageCopyCaptureSessionV1,
    output: Option<Output>,
    size: Size<i32, Physical>,
    /// The frame asked for, once its capture is sent.
    frame: Option<ExtImageCopyCaptureFrameV1>,
    /// The changes of the screen at the last frame given: the next one waits for another.
    given: Option<u64>,
}

/// A frame: its buffer, and whether its capture was asked for.
#[derive(Default)]
struct FrameData(Mutex<FrameState>);

#[derive(Default)]
struct FrameState {
    buffer: Option<WlBuffer>,
    captured: bool,
}

pub(crate) fn create_globals(dh: &DisplayHandle) {
    dh.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ());
    dh.create_global::<State, ExtOutputImageCaptureSourceManagerV1, ()>(1, ());
}

impl State {
    /// The capture sessions open: the bar shows that something is shared (lot D-28e).
    pub fn capture_sessions(&self) -> usize {
        self.screencast.sessions.len()
    }

    /// The screen `output` showed a new picture: the frames waiting for it are due. The real
    /// session calls it for each frame with damage it queues; the frames of a screen that
    /// shows nothing new wait.
    pub fn screen_changed(&mut self, output: &Output) {
        let changes = &mut self.screencast.changes;
        match changes.iter_mut().find(|(o, _)| o == output) {
            Some((_, count)) => *count = count.wrapping_add(1),
            None => changes.push((output.clone(), 1)),
        }
    }

    fn changes(&self, output: Option<&Output>) -> u64 {
        let changes = self.screencast.changes.iter();
        output
            .and_then(|output| changes.clone().find(|(o, _)| o == output))
            .map_or(0, |(_, count)| *count)
    }

    /// Size of what a session on `output` captures: the screen's frame, in pixels.
    fn capture_size(&self, output: &Output) -> Option<Size<i32, Physical>> {
        let views = self.views();
        let (_, view) = views.iter().find(|(o, _)| o == output)?;
        Some(view.frame.size)
    }

    /// Gives the frames asked for, once at each turn of the loop: those whose screen showed a
    /// new picture since the session's previous frame.
    pub(crate) fn refresh_screencast(&mut self) {
        let screens: Vec<Output> = self.space.outputs().cloned().collect();
        self.screencast.changes.retain(|(o, _)| screens.contains(o));
        let sessions = &mut self.screencast.sessions;
        sessions.retain(|s| s.resource.is_alive());
        for session in sessions.iter_mut() {
            session.frame = session.frame.take().filter(|f| f.is_alive());
        }
        let locked = self.is_locked();
        let due: Vec<(usize, u64)> = (0..self.screencast.sessions.len())
            .filter_map(|i| {
                let session = &self.screencast.sessions[i];
                let changes = self.changes(session.output.as_ref());
                let new = session.given != Some(changes);
                (session.frame.as_ref().is_some_and(asked) && new).then_some((i, changes))
            })
            .collect();
        // One drawing per screen, whatever the number of sessions on it.
        let mut drawn: Vec<(Output, Option<Image>)> = Vec::new();
        for (i, changes) in due {
            let (output, size) = {
                let session = &self.screencast.sessions[i];
                (session.output.clone(), session.size)
            };
            let output = output.filter(|o| self.capture_size(o) == Some(size));
            let pixels = match (&output, locked) {
                // The screen is gone or changed size: this session is over.
                (None, _) => Pixels::Gone,
                // Locked: black, the screen is never drawn.
                (Some(_), true) => Pixels::Black,
                (Some(output), false) => {
                    if !drawn.iter().any(|(o, _)| o == output) {
                        let image = self.draw_capture(&Target::Screen(output.clone()));
                        drawn.push((output.clone(), image));
                    }
                    match drawn.iter().find(|(o, _)| o == output) {
                        Some((_, Some(image))) => Pixels::Image(image),
                        _ => Pixels::Failed,
                    }
                }
            };
            let session = &mut self.screencast.sessions[i];
            let Some(frame) = session.frame.take() else {
                continue;
            };
            let image = match pixels {
                Pixels::Image(image) => Some(image),
                Pixels::Black => None,
                Pixels::Failed => {
                    frame.failed(FailureReason::Unknown);
                    continue;
                }
                Pixels::Gone => {
                    frame.failed(FailureReason::Stopped);
                    session.resource.stopped();
                    session.output = None;
                    continue;
                }
            };
            match give(&frame, image, size) {
                Ok(()) => session.given = Some(changes),
                Err(reason) => frame.failed(reason),
            }
        }
    }
}

/// What a frame gets.
enum Pixels<'a> {
    Image(&'a Image),
    /// The session is locked.
    Black,
    /// Nothing could draw the screen (no renderer).
    Failed,
    /// The screen is gone or changed size.
    Gone,
}

/// The client asked for the capture of `frame`.
fn asked(frame: &ExtImageCopyCaptureFrameV1) -> bool {
    let data = frame.data::<FrameData>();
    data.and_then(|d| d.0.lock().ok())
        .is_some_and(|d| d.captured)
}

/// Copies `image` (black for `None`) into the buffer of `frame`, of `size`, and tells the client.
fn give(
    frame: &ExtImageCopyCaptureFrameV1,
    image: Option<&Image>,
    size: Size<i32, Physical>,
) -> Result<(), FailureReason> {
    let state = frame.data::<FrameData>().ok_or(FailureReason::Unknown)?;
    let buffer = state
        .0
        .lock()
        .map_err(|_| FailureReason::Unknown)?
        .buffer
        .clone();
    let buffer = buffer.ok_or(FailureReason::Unknown)?;
    if image.is_some_and(|i| (i.width as i32, i.height as i32) != (size.w, size.h)) {
        return Err(FailureReason::Unknown);
    }
    let copied = with_buffer_contents_mut(&buffer, |ptr, len, data| {
        if !fits(&data, size, len) {
            return false;
        }
        // SAFETY: smithay maps the pool of `len` bytes at `ptr` for the duration of this
        // closure, and catches a client that shrinks it under us (SIGBUS); nothing else touches
        // the pool meanwhile, the compositor's thread being here.
        let pool = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        fill(pool, &data, image);
        true
    });
    if !matches!(copied, Ok(true)) {
        return Err(FailureReason::BufferConstraints);
    }
    frame.transform(Transform::Normal);
    frame.damage(0, 0, size.w, size.h);
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let seconds = u64::try_from(now.tv_sec).unwrap_or(0);
    let nanos = u32::try_from(now.tv_nsec).unwrap_or(0);
    frame.presentation_time((seconds >> 32) as u32, seconds as u32, nanos);
    frame.ready();
    Ok(())
}

/// The buffer described by `data`, in a pool of `len` bytes, takes an image of `size` in one of
/// `FORMATS`.
fn fits(data: &BufferData, size: Size<i32, Physical>, len: usize) -> bool {
    let rows = usize::try_from(data.height).unwrap_or(0);
    let (stride, offset) = (data.stride as i64, data.offset as i64);
    let end = offset + stride * (rows as i64 - 1) + i64::from(size.w) * 4;
    FORMATS.contains(&data.format)
        && (data.width, data.height) == (size.w, size.h)
        && stride >= i64::from(size.w) * 4
        && offset >= 0
        && end <= len as i64
}

/// Writes `image`, or black, row by row into `pool` where `data` says, blue first.
fn fill(pool: &mut [u8], data: &BufferData, image: Option<&Image>) {
    let width = data.width as usize * 4;
    for row in 0..data.height as usize {
        let start = data.offset as usize + row * data.stride as usize;
        let target = pool[start..start + width].chunks_exact_mut(4);
        match image {
            Some(image) => {
                let source = image.rgba[row * width..(row + 1) * width].chunks_exact(4);
                for (pixel, rgba) in target.zip(source) {
                    pixel.copy_from_slice(&[rgba[2], rgba[1], rgba[0], rgba[3]]);
                }
            }
            None => target.for_each(|pixel| pixel.copy_from_slice(&[0, 0, 0, 255])),
        }
    }
}

impl GlobalDispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtImageCopyCaptureManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _: &()) -> bool {
        State::is_shell_client(&client)
    }
}

impl GlobalDispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtOutputImageCaptureSourceManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _: &()) -> bool {
        State::is_shell_client(&client)
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtOutputImageCaptureSourceManagerV1,
        request: ext_output_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_output_image_capture_source_manager_v1::Request::CreateSource {
            source,
            output,
        } = request
        {
            data_init.init(source, SourceData(Output::from_resource(&output)));
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, SourceData> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtImageCaptureSourceV1,
        _: ext_image_capture_source_v1::Request,
        _: &SourceData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        manager: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            ext_image_copy_capture_manager_v1::Request::CreateSession {
                session,
                source,
                options,
            } => {
                if !matches!(options, WEnum::Value(_)) {
                    let error = ext_image_copy_capture_manager_v1::Error::InvalidOption;
                    manager.post_error(error, "unknown option");
                    return;
                }
                // The cursor is not painted yet (lot D-28c): the frames are as without it.
                let output = source.data::<SourceData>().and_then(|s| s.0.clone());
                let resource = data_init.init(session, ());
                let size = output.as_ref().and_then(|o| state.capture_size(o));
                let (Some(output), Some(size)) = (output, size) else {
                    resource.stopped();
                    return;
                };
                for format in FORMATS {
                    resource.shm_format(format);
                }
                resource.buffer_size(size.w as u32, size.h as u32);
                resource.done();
                tracing::info!(screen = output.name(), "screen capture session");
                state.screencast.sessions.push(Session {
                    resource,
                    output: Some(output),
                    size,
                    frame: None,
                    given: None,
                });
            }
            ext_image_copy_capture_manager_v1::Request::CreatePointerCursorSession {
                session,
                ..
            } => {
                data_init.init(session, ());
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtImageCopyCaptureCursorSessionV1,
        request: ext_image_copy_capture_cursor_session_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // No cursor of its own yet (lot D-28c): its session stops at once.
        if let ext_image_copy_capture_cursor_session_v1::Request::GetCaptureSession { session } =
            request
        {
            data_init.init(session, ()).stopped();
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        resource: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            let session = (state.screencast.sessions.iter()).find(|s| &s.resource == resource);
            if session.is_some_and(|s| s.frame.as_ref().is_some_and(Resource::is_alive)) {
                let error = ext_image_copy_capture_session_v1::Error::DuplicateFrame;
                resource.post_error(error, "a frame already exists");
                return;
            }
            let frame = data_init.init(frame, FrameData::default());
            match (state.screencast.sessions.iter_mut()).find(|s| &s.resource == resource) {
                Some(session) => session.frame = Some(frame),
                // A session that stopped: its frames fail.
                None => frame.failed(FailureReason::Stopped),
            }
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        frame: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &FrameData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use ext_image_copy_capture_frame_v1::{Error, Request};
        let Ok(mut current) = data.0.lock() else {
            return;
        };
        let captured = current.captured;
        match request {
            Request::AttachBuffer { .. } | Request::DamageBuffer { .. } | Request::Capture
                if captured =>
            {
                frame.post_error(Error::AlreadyCaptured, "the capture was asked for");
            }
            Request::AttachBuffer { buffer } => current.buffer = Some(buffer),
            Request::DamageBuffer {
                x,
                y,
                width,
                height,
            } => {
                if x < 0 || y < 0 || width <= 0 || height <= 0 {
                    frame.post_error(Error::InvalidBufferDamage, "invalid damage");
                }
            }
            Request::Capture if current.buffer.is_none() => {
                frame.post_error(Error::NoBuffer, "no buffer attached");
            }
            // Given at the next turn of the loop (`refresh_screencast`).
            Request::Capture => current.captured = true,
            _ => {}
        }
    }
}
