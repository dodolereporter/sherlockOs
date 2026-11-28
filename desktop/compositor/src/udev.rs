//! Real session: the compositor started from a text console or by the login screen.
//!
//! - **Session**: libseat (seatd or logind) opens the devices without root and tells when the
//!   console is left or entered again (Ctrl + Alt + Fn).
//! - **Card**: the one `card::choose` picks from `/sys` renders, and drives its screens. Another
//!   card (the NVIDIA card a hybrid laptop wires its HDMI port to) is opened only while a screen
//!   is plugged into it: its screens are drawn by the same renderer, into buffers it imports
//!   (PRIME), and it is closed when the last one goes, to sleep again (lot D-16, ADR 0037).
//!   udev reports screens plugged in and out.
//! - **Screens**: every connector with a screen, each driven by a `DrmCompositor` at its
//!   preferred mode, drawn by the same GLES scene as the windowed backend (`render::Scene`).
//! - **Input**: libinput (keyboard, mouse, touchpad with its setting and gestures, lot D-31).
//! - **Frames** (lot P-01): a screen draws only when something changed (`pacing`), at the pace
//!   of its vertical blank, and redraws only what changed (`damage`).
//!
//! See "Real session" in `docs/compositor.md`.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use desktop_config::displays::Identity;
use sherlock_ui::color;
use smithay::{
    backend::{
        allocator::{
            Fourcc, Modifier,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmDeviceNotifier, DrmEvent,
            compositor::{DrmCompositor, FrameFlags},
            exporter::gbm::GbmFramebufferExporter,
        },
        egl::{EGLContext, EGLDisplay},
        input::{Event, InputEvent, KeyState, PointerMotionEvent},
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma,
            element::{memory::MemoryRenderBufferRenderElement, render_elements},
            gles::GlesRenderer,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent},
    },
    output::{Mode, Output, OutputModeSource, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, LoopHandle, RegistrationToken,
            ping::make_ping,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{self, Device as _, ModeTypeFlags, connector, crtc, property},
        input::Libinput,
        rustix::fs::{Dev, OFlags},
    },
    utils::{DeviceFd, Logical, Point, Rectangle, Transform},
};

use crate::{
    Error, HIDDEN_FRAME_INTERVAL, State,
    anim::PreviewMotion,
    card,
    cursor::{Cursors, Next as CursorNext},
    damage::Ids,
    dmabuf,
    options::Options,
    output::{Details, View},
    pacing::{Pacing, Phase},
    perf,
    render::{Scene, SceneElement},
    wallpaper::{Textures, Wallpaper},
};

/// Time between two vertical blanks of `mode`.
fn period(mode: control::Mode) -> Duration {
    match mode.vrefresh() {
        0 => DEFAULT_PERIOD,
        hz => Duration::from_secs(1) / hz,
    }
}

/// Only the cursor may leave the composition, for the cursor plane: a client buffer on a plane
/// of its own would lose its rounded clip.
const FRAME_FLAGS: FrameFlags = FrameFlags::ALLOW_CURSOR_PLANE_SCANOUT;

/// Frame period of a screen whose mode gives no refresh rate.
const DEFAULT_PERIOD: Duration = Duration::from_micros(16_667);

/// The pointer moved by `delta` from `from`: kept on the screens (`areas`, side by side).
pub fn clamp_pointer(
    from: Point<f64, Logical>,
    delta: Point<f64, Logical>,
    areas: &[Rectangle<i32, Logical>],
) -> Point<f64, Logical> {
    let to = from + delta;
    let Some(bounds) = areas.iter().copied().reduce(|a, b| a.merge(b)) else {
        return from;
    };
    let x = to.x.clamp(
        f64::from(bounds.loc.x),
        f64::from(bounds.loc.x + bounds.size.w) - 1.0,
    );
    // The screen under that column, whose height may be smaller than the others'.
    let area = areas
        .iter()
        .find(|a| x >= f64::from(a.loc.x) && x < f64::from(a.loc.x + a.size.w))
        .unwrap_or(&bounds);
    let y = to.y.clamp(
        f64::from(area.loc.y),
        f64::from(area.loc.y + area.size.h) - 1.0,
    );
    (x, y).into()
}

render_elements! {
    /// A frame of the real session: the cursor in front of the scene.
    SessionElement<=GlesRenderer>;
    Cursor=MemoryRenderBufferRenderElement<GlesRenderer>,
    Scene=SceneElement,
}

type Compositor =
    DrmCompositor<GbmAllocator<DrmDeviceFd>, GbmFramebufferExporter<DrmDeviceFd>, (), DrmDeviceFd>;

/// A screen's CRTC, on its card: CRTC numbers start again on each card.
type Key = (Dev, crtc::Handle);

/// A card besides the one that renders, opened while a screen is plugged into it (lot D-16):
/// its screens show buffers the renderer drew on the other card, imported into it.
struct Secondary {
    device: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    /// The descriptor the seat gave, kept to give it back when the card closes: until then, the
    /// seat's own descriptor keeps the card awake.
    fd: DeviceFd,
    /// `card1`, for `/sys`.
    name: String,
    /// Its vertical blanks, in the loop.
    events: RegistrationToken,
}

/// A connector with a screen, driven by a CRTC.
struct Head {
    output: Output,
    /// The card it is plugged into.
    card: Dev,
    connector: connector::Handle,
    compositor: Compositor,
    /// When the next frame is due.
    pacing: Pacing,
    /// Identities of the elements the compositor draws, for the damage tracker of this screen.
    ids: Ids,
    /// Time between two vertical blanks.
    period: Duration,
    /// The frame waiting for the vertical blank was built while the session was locked: once
    /// shown, the lock can be confirmed (lot D-25a).
    lock_queued: bool,
    /// The modes of the screen, the preferred one first (lot D-26b).
    modes: Vec<control::Mode>,
    /// Switched off by a configuration: nothing drawn (lot D-26b).
    off: bool,
    /// The screen is off (DPMS), the screens being dark (lot D-38).
    dark: bool,
    /// Frames that failed before the first one was shown: on another card, a few of them and
    /// the card is given up, so that it does not stay awake for a black screen (lot D-16).
    misses: u32,
    shown: bool,
}

