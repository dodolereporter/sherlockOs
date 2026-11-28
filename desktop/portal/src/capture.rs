//! The capture client of the portal, on the compositor's shell socket (ADR 0028): lists the
//! screens and windows, and captures the one chosen into a PipeWire stream, frame after frame,
//! on a thread of its own, until it is stopped or its source is gone.
//!
//! The compositor gives a frame when the source shows a new picture, at most once per vertical
//! blank: the thread waits for it with a short timeout, so that a stop is seen at once even on a
//! still screen.

use std::{
    fs::File,
    os::{fd::AsFd as _, unix::net::UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

use memmap2::MmapMut;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use wayland_client::{
    Connection, Dispatch, EventQueue, QueueHandle, WEnum, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_buffer::WlBuffer,
        wl_output::{self, WlOutput},
        wl_registry::WlRegistry,
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
    },
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

use crate::pipewire;

/// How long the thread waits for the compositor before it looks at the stop flag again.
const WAIT: Duration = Duration::from_millis(100);
/// The largest frame taken, in pixels a side: a bigger size from the compositor is refused.
const MAX_SIDE: u32 = 16384;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the compositor's shell socket is not known (WAYLAND_DISPLAY)")]
    NoSocket,
    #[error("connecting to the compositor: {0}")]
    Connect(#[from] std::io::Error),
    #[error("Wayland: {0}")]
    Wayland(String),
    #[error("the compositor lets this program capture nothing")]
    NotAllowed,
    #[error("the screen or window is gone")]
    Gone,
    #[error(transparent)]
    PipeWire(#[from] pipewire::Error),
}

fn wayland(err: impl std::fmt::Display) -> Error {
    Error::Wayland(err.to_string())
}

/// A screen, by the name of its output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub name: String,
    pub description: String,
}

/// A window, by its identifier in the foreign toplevel list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub identifier: String,
    pub title: String,
    pub app_id: String,
}

/// What can be shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Screen(Screen),
    Window(Window),
}

/// What a connection learnt.
#[derive(Default)]
struct Client {
    outputs: Vec<(WlOutput, Screen)>,
    toplevels: Vec<(ExtForeignToplevelHandleV1, Window)>,
    /// The constraints of the session: size, `done`, `stopped`.
    size: Option<(u32, u32)>,
    xrgb: bool,
    done: bool,
    stopped: bool,
    /// The frame's outcome: ready, or failed with a reason.
    frame: Option<Result<(), Option<FailureReason>>>,
}

struct Wayland {
    conn: Connection,
    queue: EventQueue<Client>,
    client: Client,
    globals: wayland_client::globals::GlobalList,
}

impl Wayland {
    /// Connects to the shell socket and reads the screens and the windows.
    fn connect() -> Result<Self, Error> {
        let path = shell_protocol::socket_from_env().ok_or(Error::NoSocket)?;
        let conn = Connection::from_socket(UnixStream::connect(path)?).map_err(wayland)?;
        let (globals, queue) = registry_queue_init::<Client>(&conn).map_err(wayland)?;
        let mut wayland = Self {
            conn,
            queue,
            client: Client::default(),
            globals,
        };
        let qh = wayland.queue.handle();
        // The capture globals are offered to the portal only: without them, nothing to do.
        wayland.bind::<ExtImageCopyCaptureManagerV1>(1)?;
        for global in wayland.globals.contents().clone_list() {
            if global.interface == "wl_output" {
                let version = global.version.min(4);
                let output: WlOutput =
                    (wayland.globals.registry()).bind(global.name, version, &qh, ());
                let screen = Screen {
                    name: String::new(),
                    description: String::new(),
                };
                wayland.client.outputs.push((output, screen));
            }
        }
        let _list: ExtForeignToplevelListV1 = wayland.bind(1)?;
        wayland.roundtrip()?;
        wayland.roundtrip()?;
        Ok(wayland)
    }

    fn bind<I>(&self, version: u32) -> Result<I, Error>
    where
        I: wayland_client::Proxy + 'static,
        Client: Dispatch<I, ()>,
    {
        let qh = self.queue.handle();
        (self.globals)
            .bind(&qh, version..=version, ())
            .map_err(|_| Error::NotAllowed)
    }

    fn roundtrip(&mut self) -> Result<(), Error> {
        self.queue.roundtrip(&mut self.client).map_err(wayland)?;
        Ok(())
    }

    /// Reads what came, waiting `WAIT` at most.
    fn wait(&mut self) -> Result<(), Error> {
        self.conn.flush().map_err(wayland)?;
        if let Some(guard) = self.queue.prepare_read() {
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let timeout = Timespec::try_from(WAIT).ok();
            let ready = poll(&mut fds, timeout.as_ref()).map_err(wayland)?;
            if ready > 0 {
                guard.read().map_err(wayland)?;
            }
        }
        self.queue
            .dispatch_pending(&mut self.client)
            .map_err(wayland)?;
        Ok(())
    }
}

