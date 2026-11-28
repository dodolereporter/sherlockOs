//! An application dragged from the Applications view to the dock (lot D-41d), as from Launchpad
//! on macOS. winit, under iced, neither starts a drag nor receives a drop on Wayland: as Evidence
//! does (`desktop/evidence/src/dnd.rs`, lot E-03), the view and the dock speak `wl_data_device`
//! themselves, on their surface's own connection (`Backend::from_foreign_display`), with an
//! event queue of their own in a thread. The interface thread sends `Command`s and gets `Event`s.
//!
//! What is dragged is a desktop id, in the private type `MIME`: no other application is offered
//! anything, and the dock accepts nothing else. The id read from a drop is bounded
//! (`MAX_ID_BYTES`, `PATIENCE`) and read off the interface thread.

use std::{
    fs::File,
    io::{Read as _, Write as _},
    os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _, OwnedFd},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::Duration,
};

use iced::futures::channel::mpsc as stream;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::{Backend, ObjectId},
    event_created_child,
    protocol::{
        wl_buffer::{self, WlBuffer},
        wl_compositor::WlCompositor,
        wl_data_device::{self, WlDataDevice},
        wl_data_device_manager::{DndAction, WlDataDeviceManager},
        wl_data_offer::{self, WlDataOffer},
        wl_data_source::{self, WlDataSource},
        wl_pointer::{self, WlPointer},
        wl_region::WlRegion,
        wl_registry::{self, WlRegistry},
        wl_seat::{self, WlSeat},
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};

/// The private type of a dragged application: its desktop id, in UTF-8.
pub const MIME: &str = "application/x-sherlockos-desktop-id";
/// Longest desktop id read from a drop.
pub const MAX_ID_BYTES: usize = 255;
/// Longest wait for the id once dropped.
const PATIENCE: Duration = Duration::from_secs(2);
/// Pace of the thread when nothing comes.
const TICK: Duration = Duration::from_millis(16);

/// What the interface asks.
#[derive(Debug)]
pub enum Command {
    /// The pointer, pressed on an application, moved far enough: drag its desktop `id`, shown
    /// by its icon file drawn `side` pixels wide at `scale`. The surface lets the pointer
    /// through from now on, so that the dock under it gets the drag.
    Drag {
        id: String,
        icon: Option<PathBuf>,
        side: u32,
        scale: i32,
    },
    /// The drag is over: the surface takes the pointer again from its next commit.
    Restore,
}

/// What happens on the Wayland side.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// An application dragged over the surface, at this place (logical).
    Over(f32, f32),
    /// It left the surface without being dropped.
    Left,
    /// It was dropped there: its desktop id.
    Dropped(String),
    /// The drag started here ended: dropped somewhere that took it, or not.
    DragEnded(bool),
}

/// The way to the thread.
#[derive(Debug, Clone)]
pub struct Link {
    commands: mpsc::Sender<Command>,
}

impl Link {
    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            tracing::debug!("drag and drop thread gone");
        }
    }
}

/// The raw pointers of the surface's `wl_display` and `wl_surface`, as numbers to cross threads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handles {
    display: usize,
    surface: usize,
}

/// The Wayland pointers of `window`, if it is on Wayland.
pub fn handles(window: &dyn iced::window::Window) -> Option<Handles> {
    use iced::window::raw_window_handle::{RawDisplayHandle, RawWindowHandle};
    let display = match window.display_handle().ok()?.as_raw() {
        RawDisplayHandle::Wayland(d) => d.display.as_ptr() as usize,
        _ => return None,
    };
    let surface = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Wayland(w) => w.surface.as_ptr() as usize,
        _ => return None,
    };
    Some(Handles { display, surface })
}

/// Starts the thread on the surface's connection. The events come through the receiver.
pub fn start(handles: Handles) -> Option<(Link, stream::UnboundedReceiver<Event>)> {
    // SAFETY: `display` is the `wl_display` the toolkit keeps open for the whole life of the
    // program (the dock and the view are resident). libwayland-client is thread-safe: this
    // connection reads into an event queue of its own.
    let backend = unsafe { Backend::from_foreign_display(handles.display as *mut _) };
    let conn = Connection::from_backend(backend);
    // SAFETY: `surface` is the live `wl_surface` of the same display, kept by the toolkit as
    // long as its window. It is only named in requests here, never destroyed.
    let id =
        unsafe { ObjectId::from_ptr(WlSurface::interface(), handles.surface as *mut _) }.ok()?;
    let surface = WlSurface::from_id(&conn, id).ok()?;
    let (commands, received) = mpsc::channel();
    let (events, stream) = stream::unbounded();
    let spawned = thread::Builder::new()
        .name("app-dnd".into())
        .spawn(move || run(conn, surface, received, events));
    match spawned {
        Ok(_) => Some((Link { commands }, stream)),
        Err(err) => {
            tracing::warn!("no drag and drop: {err}");
            None
        }
    }
}