/// What a change of another card calls for (lot D-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    /// Closed and nothing plugged in, or given up: it stays closed, asleep.
    Leave,
    /// Closed, a screen plugged in: open it.
    Open,
    /// Open: its screens are read again, and it closes if none is left.
    Rescan,
}

fn decide(open: bool, plugged: bool, refused: bool) -> Decision {
    match (open, plugged, refused) {
        (true, ..) => Decision::Rescan,
        (false, true, false) => Decision::Open,
        _ => Decision::Leave,
    }
}

/// Failed frames, before any was shown, after which another card is given up.
const SECONDARY_MISSES: u32 = 3;

/// Fields are dropped in this order: the screens and the card (which gives the console its
/// display state back) before the seat.
struct Backend {
    heads: HashMap<Key, Head>,
    /// The other cards opened, while a screen is plugged into them (lot D-16).
    others: HashMap<Dev, Secondary>,
    scene: Scene,
    textures: Textures,
    cursors: Cursors,
    renderer: GlesRenderer,
    device: DrmDevice,
    gbm: GbmDevice<DrmDeviceFd>,
    libinput: Libinput,
    session: LibSeatSession,
    /// Device number of the card, as udev names it.
    device_id: Dev,
    /// The other cards of the seat and their node, opened or not.
    known: HashMap<Dev, PathBuf>,
    /// Cards whose screen could not be driven: not opened again until it is unplugged.
    refused: HashSet<Dev>,
    /// Other cards may be opened (`card::secondary_allowed`).
    secondary: bool,
    preview: PreviewMotion,
    options: Options,
    /// Frames that failed in a row: only some of them are logged.
    failures: u64,
    /// The backend itself, for the timers it starts.
    me: Weak<RefCell<Backend>>,
    handle: LoopHandle<'static, State>,
}

/// The real session, to close when leaving the loop.
pub struct RealSession {
    backend: Rc<RefCell<Backend>>,
    handle: LoopHandle<'static, State>,
    sources: Vec<RegistrationToken>,
}

impl RealSession {
    /// Releases the screens and the card, then the seat: the console gets back its display
    /// state while the compositor is still allowed to set it. Smithay's loop is never freed
    /// (its X11 window manager keeps a handle to it), so its sources are removed here.
    pub fn close(self, state: &mut State) {
        for token in self.sources {
            self.handle.remove(token);
        }
        state.switch_vt = None;
        state.set_redraw_ping(None);
        match Rc::try_unwrap(self.backend) {
            Ok(backend) => drop(backend.into_inner()),
            Err(backend) => {
                tracing::warn!("the real session is still in use: its screens are released");
                backend.borrow_mut().heads.clear();
            }
        }
    }
}

