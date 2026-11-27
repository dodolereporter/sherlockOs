//! Drag and drop and the clipboard of files with the other applications (lot E-03), on the
//! Wayland connection of the window.
//!
//! winit, under iced, neither starts a drag nor receives a drop on Wayland, and iced's clipboard
//! only knows text. Evidence speaks `wl_data_device` itself, on the window's own connection
//! (`Backend::from_foreign_display`), with an event queue of its own in a thread: its own
//! `wl_seat`, `wl_pointer` (for the serial of the press that starts a drag) and `wl_keyboard`
//! (for the serial of the key that copies), a data device, and the drag icon's surface. The
//! interface thread sends it `Command`s and gets `Event`s back.
//!
//! What comes from the others is bounded (`transfer::MAX_BYTES`, `PATIENCE` per piece) and read
//! off the interface thread.

use std::{
    fs::File,
    io::{Read as _, Write as _},
    os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _, OwnedFd},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
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
        wl_keyboard::{self, WlKeyboard},
        wl_pointer::{self, WlPointer},
        wl_registry::{self, WlRegistry},
        wl_seat::{self, WlSeat},
        wl_shm::{Format, WlShm},
        wl_shm_pool::WlShmPool,
        wl_surface::WlSurface,
    },
};

use crate::transfer::{self, Mode};

/// Longest wait for the other application to write the next piece of a list.
const PATIENCE: Duration = Duration::from_secs(5);
/// How long the icon of a refused drag stays, for the compositor to glide it back.
const ICON_LINGER: Duration = sherlock_ui::drag::RETURN;
/// Pace of the thread when nothing comes.
const TICK: Duration = Duration::from_millis(16);

/// The picture under the pointer during a drag: straight-alpha RGBA, at `scale` pixels per
/// logical pixel, with the pointer at `hotspot` (logical) in it.
#[derive(Debug, Clone)]
pub struct Icon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub scale: i32,
    pub hotspot: (i32, i32),
}

/// What the interface asks.
#[derive(Debug)]
pub enum Command {
    /// The pointer, pressed on the selection, moved far enough: drag these files, shown by the
    /// first ones (`drag_image`).
    Drag(Vec<PathBuf>, Vec<crate::drag_image::Item>),
    /// Ctrl + C or Ctrl + X: these files go to the clipboard.
    Clip(Mode, Vec<PathBuf>),
    /// Ctrl + V: the files of the clipboard, if it holds some (`Event::Pasted`).
    Paste,
    /// The action wanted for a drop on the window, from the keys held: copy with Ctrl.
    Prefer(Action),
    /// A drop on the window cannot land where the pointer is (not a folder, the folder dragged
    /// itself): it is refused until `accept` again.
    Accept(bool),
}

/// What the drop does with the files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Copy,
    Move,
}

/// What happens on the Wayland side.
#[derive(Debug, Clone)]
pub enum Event {
    /// Files are dragged over the window, at that logical point.
    Over(f32, f32),
    /// They left it, or the drag was cancelled.
    Left,
    /// They were dropped there, to be copied or moved.
    Dropped(f32, f32, Action, Vec<PathBuf>),
    /// The drag started here is over: taken, or not.
    DragEnded(bool),
    /// The files of the clipboard (`None`: none).
    Pasted(Option<(Mode, Vec<PathBuf>)>),
}

/// The thread, for the interface.
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

/// The raw pointers of the window's `wl_display` and `wl_surface`, as numbers to cross threads.
#[derive(Debug, Clone, Copy)]
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

/// Starts the thread on the window's connection. The events come through the receiver.
pub fn start(handles: Handles) -> Option<(Link, stream::UnboundedReceiver<Event>)> {
    // SAFETY: `display` is the `wl_display` of the window that iced keeps open for the whole
    // life of the application (Evidence has one window, and quits with it). libwayland-client is
    // thread-safe: this connection reads into an event queue of its own.
    let backend = unsafe { Backend::from_foreign_display(handles.display as *mut _) };
    let conn = Connection::from_backend(backend);
    // SAFETY: `surface` is the window's live `wl_surface`, of the same display, kept by winit as
    // long as the window. It is only named in requests (`start_drag`), never destroyed here.
    let id =
        unsafe { ObjectId::from_ptr(WlSurface::interface(), handles.surface as *mut _) }.ok()?;
    let surface = WlSurface::from_id(&conn, id).ok()?;
    let (commands, received) = mpsc::channel();
    let (events, stream) = stream::unbounded();
    let spawned = thread::Builder::new()
        .name("evidence-dnd".into())
        .spawn(move || run(conn, surface, received, events));
    match spawned {
        Ok(_) => Some((Link { commands }, stream)),
        Err(err) => {
            tracing::warn!("no drag and drop: {err}");
            None
        }
    }
}

