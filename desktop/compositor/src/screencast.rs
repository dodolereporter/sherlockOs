//! Screen capture for screen sharing (lots D-28b and D-28c, ADR 0028): `ext-image-copy-capture-v1`
//! with `ext-output-image-capture-source-v1` for a screen and, with `ext-foreign-toplevel-list-v1`,
//! `ext-foreign-toplevel-image-capture-source-v1` for a window. Smithay 0.7 has a helper for the
//! list only: the capture is written here.
//!
//! Only the portal backend of the session sees these globals, never an ordinary application:
//! a client of the shell socket whose program is [`PORTAL`] (`Access`). It makes a source of a
//! screen or of a window, opens a session on it and learns the buffer it needs (size, shared
//! memory formats), then asks for frames one at a time: the compositor draws the source
//! offscreen with the `Capturer` of the screenshots (lot D-35c), the cursor over it if the
//! session asked for it, copies it into the client's buffer and says it is ready. A frame waits
//! until a screen showed a new picture since the session's previous one (`screen_changed`, from
//! the backend). A source that changes size sends its new constraints and fails the frame
//! asked for, as the protocol wants. While the session is locked nothing is drawn: every frame
//! is black.

use std::{os::unix::net::UnixStream, path::Path, sync::Mutex};

use smithay::{
    delegate_foreign_toplevel_list,
    output::Output,
    reexports::{
        rustix,
        wayland_protocols::ext::{
            image_capture_source::v1::server::{
                ext_foreign_toplevel_image_capture_source_manager_v1::{
                    self, ExtForeignToplevelImageCaptureSourceManagerV1,
                },
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
    wayland::{
        foreign_toplevel_list::{
            ForeignToplevelHandle, ForeignToplevelListHandler, ForeignToplevelListState,
        },
        shm::{BufferData, with_buffer_contents_mut},
    },
};

use crate::{
    ClientState, State,
    capture::{Image, Target},
};

/// The program of the portal backend (lot D-28d): the only one that may capture.
pub const PORTAL: &str = "sherlock-portal";

/// Which clients of the shell socket may capture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Access {
    /// The program [`PORTAL`] only, recognised by the executable of the process at the other
    /// end of the socket: the real session.
    #[default]
    Portal,
    /// Every client of the shell socket: the tests, whose clients live in the test process.
    ShellClients,
}

/// The shared memory formats offered: the two every compositor has, blue first in memory. The
/// renderer reads red first: the copy swaps them.
const FORMATS: [Format; 2] = [Format::Xrgb8888, Format::Argb8888];

/// What a capture source shows.
#[derive(Clone, PartialEq)]
enum Source {
    Screen(Output),
    /// A window, by its id (`State::window_by_id`).
    Window(u32),
    /// A screen or window gone or unknown: its sessions can only stop.
    Gone,
}

struct SourceData(Source);

/// Id of the window a handle of the foreign toplevel list stands for.
struct ListedWindow(u32);

/// The sessions open, each with its source, how many new pictures each screen showed, and the
/// windows listed for the capture clients.
pub struct Screencast {
    sessions: Vec<Session>,
    changes: Vec<(Output, u64)>,
    access: Access,
    list: ForeignToplevelListState,
    /// Each window listed, by id, with its handle.
    listed: Vec<(u32, ForeignToplevelHandle)>,
}

struct Session {
    resource: ExtImageCopyCaptureSessionV1,
    source: Source,
    /// The cursor is drawn into the frames.
    cursor: bool,
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

impl Screencast {
    /// Creates the globals: only the capture clients see them.
    pub(crate) fn new(dh: &DisplayHandle) -> Self {
        dh.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ());
        dh.create_global::<State, ExtOutputImageCaptureSourceManagerV1, ()>(1, ());
        dh.create_global::<State, ExtForeignToplevelImageCaptureSourceManagerV1, ()>(1, ());
        Self {
            sessions: Vec::new(),
            changes: Vec::new(),
            access: Access::default(),
            list: ForeignToplevelListState::new_with_filter::<State>(dh, may_capture),
            listed: Vec::new(),
        }
    }
}

/// Whether `client` sees the capture globals: given when it connected (`Access`).
fn may_capture(client: &Client) -> bool {
    client.get_data::<ClientState>().is_some_and(|d| d.capture)
}

/// Whether the process at the other end of `stream` runs the program [`PORTAL`].
pub(crate) fn is_portal(stream: &UnixStream) -> bool {
    let Ok(peer) = rustix::net::sockopt::socket_peercred(stream) else {
        return false;
    };
    let exe = format!("/proc/{}/exe", peer.pid.as_raw_nonzero());
    let program = std::fs::read_link(exe).ok();
    program_is_portal(program.as_deref())
}

/// Whether `program`, the executable of a process, is the portal's. A program replaced on disk
/// while it runs reads ` (deleted)` at the end: an update, still the portal.
fn program_is_portal(program: Option<&Path>) -> bool {
    let name = program.and_then(Path::file_name).and_then(|n| n.to_str());
    name.is_some_and(|n| n.strip_suffix(" (deleted)").unwrap_or(n) == PORTAL)
}

impl State {
    /// The capture sessions open: the bar shows that something is shared (lot D-28d).
    pub fn capture_sessions(&self) -> usize {
        self.screencast.sessions.len()
    }

    /// Which clients of the shell socket connected from now on may capture.
    pub fn set_capture_access(&mut self, access: Access) {
        self.screencast.access = access;
    }

    /// Whether the client of the shell socket at the other end of `stream` may capture.
    pub(crate) fn capture_allowed(&self, stream: &UnixStream) -> bool {
        match self.screencast.access {
            Access::ShellClients => true,
            Access::Portal => is_portal(stream),
        }
    }

    /// Lists the windows for the capture clients: a handle for each new one, its title and
    /// application changed, `closed` for those gone. Called with the shell's refresh, after a
    /// change.
    pub(crate) fn refresh_toplevel_list(&mut self, windows: &[(u32, String, String)]) {
        let screencast = &mut self.screencast;
        screencast.listed.retain(|(id, handle)| {
            let open = windows.iter().any(|w| w.0 == *id);
            if !open {
                screencast.list.remove_toplevel(handle);
            }
            open
        });
        for (id, title, app_id) in windows {
            match screencast.listed.iter().find(|(i, _)| i == id) {
                Some((_, handle)) => {
                    if handle.title() != *title || handle.app_id() != *app_id {
                        handle.send_title(title);
                        handle.send_app_id(app_id);
                        handle.send_done();
                    }
                }
                None => {
                    let handle = screencast.list.new_toplevel::<State>(title, app_id);
                    handle.user_data().insert_if_missing(|| ListedWindow(*id));
                    screencast.listed.push((*id, handle));
                }
            }
        }
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

    /// The new pictures a session on `source` waits for: those of its screen, or of any screen
    /// for a window, which may be anywhere or hidden.
    fn changes(&self, source: &Source) -> u64 {
        let changes = self.screencast.changes.iter();
        match source {
            Source::Screen(output) => changes
                .clone()
                .find(|(o, _)| o == output)
                .map_or(0, |(_, count)| *count),
            Source::Window(_) => changes.fold(0, |sum, (_, count)| sum.wrapping_add(*count)),
            Source::Gone => 0,
        }
    }

    /// What a session on `source` draws, and the size of its frames in pixels: a screen's
    /// frame, a window as `window_anim::snapshot` draws it. `None` for a source gone.
    fn capture_target(&self, source: &Source) -> Option<(Target, Size<i32, Physical>)> {
        let target = match source {
            Source::Screen(output) => Target::Screen(output.clone()),
            Source::Window(id) => Target::Window(self.window_by_id(*id)?),
            Source::Gone => return None,
        };
        let size = crate::capture::target_size(self, &target)?;
        Some((target, size))
    }

    /// Gives the frames asked for, once at each turn of the loop: those whose source showed a
    /// new picture since the session's previous frame. A source that changed size gives its new
    /// constraints first.
    pub(crate) fn refresh_screencast(&mut self) {
        let screens: Vec<Output> = self.space.outputs().cloned().collect();
        self.screencast.changes.retain(|(o, _)| screens.contains(o));
        self.screencast.list.cleanup_closed_handles();
        let sessions = &mut self.screencast.sessions;
        sessions.retain(|s| s.resource.is_alive());
        for session in sessions.iter_mut() {
            session.frame = session.frame.take().filter(|f| f.is_alive());
        }
        let locked = self.is_locked();
        // One drawing per source, whatever the number of sessions on it.
        let mut drawn: Vec<(Source, bool, Option<Image>)> = Vec::new();
        for i in 0..self.screencast.sessions.len() {
            let (source, cursor, size, given) = {
                let s = &self.screencast.sessions[i];
                (s.source.clone(), s.cursor, s.size, s.given)
            };
            let target = self.capture_target(&source);
            let changes = self.changes(&source);
            let session = &mut self.screencast.sessions[i];
            let Some((target, now)) = target else {
                // The screen or the window is gone: this session is over.
                if session.source != Source::Gone {
                    session.source = Source::Gone;
                    session.resource.stopped();
                }
                if let Some(frame) = session.frame.take() {
                    frame.failed(FailureReason::Stopped);
                }
                continue;
            };
            if now != size {
                session.size = now;
                constraints(&session.resource, now);
                if let Some(frame) = session.frame.take() {
                    frame.failed(FailureReason::BufferConstraints);
                }
                continue;
            }
            let due = session.frame.as_ref().is_some_and(asked) && given != Some(changes);
            if !due {
                continue;
            }
            let pixels = if locked {
                // Locked: black, nothing is drawn.
                Pixels::Black
            } else {
                let known = drawn
                    .iter()
                    .position(|(s, c, _)| *s == source && *c == cursor);
                let index = known.unwrap_or_else(|| {
                    let image = self.draw_capture(&target, cursor);
                    drawn.push((source.clone(), cursor, image));
                    drawn.len() - 1
                });
                match &drawn[index].2 {
                    Some(image) if image_size(image) == size => Pixels::Image(image),
                    // Drawn at another size than announced (a window resized in between):
                    // the next turn gives the new constraints.
                    Some(_) => continue,
                    None => Pixels::Failed,
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
            };
            match give(&frame, image, size) {
                Ok(()) => session.given = Some(changes),
                Err(reason) => frame.failed(reason),
            }
        }
    }
}

/// The size of `image` in pixels.
fn image_size(image: &Image) -> Size<i32, Physical> {
    (image.width as i32, image.height as i32).into()
}

/// Tells a session the buffer its frames need: `size`, in one of `FORMATS`.
fn constraints(resource: &ExtImageCopyCaptureSessionV1, size: Size<i32, Physical>) {
    for format in FORMATS {
        resource.shm_format(format);
    }
    resource.buffer_size(size.w as u32, size.h as u32);
    resource.done();
}

/// What a frame gets.
enum Pixels<'a> {
    Image(&'a Image),
    /// The session is locked.
    Black,
    /// Nothing could draw the source (no renderer).
    Failed,
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
    if image.is_some_and(|i| image_size(i) != size) {
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
        let (target, _) = pool[start..start + width].as_chunks_mut::<4>();
        match image {
            Some(image) => {
                let (source, _) = image.rgba[row * width..(row + 1) * width].as_chunks::<4>();
                for (pixel, [r, g, b, a]) in target.iter_mut().zip(source) {
                    *pixel = [*b, *g, *r, *a];
                }
            }
            None => target.fill([0, 0, 0, 255]),
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
        may_capture(&client)
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
        may_capture(&client)
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
            let output = Output::from_resource(&output);
            data_init.init(
                source,
                SourceData(output.map_or(Source::Gone, Source::Screen)),
            );
        }
    }
}

impl GlobalDispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ExtForeignToplevelImageCaptureSourceManagerV1>,
        _: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }

    fn can_view(client: Client, _: &()) -> bool {
        may_capture(&client)
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
            source,
            toplevel_handle,
        } = request
        {
            let handle = ForeignToplevelHandle::from_resource(&toplevel_handle);
            let id = handle
                .filter(|h| !h.is_closed())
                .and_then(|h| h.user_data().get::<ListedWindow>().map(|w| w.0));
            data_init.init(source, SourceData(id.map_or(Source::Gone, Source::Window)));
        }
    }
}

impl ForeignToplevelListHandler for State {
    fn foreign_toplevel_list_state(&mut self) -> &mut ForeignToplevelListState {
        &mut self.screencast.list
    }
}

delegate_foreign_toplevel_list!(State);

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
                let cursor = matches!(
                    options,
                    WEnum::Value(o) if o.contains(ext_image_copy_capture_manager_v1::Options::PaintCursors)
                );
                let source = source
                    .data::<SourceData>()
                    .map_or(Source::Gone, |s| s.0.clone());
                let resource = data_init.init(session, ());
                let Some((target, size)) = state.capture_target(&source) else {
                    resource.stopped();
                    return;
                };
                constraints(&resource, size);
                match target {
                    Target::Screen(output) => tracing::info!(screen = output.name(), "capture"),
                    Target::Window(_) => tracing::info!("window capture"),
                }
                state.screencast.sessions.push(Session {
                    resource,
                    source,
                    cursor,
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
        // The cursor is drawn into the frames when asked (`paint_cursors`), never given apart
        // as metadata: its own session stops at once.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_portal_program_may_capture() {
        let portal = Path::new("/usr/local/libexec/sherlockos/sherlock-portal");
        assert!(program_is_portal(Some(portal)));
        let updated = Path::new("/usr/local/libexec/sherlockos/sherlock-portal (deleted)");
        assert!(program_is_portal(Some(updated)));
        for other in ["/usr/bin/bar", "/tmp/sherlock-portal-x", "/"] {
            assert!(!program_is_portal(Some(Path::new(other))), "{other}");
        }
        assert!(!program_is_portal(None));
    }
}