/// The screens and the windows that can be shared now.
pub fn sources() -> Result<Vec<Source>, Error> {
    let wayland = Wayland::connect()?;
    let screens = wayland
        .client
        .outputs
        .into_iter()
        .map(|(_, s)| Source::Screen(s));
    let windows = wayland
        .client
        .toplevels
        .into_iter()
        .map(|(_, w)| Source::Window(w));
    Ok(screens.chain(windows).collect())
}

/// A source shared into PipeWire, until dropped.
pub struct Share {
    pub node: u32,
    pub size: (u32, u32),
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Share {
    /// Whether the share still runs: its source is there and PipeWire reads it.
    pub fn running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }
}

impl Drop for Share {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Starts sharing `source`, the cursor drawn in if `cursor`. Returns once the PipeWire node is
/// there, for the portal's answer.
pub fn share(source: Source, cursor: bool) -> Result<Share, Error> {
    let stop = Arc::new(AtomicBool::new(false));
    let (started, outcome) = mpsc::channel();
    let flag = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name(String::from("screen share"))
        .spawn(move || {
            if let Err(err) = run(&source, cursor, &flag, &started) {
                tracing::info!("screen share ended: {err}");
                let _ = started.send(Err(err));
            }
        })?;
    match outcome.recv() {
        Ok(Ok((node, size))) => Ok(Share {
            node,
            size,
            stop,
            thread: Some(thread),
        }),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(Error::Gone),
    }
}

/// A share's start: its PipeWire node and size, or why it could not start.
type Started = Result<(u32, (u32, u32)), Error>;

/// A buffer in shared memory, mapped, and its size.
struct Buffer {
    buffer: WlBuffer,
    map: MmapMut,
    size: (u32, u32),
}

impl Buffer {
    fn new(wayland: &Wayland, shm: &WlShm, (w, h): (u32, u32)) -> Result<Self, Error> {
        let len = w as usize * 4 * h as usize;
        let fd = rustix::fs::memfd_create("sherlock-portal", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(std::io::Error::from)?;
        let file = File::from(fd);
        file.set_len(len as u64)?;
        // SAFETY: the file is ours alone (a memfd); the compositor writes into it only between
        // `capture` and `ready`, while this thread waits.
        let map = unsafe { MmapMut::map_mut(&file) }?;
        let qh = wayland.queue.handle();
        let pool = shm.create_pool(file.as_fd(), len as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            w as i32 * 4,
            Format::Xrgb8888,
            &qh,
            (),
        );
        pool.destroy();
        Ok(Self {
            buffer,
            map,
            size: (w, h),
        })
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

/// A capture session on `source`, its first constraints, and the shared memory to fill.
struct Opened {
    wayland: Wayland,
    session: ExtImageCopyCaptureSessionV1,
    shm: WlShm,
    size: (u32, u32),
}

fn open(source: &Source, cursor: bool, stop: &AtomicBool) -> Result<Opened, Error> {
    let mut wayland = Wayland::connect()?;
    let qh = wayland.queue.handle();
    let capture_source: ExtImageCaptureSourceV1 = match source {
        Source::Screen(screen) => {
            let outputs = &wayland.client.outputs;
            let (output, _) =
                (outputs.iter().find(|(_, s)| s.name == screen.name)).ok_or(Error::Gone)?;
            let manager: ExtOutputImageCaptureSourceManagerV1 = wayland.bind(1)?;
            manager.create_source(output, &qh, ())
        }
        Source::Window(window) => {
            let toplevels = &wayland.client.toplevels;
            let found = toplevels
                .iter()
                .find(|(_, w)| w.identifier == window.identifier);
            let (handle, _) = found.ok_or(Error::Gone)?;
            let manager: ExtForeignToplevelImageCaptureSourceManagerV1 = wayland.bind(1)?;
            manager.create_source(handle, &qh, ())
        }
    };
    let shm: WlShm = wayland.bind(1)?;
    let manager: ExtImageCopyCaptureManagerV1 = wayland.bind(1)?;
    let options = match cursor {
        true => Options::PaintCursors,
        false => Options::empty(),
    };
    let session = manager.create_session(&capture_source, options, &qh, ());
    let size = constraints(&mut wayland, stop)?;
    Ok(Opened {
        wayland,
        session,
        shm,
        size,
    })
}

/// Asks for one frame into `buffer` and waits for it, or for `stop`.
fn frame(
    wayland: &mut Wayland,
    session: &ExtImageCopyCaptureSessionV1,
    buffer: &Buffer,
    stop: &AtomicBool,
) -> Result<Option<Result<(), Option<FailureReason>>>, Error> {
    let qh = wayland.queue.handle();
    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&buffer.buffer);
    let (w, h) = buffer.size;
    frame.damage_buffer(0, 0, w as i32, h as i32);
    frame.capture();
    wayland.client.frame = None;
    while wayland.client.frame.is_none() && !stop.load(Ordering::Relaxed) {
        wayland.wait()?;
    }
    frame.destroy();
    Ok(wayland.client.frame.take())
}

/// One picture of a source: `height` rows of `width` pixels, BGRx.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub bgrx: Vec<u8>,
}

/// One picture of `source`, without the cursor, for the picker's thumbnails. Gives up once
/// `stop` is set.
pub fn picture(source: &Source, stop: &AtomicBool) -> Result<Picture, Error> {
    let Opened {
        mut wayland,
        session,
        shm,
        size,
    } = open(source, false, stop)?;
    let buffer = Buffer::new(&wayland, &shm, size)?;
    match frame(&mut wayland, &session, &buffer, stop)? {
        Some(Ok(())) => Ok(Picture {
            width: size.0,
            height: size.1,
            bgrx: buffer.map.to_vec(),
        }),
        _ => Err(Error::Gone),
    }
}

fn run(
    source: &Source,
    cursor: bool,
    stop: &AtomicBool,
    started: &mpsc::Sender<Started>,
) -> Result<(), Error> {
    let Opened {
        mut wayland,
        session,
        shm,
        size,
    } = open(source, cursor, stop)?;
    let mut buffer = Buffer::new(&wayland, &shm, size)?;
    let name = match source {
        Source::Screen(_) => "sherlock-screen-share",
        Source::Window(_) => "sherlock-window-share",
    };
    let mut stream = pipewire::Stream::new(name, size)?;
    let _ = started.send(Ok((stream.node(), size)));
    while !stop.load(Ordering::Relaxed) {
        if wayland.client.done {
            // New constraints: the source changed size.
            let size = constraints(&mut wayland, stop)?;
            if size != buffer.size {
                buffer = Buffer::new(&wayland, &shm, size)?;
                stream.resize(size);
            }
        }
        let outcome = frame(&mut wayland, &session, &buffer, stop)?;
        if stream.failed() {
            return Err(Error::Wayland(String::from("PipeWire stopped reading")));
        }
        let (w, _) = buffer.size;
        match outcome {
            Some(Ok(())) => stream.push(&buffer.map, w as usize * 4),
            Some(Err(Some(FailureReason::BufferConstraints))) => {}
            Some(Err(Some(FailureReason::Stopped))) => return Err(Error::Gone),
            Some(Err(_)) => std::thread::sleep(WAIT),
            None => {}
        }
        if wayland.client.stopped {
            return Err(Error::Gone);
        }
    }
    session.destroy();
    Ok(())
}

/// Waits for the session's constraints: a size in `xrgb8888`.
fn constraints(wayland: &mut Wayland, stop: &AtomicBool) -> Result<(u32, u32), Error> {
    while !wayland.client.done && !wayland.client.stopped && !stop.load(Ordering::Relaxed) {
        wayland.wait()?;
    }
    wayland.client.done = false;
    // The formats come again with the next constraints.
    let xrgb = std::mem::take(&mut wayland.client.xrgb);
    match wayland.client.size {
        _ if wayland.client.stopped => Err(Error::Gone),
        Some((w, h)) if xrgb && (1..=MAX_SIDE).contains(&w) && (1..=MAX_SIDE).contains(&h) => {
            Ok((w, h))
        }
        _ => Err(Error::Wayland(String::from("no usable buffer size"))),
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for Client {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlOutput, ()> for Client {
    fn event(
        client: &mut Self,
        output: &WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, screen)) = client.outputs.iter_mut().find(|(o, _)| o == output) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => screen.name = name,
            wl_output::Event::Description { description } => screen.description = description,
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            let window = Window {
                identifier: String::new(),
                title: String::new(),
                app_id: String::new(),
            };
            client.toplevels.push((toplevel, window));
        }
    }

    event_created_child!(Client, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for Client {
    fn event(
        client: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_foreign_toplevel_handle_v1::Event;
        if let Event::Closed = event {
            client.toplevels.retain(|(h, _)| h != handle);
            return;
        }
        let Some((_, window)) = client.toplevels.iter_mut().find(|(h, _)| h == handle) else {
            return;
        };
        match event {
            Event::Identifier { identifier } => window.identifier = identifier,
            Event::Title { title } => window.title = title,
            Event::AppId { app_id } => window.app_id = app_id,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => client.size = Some((width, height)),
            Event::ShmFormat {
                format: WEnum::Value(Format::Xrgb8888),
            } => client.xrgb = true,
            Event::Done => client.done = true,
            Event::Stopped => client.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for Client {
    fn event(
        client: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Ready => client.frame = Some(Ok(())),
            Event::Failed { reason } => client.frame = Some(Err(reason.into_result().ok())),
            _ => {}
        }
    }
}

delegate_noop!(Client: ignore WlShm);
delegate_noop!(Client: WlShmPool);
delegate_noop!(Client: ignore WlBuffer);
delegate_noop!(Client: ExtImageCopyCaptureManagerV1);
delegate_noop!(Client: ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(Client: ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(Client: ExtImageCaptureSourceV1);