/// What a reading thread hands back.
enum Read {
    Dropped(WlDataOffer, Option<Vec<u8>>),
    Pasted(String, Option<Vec<u8>>),
}

/// The state of the thread.
struct Worker {
    events: stream::UnboundedSender<Event>,
    reads: mpsc::Sender<Read>,
    globals: Vec<(u32, String, u32)>,
    registry: WlRegistry,
    seat: Option<WlSeat>,
    device: Option<WlDataDevice>,
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    manager: Option<WlDataDeviceManager>,
    /// Serial of the last press of the pointer on the window, and of the last key.
    pressed: Option<u32>,
    keyed: Option<u32>,
    /// Types of the offers received, by offer.
    offers: Vec<(WlDataOffer, Vec<String>)>,
    /// The clipboard's offer.
    selection: Option<WlDataOffer>,
    /// The offer dragged over the window, with its serial and where it is.
    over: Option<Over>,
    preferred: Action,
    accept: bool,
    /// The drag started here: its source and icon.
    dragging: Option<(WlDataSource, Option<WlSurface>)>,
    /// Icons to destroy once the compositor has glided them back.
    lingering: Vec<(WlSurface, Instant)>,
}

struct Over {
    offer: WlDataOffer,
    serial: u32,
    at: (f32, f32),
    action: Option<Action>,
}