/// Opens the session and the card, creates one screen per connector with a screen, and plugs
/// input, screen changes, frames and console switches into the loop. Fails if no screen can be
/// driven.
pub fn init(
    event_loop: &EventLoop<'static, State>,
    state: &mut State,
    options: &Options,
) -> Result<RealSession, Error> {
    let fail = |what: &str, err: &dyn std::fmt::Display| Error::Session(format!("{what}: {err}"));
    let (session, notifier) = LibSeatSession::new().map_err(|e| fail("libseat", &e))?;
    let path = card::primary().map_err(Error::Session)?;
    let udev = UdevBackend::new(session.seat()).map_err(|e| fail("udev", &e))?;
    let device_id = udev
        .device_list()
        .find(|(_, p)| *p == path)
        .map(|(id, _)| id)
        .ok_or_else(|| {
            Error::Session(format!(
                "{} is not on seat {}",
                path.display(),
                session.seat()
            ))
        })?;
    tracing::info!(card = %path.display(), seat = session.seat(), "real session");
    // The other cards of the seat: opened only once a screen is plugged into them (lot D-16).
    let known: HashMap<Dev, PathBuf> = udev
        .device_list()
        .filter(|(id, _)| *id != device_id)
        .map(|(id, path)| (id, path.to_owned()))
        .collect();

    let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK;
    let mut opener = session.clone();
    let fd = opener
        .open(&path, flags)
        .map_err(|e| fail("opening the card", &e))?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (device, drm_events) = DrmDevice::new(fd.clone(), true).map_err(|e| fail("DRM", &e))?;
    let gbm = GbmDevice::new(fd).map_err(|e| fail("GBM", &e))?;
    // SAFETY: the GBM device lives as long as the display, which keeps a clone of it.
    let egl = unsafe { EGLDisplay::new(gbm.clone()) }.map_err(|e| fail("EGL", &e))?;
    let context = EGLContext::new(&egl).map_err(|e| fail("EGL context", &e))?;
    // SAFETY: the context is current on this thread only, the renderer's.
    let mut renderer = unsafe { GlesRenderer::new(context) }?;
    let scene = Scene::new(&mut renderer)?;

    let mut libinput = Libinput::new_with_udev(LibinputSessionInterface::from(session.clone()));
    if libinput.udev_assign_seat(&session.seat()).is_err() {
        return Err(Error::Session(
            "libinput: cannot read the seat's devices".into(),
        ));
    }

    let var = |key: &str| std::env::var_os(key);
    state.wallpaper = Wallpaper::start(
        &event_loop.handle(),
        desktop_config::wallpaper::config_file(var),
        desktop_config::wallpaper::dirs(var),
    )?;
    state.watch_pointer(
        &event_loop.handle(),
        desktop_config::pointer::config_file(var),
    )?;
    state.watch_touchpad(
        &event_loop.handle(),
        desktop_config::touchpad::config_file(var),
    )?;
    // The layout and the repeat of Control Room's keyboard page (lot D-26c).
    state.watch_keyboard(
        &event_loop.handle(),
        desktop_config::keyboard::config_file(var),
    )?;
    // The lock after inactivity, and the lock of logind and before the sleep (lot D-25a3). The
    // login screen has no session to lock (`greeter.rs`): its inactivity only turns the screens
    // off, after the veil.
    state.watch_idle(&event_loop.handle(), desktop_config::lock::config_file(var))?;
    // What is copied, for Super + V, and its setting (lot D-42).
    state.watch_clipboard_settings(
        &event_loop.handle(),
        desktop_config::clipboard::config_file(var),
    )?;
    if !state.is_greeter() {
        state.watch_logind(&event_loop.handle(), None)?;
    }
    // The screens off after inactivity (lot D-38).
    state.watch_power(
        &event_loop.handle(),
        desktop_config::power::config_file(var),
    )?;
    // What keeps the session awake: videos, the bus, logind (lot D-43).
    state.watch_inhibitors(&event_loop.handle(), None, None)?;
    // The media keys, through the machine's sound, logind and players (lot D-35).
    state.enable_media_keys(crate::media_keys::Controls::system());
    let mut switcher = session.clone();
    state.switch_vt = Some(Box::new(move |vt| {
        if let Err(err) = switcher.change_vt(vt) {
            tracing::warn!(vt, "switching console: {err}");
        }
    }));

    let backend = Rc::new(RefCell::new(Backend {
        heads: HashMap::new(),
        others: HashMap::new(),
        scene,
        textures: Textures::default(),
        cursors: Cursors::load(state.theme()),
        renderer,
        device,
        gbm,
        libinput: libinput.clone(),
        session,
        device_id,
        known,
        refused: HashSet::new(),
        secondary: card::secondary_allowed(var),
        preview: PreviewMotion::default(),
        options: options.clone(),
        failures: 0,
        me: Weak::new(),
        handle: event_loop.handle(),
    }));
    backend.borrow_mut().me = Rc::downgrade(&backend);
    enable_dmabuf(state, &backend);
    // The last image of a window that closes (lot D-34): drawn while the clients are read,
    // never during a frame, since the loop runs one source at a time.
    let weak = Rc::downgrade(&backend);
    state.set_snapshotter(Box::new(move |state, window| {
        let backend = weak.upgrade()?;
        let mut backend = backend.try_borrow_mut().ok()?;
        let Backend {
            scene, renderer, ..
        } = &mut *backend;
        crate::window_anim::snapshot(scene, renderer, state, window)
    }));
    // The last frame of each screen when the session locks, frozen under the curtain (lot
    // D-24e), drawn the same way.
    let weak = Rc::downgrade(&backend);
    state.set_freezer(Box::new(move |state, output| {
        let backend = weak.upgrade()?;
        let mut backend = backend.try_borrow_mut().ok()?;
        let Backend {
            scene,
            renderer,
            textures,
            ..
        } = &mut *backend;
        let now = state.now();
        crate::capture::screen_texture(scene, renderer, textures, state, output, now)
    }));
    // The last image of a menu of the shell that closes (lot UI-06), the same way.
    let weak = Rc::downgrade(&backend);
    state.set_popup_snapshotter(Box::new(move |state, surface, size, scale| {
        let backend = weak.upgrade()?;
        let mut backend = backend.try_borrow_mut().ok()?;
        let Backend {
            scene, renderer, ..
        } = &mut *backend;
        crate::popup_anim::snapshot(scene, renderer, state, surface, size, scale)
    }));
    // Screenshots (lot D-35c), drawn the same way, between two frames. Not of the login
    // screen (`greeter.rs`).
    if !state.is_greeter() {
        state.enable_screenshots(crate::capture::Settings::default());
    }
    let weak = Rc::downgrade(&backend);
    state.set_capturer(Box::new(move |state, target, cursor| {
        let backend = weak.upgrade()?;
        let mut backend = backend.try_borrow_mut().ok()?;
        let Backend {
            scene,
            renderer,
            textures,
            cursors,
            ..
        } = &mut *backend;
        let cursors = cursor.then_some(cursors);
        crate::capture::image(scene, renderer, textures, state, target, cursors)
    }));
    // The screens' saved settings, and the hardware's part of a configuration (ADR 0025).
    state.remember_screens(desktop_config::displays::config_file(var));
    // Do not disturb for the time of a projection, Super + P (lot D-39).
    state.set_projection_notifications(notifications_config::path());
    let weak = Rc::downgrade(&backend);
    state.set_screen_driver(Box::new(move |output, wanted| {
        let backend = weak.upgrade().ok_or("the session is closed")?;
        let mut backend = backend.try_borrow_mut().map_err(|e| e.to_string())?;
        backend.drive(output, wanted)
    }));
    backend.borrow_mut().scan(state);
    if backend.borrow().heads.is_empty() {
        // The card's screens were switched off when it was opened: better leave than stay dark.
        return Err(Error::Session(format!(
            "no screen driven on {}: none plugged in, or none could be lit (see the log)",
            path.display()
        )));
    }
    let handle = event_loop.handle();
    let mut sources = insert_sources(&handle, &backend, notifier, drm_events, udev)?;
    // A screen already plugged into another card at start (lot D-16).
    let others: Vec<Dev> = backend.borrow().known.keys().copied().collect();
    for card in others {
        backend.borrow_mut().consider(state, card);
    }
    let input = LibinputInputBackend::new(libinput);
    // When libinput's events were last read (lot D-25g).
    let read = Rc::new(Cell::new(Instant::now()));
    let seen = read.clone();
    let token = handle
        .insert_source(input, move |event, _, state| {
            seen.set(Instant::now());
            let since = state.perf.start();
            input_event(event, state);
            state.perf.stop(perf::Phase::Input, since);
        })
        .map_err(|e| e.error)?;
    sources.push(token);
    sources.push(watch_input(&handle, &backend, read)?);
    // Every change that may show wakes the screens up (`State::queue_redraw`).
    let (ping, wake) = make_ping().map_err(|e| fail("frame scheduling", &e))?;
    let b = backend.clone();
    let token = handle
        .insert_source(wake, move |_, _, state| b.borrow_mut().render_all(state))
        .map_err(|e| e.error)?;
    sources.push(token);
    state.set_redraw_ping(Some(ping));
    Ok(RealSession {
        backend,
        handle,
        sources,
    })
}

/// Offers linux-dmabuf (lot D-06) on the render node of the card the session drives, with the
/// formats its renderer imports. Without it, clients keep `wl_shm`.
fn enable_dmabuf(state: &mut State, backend: &Rc<RefCell<Backend>>) {
    let (node, formats) = {
        let backend = backend.borrow();
        match dmabuf::render_node(&backend.renderer) {
            Ok(node) => (node, backend.renderer.dmabuf_formats()),
            Err(err) => {
                tracing::warn!("no linux-dmabuf, GPU clients will draw in software: {err}");
                return;
            }
        }
    };
    tracing::info!(node = %node.path.display(), "linux-dmabuf offered");
    let backend = Rc::clone(backend);
    let import: dmabuf::Importer = Box::new(move |buffer| {
        // Called while the clients are read, never during a frame: the loop runs one source
        // at a time.
        let mut backend = backend.try_borrow_mut()?;
        backend.renderer.import_dmabuf(buffer, None)?;
        Ok(())
    });
    if let Err(err) = state.enable_dmabuf(node.dev, formats, import) {
        tracing::warn!("no linux-dmabuf, the feedback table cannot be created: {err}");
    }
}