/// The state of the thread.
struct Worker {
    events: stream::UnboundedSender<Event>,
    reads: mpsc::Sender<Option<String>>,
    globals: Vec<(u32, String, u32)>,
    registry: WlRegistry,
    seat: Option<WlSeat>,
    device: Option<WlDataDevice>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    manager: Option<WlDataDeviceManager>,
    /// Serial of the press of the pointer still held on the surface.
    pressed: Option<u32>,
    /// Types of the offers received, by offer.
    offers: Vec<(WlDataOffer, Vec<String>)>,
    /// The offer of an application dragged over the surface.
    over: Option<WlDataOffer>,
    /// The drag started here: its source and icon.
    dragging: Option<(WlDataSource, Option<WlSurface>)>,
}

fn run(
    conn: Connection,
    surface: WlSurface,
    commands: mpsc::Receiver<Command>,
    events: stream::UnboundedSender<Event>,
) {
    let mut queue: EventQueue<Worker> = conn.new_event_queue();
    let qh = queue.handle();
    let (reads, read) = mpsc::channel();
    let mut worker = Worker {
        events,
        reads,
        globals: Vec::new(),
        registry: conn.display().get_registry(&qh, ()),
        seat: None,
        device: None,
        compositor: None,
        shm: None,
        manager: None,
        pressed: None,
        offers: Vec::new(),
        over: None,
        dragging: None,
    };
    if let Err(err) = queue.roundtrip(&mut worker) {
        tracing::warn!("no drag and drop: {err}");
        return;
    }
    worker.bind(&qh);
    loop {
        if let Err(err) = queue.dispatch_pending(&mut worker) {
            tracing::warn!("drag and drop stopped: {err}");
            return;
        }
        loop {
            match commands.try_recv() {
                Ok(command) => worker.command(command, &surface, &qh),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        while let Ok(id) = read.try_recv() {
            worker.send(id.map_or(Event::Left, Event::Dropped));
        }
        if conn.flush().is_err() {
            return;
        }
        if let Some(guard) = queue.prepare_read() {
            if readable(guard.connection_fd().as_raw_fd(), TICK) {
                // Nothing to read is not an error; a broken connection shows at the dispatch.
                let _ = guard.read();
            }
        } else {
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Waits at most `wait` for `fd` to be readable.
fn readable(fd: i32, wait: Duration) -> bool {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: one valid `pollfd`, on the stack, for the duration of the call.
    unsafe { libc::poll(&mut poll, 1, ms) > 0 }
}

impl Worker {
    fn bind(&mut self, qh: &QueueHandle<Self>) {
        let find = |name: &str| {
            self.globals
                .iter()
                .find(|g| g.1 == name)
                .map(|g| (g.0, g.2))
        };
        let registry = &self.registry;
        if let Some((id, v)) = find(WlSeat::interface().name) {
            self.seat = Some(registry.bind(id, v.min(5), qh, ()));
        }
        if let Some((id, v)) = find(WlCompositor::interface().name) {
            self.compositor = Some(registry.bind(id, v.min(4), qh, ()));
        }
        if let Some((id, _)) = find(WlShm::interface().name) {
            self.shm = Some(registry.bind(id, 1, qh, ()));
        }
        if let Some((id, v)) = find(WlDataDeviceManager::interface().name) {
            let manager: WlDataDeviceManager = registry.bind(id, v.min(3), qh, ());
            if let Some(seat) = &self.seat {
                self.device = Some(manager.get_data_device(seat, qh, ()));
            }
            self.manager = Some(manager);
        }
    }

    fn send(&self, event: Event) {
        let _ = self.events.unbounded_send(event);
    }

    fn command(&mut self, command: Command, surface: &WlSurface, qh: &QueueHandle<Self>) {
        let (id, icon, side, scale) = match command {
            Command::Drag {
                id,
                icon,
                side,
                scale,
            } => (id, icon, side, scale),
            Command::Restore => {
                surface.set_input_region(None);
                return;
            }
        };
        let (Some(manager), Some(device), Some(serial)) =
            (&self.manager, &self.device, self.pressed)
        else {
            tracing::debug!("drag not started: no press seen");
            self.send(Event::DragEnded(false));
            return;
        };
        let source = manager.create_data_source(qh, id);
        source.offer(MIME.to_owned());
        source.set_actions(DndAction::Copy);
        // Drawn here, off the interface thread.
        let pixels = icon.and_then(|path| sherlock_ui::raster::rasterize_file(&path, side));
        let icon = pixels.and_then(|rgba| self.icon_surface(&rgba, side, scale, qh));
        device.start_drag(Some(&source), surface, icon.as_ref(), serial);
        // The view fades away: the pointer goes through it, to the dock under it. Applied at
        // the next commit of the view, which draws the fade.
        if let Some(compositor) = &self.compositor {
            let empty: WlRegion = compositor.create_region(qh, ());
            surface.set_input_region(Some(&empty));
            empty.destroy();
        }
        if let Some((old, old_icon)) = self.dragging.replace((source, icon)) {
            old.destroy();
            if let Some(old_icon) = old_icon {
                old_icon.destroy();
            }
        }
    }

    /// A surface showing the straight-alpha `rgba` of `side` pixels, the pointer in its middle.
    fn icon_surface(
        &self,
        rgba: &[u8],
        side: u32,
        scale: i32,
        qh: &QueueHandle<Self>,
    ) -> Option<WlSurface> {
        let (compositor, shm) = (self.compositor.as_ref()?, self.shm.as_ref()?);
        let stride = side.checked_mul(4)?;
        let size = usize::try_from(stride.checked_mul(side)?).ok()?;
        if rgba.len() != size || size == 0 {
            return None;
        }
        // ARGB8888 is little-endian BGRA, premultiplied.
        let mut pixels = Vec::with_capacity(size);
        for p in rgba.as_chunks::<4>().0 {
            let a = u16::from(p[3]);
            let pre = |c: u8| ((u16::from(c) * a + 127) / 255) as u8;
            pixels.extend_from_slice(&[pre(p[2]), pre(p[1]), pre(p[0]), p[3]]);
        }
        let mut file = memfd()?;
        file.write_all(&pixels).ok()?;
        let (len, w, s) = (
            i32::try_from(size).ok()?,
            i32::try_from(side).ok()?,
            i32::try_from(stride).ok()?,
        );
        let pool = shm.create_pool(file.as_fd(), len, qh, ());
        let buffer = pool.create_buffer(0, w, w, s, Format::Argb8888, qh, ());
        pool.destroy();
        let surface = compositor.create_surface(qh, ());
        let scale = scale.max(1);
        surface.set_buffer_scale(scale);
        let half = w / scale / 2;
        surface.attach(Some(&buffer), -half, -half);
        surface.damage(0, 0, w, w);
        surface.commit();
        Some(surface)
    }

    fn types_of(&self, offer: &WlDataOffer) -> &[String] {
        self.offers
            .iter()
            .find(|o| o.0 == *offer)
            .map(|o| o.1.as_slice())
            .unwrap_or_default()
    }

    fn forget(&mut self, offer: &WlDataOffer) {
        self.offers.retain(|o| o.0 != *offer);
        offer.destroy();
    }

    fn drag_ended(&mut self, source: &WlDataSource, taken: bool) {
        source.destroy();
        if let Some((dragged, icon)) = self.dragging.take() {
            if let Some(icon) = icon {
                icon.destroy();
            }
            if dragged != *source {
                return;
            }
        }
        self.send(Event::DragEnded(taken));
    }
}

/// The desktop id written to `fd`, at most `MAX_ID_BYTES`, waited `PATIENCE` at most: `None`
/// when it is longer, late, or not a plain name.
fn read_id(fd: OwnedFd) -> Option<String> {
    let mut file = File::from(fd);
    let mut data = Vec::new();
    let mut piece = [0; 256];
    loop {
        if !readable(file.as_raw_fd(), PATIENCE) {
            return None;
        }
        match file.read(&mut piece) {
            Ok(0) => break,
            Ok(n) if data.len() + n <= MAX_ID_BYTES => data.extend_from_slice(&piece[..n]),
            _ => return None,
        }
    }
    valid_id(&data)
}

/// A desktop id as the dock pins it: letters, digits, `.`, `-` and `_`, without `.desktop`.
pub fn valid_id(data: &[u8]) -> Option<String> {
    let id = std::str::from_utf8(data).ok()?.trim();
    let id = id.strip_suffix(".desktop").unwrap_or(id);
    let plain = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    plain.then(|| id.to_owned())
}

/// An anonymous file in memory, for the icon's pixels.
fn memfd() -> Option<File> {
    // SAFETY: a constant name ending with NUL, and flags of the man page.
    let fd = unsafe { libc::memfd_create(c"dock-drag-icon".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` was just created and is owned by nothing else.
    Some(unsafe { File::from_raw_fd(fd) })
}

impl Dispatch<WlRegistry, ()> for Worker {
    fn event(
        worker: &mut Self,
        _: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            worker.globals.push((name, interface, version));
        }
    }
}

impl Dispatch<WlSeat, ()> for Worker {
    fn event(
        _: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
            && caps.contains(wl_seat::Capability::Pointer)
        {
            seat.get_pointer(qh, ());
        }
    }
}

impl Dispatch<WlPointer, ()> for Worker {
    fn event(
        worker: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Only a button still held may start a drag: the press's serial, until its release.
        if let wl_pointer::Event::Button {
            serial,
            state: WEnum::Value(state),
            ..
        } = event
        {
            worker.pressed = match state {
                wl_pointer::ButtonState::Pressed => Some(serial),
                _ => None,
            };
        }
    }
}

impl Dispatch<WlDataDevice, ()> for Worker {
    fn event(
        worker: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => worker.offers.push((id, Vec::new())),
            // The clipboard is not this thread's business.
            wl_data_device::Event::Selection { id: Some(offer) } => worker.forget(&offer),
            wl_data_device::Event::Enter {
                serial, x, y, id, ..
            } => {
                let Some(offer) = id else {
                    return;
                };
                // Only an application dragged from the view: anything else is refused.
                if !worker.types_of(&offer).iter().any(|t| t == MIME) {
                    offer.accept(serial, None);
                    offer.set_actions(DndAction::empty(), DndAction::empty());
                    worker.forget(&offer);
                    return;
                }
                offer.accept(serial, Some(MIME.to_owned()));
                offer.set_actions(DndAction::Copy, DndAction::Copy);
                worker.over = Some(offer);
                worker.send(Event::Over(x as f32, y as f32));
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if worker.over.is_some() {
                    worker.send(Event::Over(x as f32, y as f32));
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = worker.over.take() {
                    worker.forget(&offer);
                    worker.send(Event::Left);
                }
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = worker.over.take() else {
                    return;
                };
                let Ok((reader, writer)) = std::io::pipe() else {
                    worker.forget(&offer);
                    worker.send(Event::Left);
                    return;
                };
                offer.receive(MIME.to_owned(), writer.as_fd());
                drop(writer);
                if offer.version() >= 3 {
                    offer.finish();
                }
                worker.forget(&offer);
                let reads = worker.reads.clone();
                let spawned = thread::Builder::new()
                    .name("dock-receive".into())
                    .spawn(move || {
                        let _ = reads.send(read_id(OwnedFd::from(reader)));
                    });
                if spawned.is_err() {
                    worker.send(Event::Left);
                }
            }
            _ => {}
        }
    }

    event_created_child!(Worker, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, ()),
    ]);
}

impl Dispatch<WlDataOffer, ()> for Worker {
    fn event(
        worker: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Some(types) = worker.offers.iter_mut().find(|o| o.0 == *offer)
            // Bounded: a client offering thousands of types gets the first ones.
            && types.1.len() < 64
        {
            types.1.push(mime_type);
        }
    }
}

impl Dispatch<WlDataSource, String> for Worker {
    fn event(
        worker: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        id: &String,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } if mime_type == MIME => {
                let data = id.clone().into_bytes();
                // Written off this thread: the reader may be slow, or never read.
                let _ = thread::Builder::new()
                    .name("app-dnd-send".into())
                    .spawn(move || {
                        let _ = File::from(fd).write_all(&data);
                    });
            }
            wl_data_source::Event::Cancelled => worker.drag_ended(source, false),
            wl_data_source::Event::DndFinished => worker.drag_ended(source, true),
            _ => {}
        }
    }
}

impl Dispatch<WlBuffer, ()> for Worker {
    fn event(
        _: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

wayland_client::delegate_noop!(Worker: ignore WlCompositor);
wayland_client::delegate_noop!(Worker: ignore WlSurface);
wayland_client::delegate_noop!(Worker: ignore WlRegion);
wayland_client::delegate_noop!(Worker: ignore WlShm);
wayland_client::delegate_noop!(Worker: ignore WlShmPool);
wayland_client::delegate_noop!(Worker: ignore WlDataDeviceManager);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_plain_desktop_id_is_taken_from_a_drop() {
        assert_eq!(
            valid_id(b"org.example.Files"),
            Some("org.example.Files".into())
        );
        assert_eq!(valid_id(b"firefox.desktop\n"), Some("firefox".into()));
        assert_eq!(valid_id(b""), None);
        assert_eq!(valid_id(b"../../etc/passwd"), None);
        assert_eq!(valid_id(b"a b"), None);
        assert_eq!(valid_id(&[0xff, 0xfe]), None);
    }
}