/// What a data source gives.
struct Given {
    mode: Mode,
    paths: Vec<PathBuf>,
    drag: bool,
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
        keyed: None,
        offers: Vec::new(),
        selection: None,
        over: None,
        preferred: Action::Move,
        accept: true,
        dragging: None,
        lingering: Vec::new(),
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
                // The interface is gone.
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        while let Ok(done) = read.try_recv() {
            worker.read_done(done);
        }
        let now = Instant::now();
        worker.lingering.retain(|(icon, until)| {
            let keep = *until > now;
            if !keep {
                icon.destroy();
            }
            keep
        });
        if conn.flush().is_err() {
            return;
        }
        if let Some(guard) = queue.prepare_read() {
            let fd = guard.connection_fd();
            if readable(fd.as_raw_fd(), TICK) {
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
        match command {
            Command::Drag(paths, items) => {
                // Drawn here, off the interface thread: a photo's thumbnail is decoded.
                let icon = crate::drag_image::build(&items, paths.len());
                self.drag(paths, icon, surface, qh);
            }
            Command::Clip(mode, paths) => {
                let (Some(manager), Some(device), Some(serial)) =
                    (&self.manager, &self.device, self.keyed)
                else {
                    return;
                };
                let source = manager.create_data_source(
                    qh,
                    Given {
                        mode,
                        paths,
                        drag: false,
                    },
                );
                transfer::OFFERED
                    .iter()
                    .for_each(|t| source.offer(t.to_string()));
                device.set_selection(Some(&source), serial);
            }
            Command::Paste => self.paste(),
            Command::Prefer(action) => {
                self.preferred = action;
                self.answer();
            }
            Command::Accept(accept) => {
                self.accept = accept;
                self.answer();
            }
        }
    }

    fn drag(
        &mut self,
        paths: Vec<PathBuf>,
        icon: Option<Icon>,
        surface: &WlSurface,
        qh: &QueueHandle<Self>,
    ) {
        let (Some(manager), Some(device), Some(serial)) =
            (&self.manager, &self.device, self.pressed)
        else {
            tracing::debug!("drag not started: no press seen");
            self.send(Event::DragEnded(false));
            return;
        };
        let source = manager.create_data_source(
            qh,
            Given {
                mode: Mode::Copy,
                paths,
                drag: true,
            },
        );
        transfer::OFFERED
            .iter()
            .for_each(|t| source.offer(t.to_string()));
        source.set_actions(DndAction::Copy | DndAction::Move);
        let icon = icon.and_then(|icon| self.icon_surface(&icon, qh));
        device.start_drag(Some(&source), surface, icon.as_ref(), serial);
        if let Some((old, old_icon)) = self.dragging.replace((source, icon)) {
            old.destroy();
            if let Some(old_icon) = old_icon {
                old_icon.destroy();
            }
        }
    }

    /// A surface showing `icon`, from a shared memory buffer.
    fn icon_surface(&self, icon: &Icon, qh: &QueueHandle<Self>) -> Option<WlSurface> {
        let (compositor, shm) = (self.compositor.as_ref()?, self.shm.as_ref()?);
        let stride = icon.width.checked_mul(4)?;
        let size = usize::try_from(stride.checked_mul(icon.height)?).ok()?;
        if icon.rgba.len() != size || size == 0 {
            return None;
        }
        // ARGB8888 is little-endian BGRA, premultiplied.
        let mut pixels = Vec::with_capacity(size);
        for p in icon.rgba.as_chunks::<4>().0 {
            let a = u16::from(p[3]);
            let pre = |c: u8| ((u16::from(c) * a + 127) / 255) as u8;
            pixels.extend_from_slice(&[pre(p[2]), pre(p[1]), pre(p[0]), p[3]]);
        }
        let mut file = memfd()?;
        file.write_all(&pixels).ok()?;
        let len = i32::try_from(size).ok()?;
        let pool = shm.create_pool(file.as_fd(), len, qh, ());
        let (w, h, s) = (
            i32::try_from(icon.width).ok()?,
            i32::try_from(icon.height).ok()?,
            i32::try_from(stride).ok()?,
        );
        let buffer = pool.create_buffer(0, w, h, s, Format::Argb8888, qh, ());
        pool.destroy();
        let surface = compositor.create_surface(qh, ());
        surface.set_buffer_scale(icon.scale.max(1));
        // The pointer at the hotspot: the icon's corner is that far from it.
        surface.attach(Some(&buffer), -icon.hotspot.0, -icon.hotspot.1);
        surface.damage(0, 0, w, h);
        surface.commit();
        Some(surface)
    }

    /// Accepts the offer dragged over the window, or refuses it, with the actions wanted.
    fn answer(&self) {
        let Some(over) = &self.over else {
            return;
        };
        let files = self
            .types_of(&over.offer)
            .iter()
            .any(|t| t == transfer::URI_LIST);
        if files && self.accept {
            over.offer
                .accept(over.serial, Some(transfer::URI_LIST.to_string()));
            let preferred = match self.preferred {
                Action::Copy => DndAction::Copy,
                Action::Move => DndAction::Move,
            };
            over.offer
                .set_actions(DndAction::Copy | DndAction::Move, preferred);
        } else {
            over.offer.accept(over.serial, None);
            over.offer
                .set_actions(DndAction::empty(), DndAction::empty());
        }
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

    fn paste(&mut self) {
        let Some(offer) = self.selection.clone() else {
            self.send(Event::Pasted(None));
            return;
        };
        let types = self.types_of(&offer);
        let mime = [transfer::GNOME_FILES, transfer::URI_LIST]
            .into_iter()
            .find(|t| types.iter().any(|o| o == t));
        let Some(mime) = mime else {
            self.send(Event::Pasted(None));
            return;
        };
        let reads = self.reads.clone();
        let (asked, mime) = (mime, mime.to_string());
        receive(&offer, asked, move |data| {
            let _ = reads.send(Read::Pasted(mime, data));
        });
    }

    fn read_done(&mut self, done: Read) {
        match done {
            Read::Pasted(mime, data) => {
                let files = data.and_then(|d| transfer::decode(&mime, &d));
                self.send(Event::Pasted(files));
            }
            Read::Dropped(offer, data) => {
                let files = data.and_then(|d| transfer::decode(transfer::URI_LIST, &d));
                let over = self.over.take().filter(|o| o.offer == offer);
                if let (Some((_, paths)), Some(over)) = (files, over) {
                    let action = over.action.unwrap_or(self.preferred);
                    // Version 3: the drop is done once read; the files are copied or moved
                    // by Evidence, off the interface thread.
                    if offer.version() >= 3 {
                        offer.finish();
                    }
                    self.send(Event::Dropped(over.at.0, over.at.1, action, paths));
                } else {
                    self.send(Event::Left);
                }
                self.forget(&offer);
            }
        }
    }
}

/// Reads `mime` from `offer` in a thread, at most `transfer::MAX_BYTES`, and hands the bytes
/// to `done` (`None` if it failed or went past the bound).
fn receive(offer: &WlDataOffer, mime: &str, done: impl FnOnce(Option<Vec<u8>>) + Send + 'static) {
    let Ok((reader, writer)) = std::io::pipe() else {
        done(None);
        return;
    };
    offer.receive(mime.to_string(), writer.as_fd());
    drop(writer);
    let spawned = thread::Builder::new()
        .name("evidence-receive".into())
        .spawn(move || done(read_bounded(OwnedFd::from(reader))));
    if let Err(err) = spawned {
        tracing::warn!("list of files not read: {err}");
    }
}

/// Up to `transfer::MAX_BYTES` from `fd`, each piece awaited `PATIENCE` at most.
fn read_bounded(fd: OwnedFd) -> Option<Vec<u8>> {
    let mut file = File::from(fd);
    let mut data = Vec::new();
    let mut piece = [0; 16 << 10];
    loop {
        if !readable(file.as_raw_fd(), PATIENCE) {
            return None;
        }
        match file.read(&mut piece) {
            Ok(0) => return Some(data),
            Ok(n) if data.len() + n <= transfer::MAX_BYTES => data.extend_from_slice(&piece[..n]),
            _ => return None,
        }
    }
}

/// An anonymous file in memory, for the icon's pixels.
fn memfd() -> Option<File> {
    // SAFETY: a constant name ending with NUL, and flags of the man page.
    let fd = unsafe { libc::memfd_create(c"evidence-drag-icon".as_ptr(), libc::MFD_CLOEXEC) };
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
        {
            if caps.contains(wl_seat::Capability::Pointer) {
                seat.get_pointer(qh, ());
            }
            if caps.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
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

impl Dispatch<WlKeyboard, ()> for Worker {
    fn event(
        worker: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { serial, .. } | wl_keyboard::Event::Key { serial, .. } => {
                worker.keyed = Some(serial);
            }
            // The keymap is winit's business: its file is closed here.
            wl_keyboard::Event::Keymap { fd, .. } => drop(fd),
            _ => {}
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
            wl_data_device::Event::Selection { id } => {
                if let Some(old) = std::mem::replace(&mut worker.selection, id)
                    && worker.over.as_ref().is_none_or(|o| o.offer != old)
                {
                    worker.forget(&old);
                }
            }
            wl_data_device::Event::Enter {
                serial, x, y, id, ..
            } => {
                let Some(offer) = id else {
                    return;
                };
                let at = (x as f32, y as f32);
                worker.over = Some(Over {
                    offer,
                    serial,
                    at,
                    action: None,
                });
                worker.accept = true;
                worker.answer();
                worker.send(Event::Over(at.0, at.1));
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(over) = worker.over.as_mut() {
                    over.at = (x as f32, y as f32);
                    let at = over.at;
                    worker.send(Event::Over(at.0, at.1));
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(over) = worker.over.take() {
                    worker.forget(&over.offer);
                }
                worker.send(Event::Left);
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = worker.over.as_ref().map(|o| o.offer.clone()) else {
                    return;
                };
                let reads = worker.reads.clone();
                let read = offer.clone();
                receive(&offer, transfer::URI_LIST, move |data| {
                    let _ = reads.send(Read::Dropped(read, data));
                });
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
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                if let Some(types) = worker.offers.iter_mut().find(|o| o.0 == *offer) {
                    // Bounded: a client offering thousands of types gets the first ones.
                    if types.1.len() < 64 {
                        types.1.push(mime_type);
                    }
                }
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(action),
            } => {
                if let Some(over) = worker.over.as_mut().filter(|o| o.offer == *offer) {
                    over.action = match action {
                        a if a.contains(DndAction::Copy) => Some(Action::Copy),
                        a if a.contains(DndAction::Move) => Some(Action::Move),
                        _ => None,
                    };
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<WlDataSource, Given> for Worker {
    fn event(
        worker: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        given: &Given,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                let Some(data) = transfer::encode(&mime_type, given.mode, &given.paths) else {
                    return;
                };
                // Written off this thread: the reader may be slow, or never read.
                let _ = thread::Builder::new()
                    .name("evidence-send".into())
                    .spawn(move || {
                        let _ = File::from(fd).write_all(&data);
                    });
            }
            wl_data_source::Event::Cancelled if given.drag => {
                worker.drag_ended(source, false);
            }
            wl_data_source::Event::Cancelled => source.destroy(),
            wl_data_source::Event::DndFinished => worker.drag_ended(source, true),
            _ => {}
        }
    }
}

impl Worker {
    fn drag_ended(&mut self, source: &WlDataSource, taken: bool) {
        source.destroy();
        let Some((dragged, icon)) = self.dragging.take() else {
            return;
        };
        if dragged != *source {
            self.dragging = Some((dragged, icon));
            return;
        }
        if let Some(icon) = icon {
            match taken {
                true => icon.destroy(),
                // The compositor glides it back while it lives.
                false => self.lingering.push((icon, Instant::now() + ICON_LINGER)),
            }
        }
        self.send(Event::DragEnded(taken));
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
wayland_client::delegate_noop!(Worker: ignore WlShm);
wayland_client::delegate_noop!(Worker: ignore WlShmPool);
wayland_client::delegate_noop!(Worker: ignore WlDataDeviceManager);