fn insert_sources(
    handle: &LoopHandle<'static, State>,
    backend: &Rc<RefCell<Backend>>,
    notifier: smithay::backend::session::libseat::LibSeatSessionNotifier,
    drm_events: smithay::backend::drm::DrmDeviceNotifier,
    udev: UdevBackend,
) -> Result<Vec<RegistrationToken>, Error> {
    let b = backend.clone();
    let session = handle.insert_source(notifier, move |event, _, state| {
        let since = state.perf.start();
        match event {
            SessionEvent::PauseSession => b.borrow_mut().pause(state),
            SessionEvent::ActivateSession => b.borrow_mut().activate(state),
        }
        state.perf.stop(perf::Phase::Screens, since);
    });
    let card = backend.borrow().device_id;
    let drm = vblanks(handle, Rc::downgrade(backend), card, drm_events);
    let b = backend.clone();
    let udev = handle.insert_source(udev, move |event, _, state| {
        let primary = b.borrow().device_id;
        let since = state.perf.start();
        match event {
            UdevEvent::Changed { device_id } if device_id == primary => b.borrow_mut().scan(state),
            UdevEvent::Removed { device_id } if device_id == primary => {
                tracing::error!("the card is gone");
                state.loop_signal.stop();
            }
            // Another card: opened while a screen is plugged into it (lot D-16).
            UdevEvent::Added { device_id, path } => {
                b.borrow_mut().known.insert(device_id, path);
                b.borrow_mut().consider(state, device_id);
            }
            UdevEvent::Changed { device_id } => b.borrow_mut().consider(state, device_id),
            UdevEvent::Removed { device_id } => {
                let mut backend = b.borrow_mut();
                backend.known.remove(&device_id);
                backend.close_secondary(state, device_id);
            }
        }
        state.perf.stop(perf::Phase::Screens, since);
    });
    // Windows out of view get their frame callbacks even when no screen draws.
    let interval = HIDDEN_FRAME_INTERVAL;
    let timer = handle.insert_source(Timer::from_duration(interval), move |_, _, state| {
        send_hidden_frames(state);
        TimeoutAction::ToDuration(interval)
    });
    Ok(vec![
        session.map_err(|e| e.error)?,
        drm?,
        udev.map_err(|e| e.error)?,
        timer.map_err(|e| e.error)?,
    ])
}

/// The vertical blanks of the card `card`, in the loop: each one ends the frame of its screen.
fn vblanks(
    handle: &LoopHandle<'static, State>,
    backend: Weak<RefCell<Backend>>,
    card: Dev,
    events: DrmDeviceNotifier,
) -> Result<RegistrationToken, Error> {
    handle
        .insert_source(events, move |event, _, state| match event {
            DrmEvent::VBlank(crtc) => {
                if let Some(backend) = backend.upgrade() {
                    backend.borrow_mut().frame_done(state, (card, crtc));
                }
            }
            DrmEvent::Error(err) => tracing::warn!("DRM: {err}"),
        })
        .map_err(|e| Error::Session(format!("DRM events: {}", e.error)))
}

/// How often the input is checked on (lot D-25g).
const INPUT_WATCH: Duration = Duration::from_secs(2);
/// Events waiting that long, the source of libinput is not read any more.
pub const INPUT_STALL: Duration = Duration::from_secs(5);

/// The input is stalled: libinput has events waiting (`readable`) that nothing read for
/// `quiet`. The loop reads a readable source at its next turn, within milliseconds: waiting
/// seconds means it no longer does.
pub fn input_stalled(quiet: Duration, readable: bool) -> bool {
    readable && quiet >= INPUT_STALL
}

/// Watches the input (lot D-25g): the screen kept drawing while no key nor click came, and
/// even the console switch did not work, twice while locked. When libinput has events nobody
/// reads, it is logged with the state of the session, then libinput is suspended and resumed,
/// which opens the devices again.
fn watch_input(
    handle: &LoopHandle<'static, State>,
    backend: &Rc<RefCell<Backend>>,
    read: Rc<Cell<Instant>>,
) -> Result<RegistrationToken, Error> {
    let b = backend.clone();
    let timer = Timer::from_duration(INPUT_WATCH);
    let token = handle.insert_source(timer, move |_, _, state| {
        let mut backend = b.borrow_mut();
        let quiet = read.get().elapsed();
        let mut fds = [smithay::reexports::rustix::event::PollFd::new(
            &backend.libinput,
            smithay::reexports::rustix::event::PollFlags::IN,
        )];
        let zero = smithay::reexports::rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let readable =
            smithay::reexports::rustix::event::poll(&mut fds, Some(&zero)).is_ok_and(|n| n > 0);
        if input_stalled(quiet, readable) {
            tracing::warn!(
                ?quiet,
                locked = state.is_locked(),
                active = backend.session.is_active(),
                "input stalled: libinput has events nobody reads; libinput suspended and resumed"
            );
            backend.libinput.suspend();
            if backend.libinput.resume().is_err() {
                tracing::warn!("libinput did not resume");
            }
            read.set(Instant::now());
        }
        TimeoutAction::ToDuration(INPUT_WATCH)
    });
    token.map_err(|e| Error::Session(format!("input watch: {}", e.error)))
}

fn input_event(event: InputEvent<LibinputInputBackend>, state: &mut State) {
    match event {
        // Touchpads take the touchpad setting (lot D-31).
        InputEvent::DeviceAdded { device } => state.input_device_added(device),
        InputEvent::DeviceRemoved { device } => state.input_device_removed(&device),
        // The mouse and the touchpad move the pointer by steps, kept on the screens.
        InputEvent::PointerMotion { event } => {
            let Some(pointer) = state.seat.get_pointer() else {
                return;
            };
            let areas: Vec<_> = state
                .space
                .outputs()
                .filter_map(|o| state.space.output_geometry(o))
                .collect();
            let to = clamp_pointer(pointer.current_location(), event.delta(), &areas);
            state.pointer_motion(to, event.time_msec());
        }
        event => state.process_input_event(event),
    }
}

impl Backend {
    /// Adds the screens plugged in, removes those unplugged, on every card open.
    fn scan(&mut self, state: &mut State) {
        if !self.session.is_active() {
            return;
        }
        let cards: Vec<Dev> = std::iter::once(self.device_id)
            .chain(self.others.keys().copied())
            .collect();
        for card in cards {
            self.scan_card(state, card);
        }
        // A card whose last screen went is closed, to sleep again (lot D-16).
        let idle: Vec<Dev> = self
            .others
            .keys()
            .filter(|card| !self.heads.values().any(|h| h.card == **card))
            .copied()
            .collect();
        for card in idle {
            self.close_secondary(state, card);
        }
        // The curtain, read once before the first frame (lot D-24e).
        state.ensure_curtain();
        self.render_all(state);
    }

    /// The DRM device of `card`, the one that renders or another one open.
    fn device(&self, card: Dev) -> Option<&DrmDevice> {
        if card == self.device_id {
            Some(&self.device)
        } else {
            self.others.get(&card).map(|o| &o.device)
        }
    }

    fn device_mut(&mut self, card: Dev) -> Option<&mut DrmDevice> {
        if card == self.device_id {
            Some(&mut self.device)
        } else {
            self.others.get_mut(&card).map(|o| &mut o.device)
        }
    }

    /// Adds the screens plugged into `card`, removes those unplugged.
    fn scan_card(&mut self, state: &mut State, card: Dev) {
        let Some(device) = self.device(card) else {
            return;
        };
        let Ok(resources) = device.resource_handles() else {
            tracing::warn!("cannot read the card's connectors");
            return;
        };
        let connected: Vec<connector::Info> = resources
            .connectors()
            .iter()
            .filter_map(|c| device.get_connector(*c, true).ok())
            .filter(|i| i.state() == connector::State::Connected && !i.modes().is_empty())
            .collect();
        let gone: Vec<Key> = self
            .heads
            .iter()
            .filter(|(_, h)| h.card == card)
            .filter(|(_, h)| !connected.iter().any(|i| i.handle() == h.connector))
            .map(|(key, _)| *key)
            .collect();
        for key in gone {
            if let Some(head) = self.heads.remove(&key) {
                tracing::info!(screen = head.output.name(), "screen unplugged");
                state.remove_screen(&head.output);
            }
        }
        for info in connected {
            if self
                .heads
                .values()
                .any(|h| h.card == card && h.connector == info.handle())
            {
                continue;
            }
            let Some(device) = self.device(card) else {
                return;
            };
            // A CRTC that one of its encoders can use, and that no screen drives yet.
            let crtc = info
                .encoders()
                .iter()
                .filter_map(|e| device.get_encoder(*e).ok())
                .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
                .find(|crtc| !self.heads.contains_key(&(card, *crtc)));
            let Some(crtc) = crtc else {
                tracing::warn!(connector = ?info.handle(), "no CRTC left for this screen");
                continue;
            };
            if let Err(err) = self.add_head(state, card, &info, crtc) {
                tracing::warn!(connector = ?info.handle(), "screen not driven: {err}");
            }
        }
    }

    /// Another card changed, appeared, or is there at start (lot D-16): opened when a screen
    /// is plugged into it, scanned when open, closed when its last screen goes. Whether a
    /// screen is plugged in is read from `/sys`, which never wakes it.
    fn consider(&mut self, state: &mut State, card: Dev) {
        if !self.secondary || !self.session.is_active() {
            return;
        }
        let Some(path) = self.known.get(&card).cloned() else {
            return;
        };
        let Some(name) = card::name_of(&path) else {
            return;
        };
        let drm = Path::new("/sys/class/drm");
        let plugged = card::has_screen(drm, &name);
        if !plugged {
            self.refused.remove(&card);
        }
        match decide(
            self.others.contains_key(&card),
            plugged,
            self.refused.contains(&card),
        ) {
            Decision::Leave => return,
            Decision::Rescan => {
                self.scan_card(state, card);
                if !self.heads.values().any(|h| h.card == card) {
                    self.close_secondary(state, card);
                }
                self.render_all(state);
                return;
            }
            Decision::Open => {}
        }
        tracing::info!(
            card = name,
            power = card::runtime_status(drm, &name).as_deref().unwrap_or("?"),
            "a screen is plugged into another card: opening it"
        );
        if let Err(err) = self.open_secondary(card, &path, &name) {
            tracing::warn!(card = name, "the other card is not opened: {err}");
            self.refused.insert(card);
            return;
        }
        self.scan_card(state, card);
        if !self.heads.values().any(|h| h.card == card) {
            tracing::warn!(card = name, "no screen of the other card could be driven");
            self.refused.insert(card);
            self.close_secondary(state, card);
        }
        state.ensure_curtain();
        self.render_all(state);
    }

    fn open_secondary(&mut self, card: Dev, path: &Path, name: &str) -> Result<(), String> {
        let flags = OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK;
        let fd = self.session.open(path, flags).map_err(|e| e.to_string())?;
        let fd = DeviceFd::from(fd);
        let opened = (|| {
            let drm_fd = DrmDeviceFd::new(fd.clone());
            let (device, events) =
                DrmDevice::new(drm_fd.clone(), true).map_err(|e| e.to_string())?;
            let gbm = GbmDevice::new(drm_fd).map_err(|e| e.to_string())?;
            let events =
                vblanks(&self.handle, self.me.clone(), card, events).map_err(|e| e.to_string())?;
            Ok::<_, String>((device, gbm, events))
        })();
        match opened {
            Ok((device, gbm, events)) => {
                self.others.insert(
                    card,
                    Secondary {
                        device,
                        gbm,
                        fd,
                        name: name.to_owned(),
                        events,
                    },
                );
                Ok(())
            }
            Err(err) => {
                self.give_back(fd, name);
                Err(err)
            }
        }
    }

    /// Closes the other card `card`: its screens go, its descriptor goes back to the seat, and
    /// the card may sleep. Its power state is logged a little later, once the kernel had the
    /// time to put it to sleep.
    fn close_secondary(&mut self, state: &mut State, card: Dev) {
        let keys: Vec<Key> = self.heads.keys().filter(|k| k.0 == card).copied().collect();
        for key in keys {
            if let Some(head) = self.heads.remove(&key) {
                tracing::info!(screen = head.output.name(), "screen of the other card gone");
                state.remove_screen(&head.output);
            }
        }
        let Some(secondary) = self.others.remove(&card) else {
            return;
        };
        let Secondary {
            device,
            gbm,
            fd,
            name,
            events,
        } = secondary;
        self.handle.remove(events);
        drop(gbm);
        drop(device);
        self.give_back(fd, &name);
        let timer = Timer::from_duration(Duration::from_secs(15));
        let inserted = self.handle.insert_source(timer, move |_, _, _| {
            let power = card::runtime_status(Path::new("/sys/class/drm"), &name);
            tracing::info!(
                card = name,
                power = power.as_deref().unwrap_or("?"),
                "the other card, after closing"
            );
            TimeoutAction::Drop
        });
        if let Err(err) = inserted {
            tracing::debug!("power timer: {}", err.error);
        }
    }

    /// Gives a card's descriptor back to the seat. Held by anything else still, it is not: the
    /// card would stay awake, which the log says.
    fn give_back(&mut self, fd: DeviceFd, name: &str) {
        match TryInto::<std::os::fd::OwnedFd>::try_into(fd) {
            Ok(owned) => {
                if let Err(err) = self.session.close(owned) {
                    tracing::warn!(card = name, "giving the card back to the seat: {err}");
                } else {
                    tracing::info!(card = name, "the other card is closed");
                }
            }
            Err(_) => tracing::error!(
                card = name,
                "the other card is still held: it may stay awake until the session ends"
            ),
        }
    }

    fn add_head(
        &mut self,
        state: &mut State,
        card: Dev,
        info: &connector::Info,
        crtc: crtc::Handle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let preferred = |m: &&control::Mode| m.mode_type().contains(ModeTypeFlags::PREFERRED);
        let mut modes: Vec<control::Mode> = Vec::new();
        for mode in info.modes().iter().filter(preferred).chain(info.modes()) {
            if !modes.iter().any(|m| Mode::from(*m) == Mode::from(*mode)) {
                modes.push(*mode);
            }
        }
        let Some(mut mode) = modes.first().copied() else {
            return Ok(());
        };
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        let identity = self
            .edid(card, info.handle())
            .and_then(|edid| crate::edid::identity(&edid))
            .unwrap_or_else(|| Identity::new("Unknown", &name, ""));
        // The mode and rotation saved for this screen (ADR 0025).
        let saved = state.saved_screen(&identity);
        if let Some(wanted) = saved.as_ref().and_then(|s| s.mode) {
            let size = (wanted.width, wanted.height).into();
            let same = |m: &&control::Mode| {
                let m = Mode::from(**m);
                m.size == size && (m.refresh - wanted.refresh).abs() < 500
            };
            mode = modes.iter().find(same).copied().unwrap_or(mode);
        }
        let transform = saved.map_or(Transform::Normal, |s| {
            crate::displays::transform(s.transform)
        });
        let (w, h) = info.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (w as i32, h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: identity.make.clone(),
                model: identity.model.clone(),
            },
        );
        let wl_mode = Mode::from(mode);
        output.change_current_state(Some(wl_mode), None, None, None);
        output.set_preferred(Mode::from(modes[0]));

        let flags = GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT;
        // On another card (lot D-16): the buffers are still allocated and drawn on the card
        // that renders, linear so that the other card can import them (PRIME) and show them;
        // no cursor plane there.
        let (exporter, formats, cursor) = match self.others.get(&card) {
            Some(other) => {
                let linear: FormatSet = self
                    .renderer
                    .dmabuf_formats()
                    .into_iter()
                    .filter(|f| f.modifier == Modifier::Linear)
                    .collect();
                (other.gbm.clone(), linear, None)
            }
            None => (
                self.gbm.clone(),
                self.renderer.dmabuf_formats(),
                Some(self.gbm.clone()),
            ),
        };
        let device = self.device_mut(card).ok_or("the card is closed")?;
        let surface = device.create_surface(crtc, mode, &[info.handle()])?;
        let cursor_size = device.cursor_size();
        // The scene is built in frame pixels, at scale 1, turned (see "Rendering").
        let source = OutputModeSource::Static {
            size: wl_mode.size,
            scale: 1.0.into(),
            transform,
        };
        let compositor = Compositor::new(
            source,
            surface,
            None,
            GbmAllocator::new(self.gbm.clone(), flags),
            GbmFramebufferExporter::new(exporter, None),
            [Fourcc::Xrgb8888, Fourcc::Argb8888],
            formats,
            cursor_size,
            // The cursor goes on the screen's cursor plane when it fits (lot P-01): moving the
            // pointer then draws nothing.
            cursor,
        )?;
        let scale = self.options.scale(state.outputs().len());
        tracing::info!(screen = name, ?mode, scale, ?identity, "screen plugged in");
        let details = Details {
            identity,
            modes: modes.iter().map(|m| Mode::from(*m)).collect(),
            rotates: true,
        };
        state.plug_screen(output.clone(), wl_mode.size, scale, details);
        let head = Head {
            output,
            card,
            connector: info.handle(),
            compositor,
            pacing: Pacing::default(),
            ids: Ids::default(),
            period: period(mode),
            lock_queued: false,
            modes,
            off: false,
            dark: false,
            misses: 0,
            shown: false,
        };
        self.heads.insert((card, crtc), head);
        Ok(())
    }

    /// The EDID of a connector's screen, as its card gives it.
    fn edid(&self, card: Dev, connector: connector::Handle) -> Option<Vec<u8>> {
        let device = self.device(card)?;
        let properties = device.get_properties(connector).ok()?;
        for (handle, value) in properties.iter() {
            let info = device.get_property(*handle).ok()?;
            if info.name().to_bytes() != b"EDID" {
                continue;
            }
            if let property::Value::Blob(blob) = info.value_type().convert_value(*value) {
                return device.get_property_blob(blob).ok();
            }
        }
        None
    }

    /// Lights a screen at a mode, turned, or switches it off (`None`): the screen's part of
    /// a configuration (ADR 0025). The new mode is tried at the next frame.
    fn drive(&mut self, output: &Output, wanted: Option<(Mode, Transform)>) -> Result<(), String> {
        let head = self
            .heads
            .values_mut()
            .find(|h| h.output == *output)
            .ok_or("no such screen")?;
        let Some((mode, transform)) = wanted else {
            head.compositor.clear().map_err(|e| e.to_string())?;
            head.off = true;
            return Ok(());
        };
        let drm_mode = head
            .modes
            .iter()
            .find(|m| Mode::from(**m) == mode)
            .copied()
            .ok_or("no such mode")?;
        if head.compositor.pending_mode() != drm_mode {
            head.compositor
                .use_mode(drm_mode)
                .map_err(|e| e.to_string())?;
        }
        head.compositor
            .set_output_mode_source(OutputModeSource::Static {
                size: mode.size,
                scale: 1.0.into(),
                transform,
            });
        head.compositor.reset_buffers();
        head.period = period(drm_mode);
        head.pacing.reset();
        head.off = false;
        Ok(())
    }

    /// The console was left: no more drawing nor input until it comes back. The keys held
    /// now are released for the clients: their releases will not come.
    fn pause(&mut self, state: &mut State) {
        tracing::info!("session paused: libinput suspended");
        self.libinput.suspend();
        if let Some(keyboard) = state.seat.get_keyboard() {
            let time = state.start_time.elapsed().as_millis() as u32;
            for key in keyboard.pressed_keys() {
                state.keyboard_key(key, KeyState::Released, time);
            }
        }
        state.forget_intercepted_keys();
        state.cancel_gestures();
        state.set_lock_console_away(true);
        self.device.pause();
        for other in self.others.values_mut() {
            other.device.pause();
        }
    }

    fn activate(&mut self, state: &mut State) {
        tracing::info!("session active again: libinput resumed");
        state.set_lock_console_away(false);
        if self.libinput.resume().is_err() {
            tracing::warn!("libinput did not resume");
        }
        if let Err(err) = self.device.activate(false) {
            tracing::warn!("DRM: {err}");
        }
        for other in self.others.values_mut() {
            if let Err(err) = other.device.activate(false) {
                tracing::warn!(card = other.name, "DRM: {err}");
            }
        }
        for head in self.heads.values_mut() {
            if let Err(err) = head.compositor.reset_state() {
                tracing::warn!("DRM: {err}");
            }
            head.compositor.reset_buffers();
            head.pacing.reset();
            // A screen switched off (or dark) stays off.
            if (head.off || head.dark)
                && let Err(err) = head.compositor.clear()
            {
                tracing::warn!("DRM: {err}");
            }
        }
        // Screens may have changed while away, on the other cards too.
        self.scan(state);
        let others: Vec<Dev> = self.known.keys().copied().collect();
        for card in others {
            self.consider(state, card);
        }
    }

    fn frame_done(&mut self, state: &mut State, crtc: Key) {
        if let Some(head) = self.heads.get_mut(&crtc) {
            if let Err(err) = head.compositor.frame_submitted() {
                tracing::warn!("DRM: {err}");
            }
            head.pacing.presented();
            if std::mem::take(&mut head.lock_queued) {
                state.lock_frame_shown(&head.output);
            }
        }
        self.render(state, crtc);
    }

    /// The estimated vertical blank after a frame that was not shown: the clients waiting for a
    /// frame callback get it, as if the frame had been shown, then the next frame if one is due.
    fn estimated(&mut self, state: &mut State, crtc: Key) {
        let Some(head) = self.heads.get_mut(&crtc) else {
            return;
        };
        // The screen was reset meanwhile.
        if head.pacing.phase() != Phase::Estimate {
            return;
        }
        head.pacing.presented();
        let output = head.output.clone();
        state.send_frames(&output);
        self.render(state, crtc);
    }

    /// Calls `estimated` for `crtc` after `period`, without blocking the loop.
    fn estimate(&self, crtc: Key, period: Duration) {
        let backend = self.me.clone();
        let timer = Timer::from_duration(period);
        let inserted = self.handle.insert_source(timer, move |_, _, state| {
            if let Some(backend) = backend.upgrade() {
                backend.borrow_mut().estimated(state, crtc);
            }
            TimeoutAction::Drop
        });
        if let Err(err) = inserted {
            tracing::warn!("frame timer: {}", err.error);
        }
    }

    fn render_all(&mut self, state: &mut State) {
        let crtcs: Vec<_> = self.heads.keys().copied().collect();
        for crtc in crtcs {
            self.render(state, crtc);
        }
    }

    /// Draws the next frame of a screen if one is due (`Pacing::wants_frame`): something
    /// changed since its last frame, or an animation runs, and its previous frame is no longer
    /// waiting for the vertical blank. Only the damage is redrawn. A frame with damage goes to
    /// the screen, its vertical blank calls `frame_done`; a frame without, or a failure, waits
    /// for the estimated blank (`estimated`). Nothing here waits for the screen.
    fn render(&mut self, state: &mut State, crtc: Key) {
        let epoch = state.redraw_epoch();
        let Some(head) = self.heads.get_mut(&crtc) else {
            return;
        };
        if head.off {
            return;
        }
        // The screens dark: the screen goes off (DPMS) once, and nothing is drawn; the next
        // frame queued turns it on again (lot D-38).
        if state.screens_dark() {
            if !head.dark && self.session.is_active() {
                match head.compositor.clear() {
                    Ok(()) => tracing::info!(output = head.output.name(), "screen off"),
                    Err(err) => tracing::warn!("DRM: turning the screen off: {err}"),
                }
                head.dark = true;
            }
            return;
        }
        if std::mem::take(&mut head.dark) {
            head.pacing.reset();
        }
        if !head.pacing.wants_frame(epoch) || !self.session.is_active() {
            return;
        }
        let Some((output, view)) = state.views().into_iter().find(|(o, _)| *o == head.output)
        else {
            return;
        };
        // Each screen has a frame of its own: its slice starts at the origin.
        let size = view.frame.size;
        let view = View {
            frame: Rectangle::from_size(size),
            frame_size: size,
            ..view
        };
        let started = Instant::now();
        let since = state.perf.start();
        let now = state.start_time.elapsed();
        // Only a frame with the curtain down confirms the lock (lot D-24e).
        let locked = state.lock_frame_complete();
        let preview = self.preview.frame(now, state.snap_preview());
        let screen = [(output.clone(), view)];
        let (mut screens, mut animating) = self.scene.screens(
            &mut self.renderer,
            &mut head.ids,
            state,
            &mut self.textures,
            &screen,
            preview,
            now,
        );
        let mut elements: Vec<SessionElement> = Vec::new();
        let pointer = state.seat.get_pointer().map(|p| p.current_location());
        self.cursors.follow(state.theme());
        // The pointer sinks at a click shown (lot C-01).
        let (press, sinking) = state.click_sink(now);
        animating |= sinking;
        if let (Some(icon), Some(at)) = (state.cursor_shape(), pointer)
            && state.space.output_under(at).any(|o| *o == output)
            && let Some((cursor, next)) = self.cursors.element(
                &mut self.renderer,
                icon,
                view.point(at),
                (view.scale, press),
                now,
            )
        {
            elements.push(SessionElement::Cursor(cursor));
            // A transition of the cursor draws at each blank; a turning cursor only when its
            // next image is due (lot D-07).
            match next {
                CursorNext::Frame => animating = true,
                CursorNext::At(due) => self.cursors.wake_at(&self.handle, due, now),
                CursorNext::Rest => {}
            }
        }
        elements.extend(
            screens
                .pop()
                .into_iter()
                .flatten()
                .map(SessionElement::Scene),
        );
        state.perf.stop(perf::Phase::Scene, since);
        let since = state.perf.start();
        let clear = color::palette().desktop.premultiplied();
        let frame = head
            .compositor
            .render_frame(&mut self.renderer, &elements, clear, FRAME_FLAGS);
        let failure = match frame {
            Ok(frame) if !frame.is_empty => match head.compositor.queue_frame(()) {
                Ok(()) => {
                    head.pacing.drawn(epoch, true, animating);
                    head.lock_queued = locked;
                    head.shown = true;
                    // A new picture: the screen captures waiting for one are due (lot D-28b).
                    state.screen_changed(&head.output);
                    None
                }
                Err(err) => Some(format!("DRM: {err}")),
            },
            Ok(_) => {
                head.pacing.drawn(epoch, false, animating);
                None
            }
            Err(err) => Some(format!("render: {err}")),
        };
        state.perf.stop(perf::Phase::Draw, since);
        let period = head.period;
        let given_up = failure.is_some() && head.card != self.device_id && !head.shown && {
            head.misses += 1;
            head.misses >= SECONDARY_MISSES
        };
        if given_up {
            let card = head.card;
            tracing::warn!(
                failure = failure.as_deref().unwrap_or(""),
                "the screen of the other card shows nothing: the card is given up"
            );
            self.refused.insert(card);
            self.close_secondary(state, card);
            return;
        }
        match failure {
            // A failure that repeats on every frame is logged at the 1st, 2nd, 4th, 8th… time,
            // not 60 times a second.
            Some(err) => {
                head.pacing.failed();
                self.failures += 1;
                if self.failures.is_power_of_two() {
                    tracing::warn!(failures = self.failures, "{err}");
                }
            }
            None if self.failures > 0 => {
                tracing::info!(failures = self.failures, "frames drawn again");
                self.failures = 0;
            }
            None => {}
        }
        let shown = head.pacing.phase() == Phase::Flip;
        state.perf.frame(shown);
        if let Some(stats) = state.frame_stats.as_mut()
            && let Some(period) = stats.record(now, started.elapsed(), animating)
        {
            tracing::info!(animation = animating, "measure: {period}");
        }
        if shown {
            state.send_frames(&output);
        } else {
            self.estimate(crtc, period);
        }
    }
}

/// Frame callbacks of the windows out of view (minimized, or on another workspace): one per
/// `HIDDEN_FRAME_INTERVAL`, even when no screen draws.
fn send_hidden_frames(state: &State) {
    let time = state.start_time.elapsed();
    let Some(output) = state.space.outputs().next() else {
        return;
    };
    for window in state.space.elements() {
        if state.workspace_shift(window, time).is_none() {
            state.send_window_frame(window, output, time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn another_card_opens_only_for_a_screen_and_never_again_once_given_up() {
        assert_eq!(
            decide(false, false, false),
            Decision::Leave,
            "asleep, nothing plugged"
        );
        assert_eq!(decide(false, true, false), Decision::Open);
        assert_eq!(
            decide(false, true, true),
            Decision::Leave,
            "given up until unplugged"
        );
        assert_eq!(
            decide(true, false, false),
            Decision::Rescan,
            "unplugged: closes after"
        );
        assert_eq!(decide(true, true, false), Decision::Rescan);
    }

    #[test]
    fn the_input_is_stalled_only_when_events_wait_unread() {
        let long = INPUT_STALL + Duration::from_secs(1);
        // Nobody touches the machine: nothing waits, nothing is wrong.
        assert!(!input_stalled(long, false));
        // Events just came: the loop reads them at its next turn.
        assert!(!input_stalled(Duration::from_millis(10), true));
        // Events wait for seconds: the source is not read any more.
        assert!(input_stalled(long, true));
    }

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn the_pointer_stays_on_the_screens() {
        // A laptop screen of 1920 × 1080 and a smaller one on its right.
        let areas = [rect(0, 0, 1920, 1080), rect(1920, 0, 1280, 720)];
        let at = |x: f64, y: f64| Point::<f64, Logical>::from((x, y));
        assert_eq!(
            clamp_pointer(at(10.0, 10.0), at(-50.0, -50.0), &areas),
            at(0.0, 0.0)
        );
        assert_eq!(
            clamp_pointer(at(1900.0, 500.0), at(100.0, 0.0), &areas),
            at(2000.0, 500.0)
        );
        // Under the smaller screen, it stops at its bottom edge.
        assert_eq!(
            clamp_pointer(at(2000.0, 700.0), at(0.0, 300.0), &areas),
            at(2000.0, 719.0)
        );
        assert_eq!(
            clamp_pointer(at(3000.0, 10.0), at(900.0, 0.0), &areas),
            at(3199.0, 10.0)
        );
        // Without a screen, it does not move.
        assert_eq!(clamp_pointer(at(5.0, 5.0), at(1.0, 1.0), &[]), at(5.0, 5.0));
    }
}
