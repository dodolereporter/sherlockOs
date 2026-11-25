//! Real session: the compositor started from a text console or by the login screen.
//!
//! - **Session**: libseat (seatd or logind) opens the devices without root and tells when the
//!   console is left or entered again (Ctrl + Alt + Fn).
//! - **Card**: the one `card::choose` picks from `/sys`, and only that one: an NVIDIA card is
//!   never opened. udev reports screens plugged in and out.
//! - **Screens**: every connector with a screen, each driven by a `DrmCompositor` at its
//!   preferred mode, drawn by the same GLES scene as the windowed backend (`render::Scene`).
//! - **Input**: libinput (keyboard, mouse, touchpad with tap to click).
//!
//! See "Real session" in `docs/compositor.md`.

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

use sherlock_ui::color;
use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent,
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
    desktop::layer_map_for_output,
    output::{Mode, Output, OutputModeSource, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{
            EventLoop, LoopHandle,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{Device as _, ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::{Dev, OFlags},
    },
    utils::{DeviceFd, Logical, Point, Rectangle, Transform},
};

use crate::{
    Error, State,
    anim::PreviewMotion,
    card,
    cursor::Cursors,
    options::Options,
    output::View,
    render::{Scene, SceneElement},
    wallpaper::{Textures, Wallpaper},
};

/// libglvnd's variable: the EGL vendor libraries it may load.
pub const EGL_VENDORS: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";

/// Folders of libglvnd's EGL vendor files.
const GLVND_DIRS: [&str; 2] = ["/etc/glvnd/egl_vendor.d", "/usr/share/glvnd/egl_vendor.d"];

/// Frame period when nothing changed on a screen: its next frame is tried after it.
const IDLE: Duration = Duration::from_millis(16);

/// Lets libglvnd load Mesa's EGL only, unless the user set `__EGL_VENDOR_LIBRARY_FILENAMES`.
/// On Debian NVIDIA's comes first (`10_nvidia.json`), and loading it may wake the card.
/// Returns the value the variable had, for the commands the compositor launches.
///
/// # Safety
///
/// Changes the process environment: the process must have a single thread.
pub unsafe fn restrict_egl_to_mesa() -> Option<OsString> {
    let previous = std::env::var_os(EGL_VENDORS);
    let dirs = GLVND_DIRS.map(Path::new);
    if previous.is_none()
        && let Some(mesa) = mesa_vendor(&dirs)
    {
        // SAFETY: the caller guarantees that the process has a single thread.
        unsafe { std::env::set_var(EGL_VENDORS, mesa) };
    }
    previous
}

/// Mesa's EGL vendor file (`50_mesa.json`) in the first of `dirs` that has one.
fn mesa_vendor(dirs: &[&Path]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        files.into_iter().find(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("mesa") && n.ends_with(".json"))
        })
    })
}

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

/// A connector with a screen, driven by a CRTC.
struct Head {
    output: Output,
    connector: connector::Handle,
    compositor: Compositor,
    /// A frame waits for the screen's vertical blank.
    queued: bool,
}

struct Backend {
    session: LibSeatSession,
    libinput: Libinput,
    device: DrmDevice,
    /// Device number of the card, as udev names it.
    device_id: Dev,
    gbm: GbmDevice<DrmDeviceFd>,
    renderer: GlesRenderer,
    scene: Scene,
    textures: Textures,
    preview: PreviewMotion,
    cursors: Cursors,
    heads: HashMap<crtc::Handle, Head>,
    options: Options,
}

/// Opens the session and the card, creates one screen per connector with a screen, and plugs
/// input, screen changes, frames and console switches into the loop.
pub fn init(
    event_loop: &EventLoop<'static, State>,
    state: &mut State,
    options: &Options,
) -> Result<(), Error> {
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
        desktop_config::data_dirs(var),
    )?;
    let mut switcher = session.clone();
    state.switch_vt = Some(Box::new(move |vt| {
        if let Err(err) = switcher.change_vt(vt) {
            tracing::warn!(vt, "switching console: {err}");
        }
    }));

    let backend = Rc::new(RefCell::new(Backend {
        session,
        libinput: libinput.clone(),
        device,
        device_id,
        gbm,
        renderer,
        scene,
        textures: Textures::default(),
        preview: PreviewMotion::default(),
        cursors: Cursors::load(),
        heads: HashMap::new(),
        options: options.clone(),
    }));
    backend.borrow_mut().scan(state);
    insert_sources(&event_loop.handle(), &backend, notifier, drm_events, udev)?;
    let input = LibinputInputBackend::new(libinput);
    event_loop
        .handle()
        .insert_source(input, |event, _, state| input_event(event, state))
        .map_err(|e| e.error)?;
    Ok(())
}

fn insert_sources(
    handle: &LoopHandle<'static, State>,
    backend: &Rc<RefCell<Backend>>,
    notifier: smithay::backend::session::libseat::LibSeatSessionNotifier,
    drm_events: smithay::backend::drm::DrmDeviceNotifier,
    udev: UdevBackend,
) -> Result<(), Error> {
    let b = backend.clone();
    handle
        .insert_source(notifier, move |event, _, state| match event {
            SessionEvent::PauseSession => b.borrow_mut().pause(state),
            SessionEvent::ActivateSession => b.borrow_mut().activate(state),
        })
        .map_err(|e| e.error)?;
    let b = backend.clone();
    handle
        .insert_source(drm_events, move |event, _, state| match event {
            DrmEvent::VBlank(crtc) => b.borrow_mut().frame_done(state, crtc),
            DrmEvent::Error(err) => tracing::warn!("DRM: {err}"),
        })
        .map_err(|e| e.error)?;
    let b = backend.clone();
    handle
        .insert_source(udev, move |event, _, state| match event {
            UdevEvent::Changed { device_id } if device_id == b.borrow().device_id => {
                b.borrow_mut().scan(state);
            }
            UdevEvent::Removed { device_id } if device_id == b.borrow().device_id => {
                tracing::error!("the card is gone");
                state.loop_signal.stop();
            }
            // Other cards are never opened.
            other => tracing::debug!(?other, "udev"),
        })
        .map_err(|e| e.error)?;
    // Screens whose last frame changed nothing are drawn again a little later.
    let b = backend.clone();
    handle
        .insert_source(Timer::from_duration(IDLE), move |_, _, state| {
            b.borrow_mut().render_all(state);
            TimeoutAction::ToDuration(IDLE)
        })
        .map_err(|e| e.error)?;
    Ok(())
}

fn input_event(event: InputEvent<LibinputInputBackend>, state: &mut State) {
    match event {
        InputEvent::DeviceAdded { mut device } => {
            // Tap to click on touchpads, off by default in libinput.
            if device.config_tap_finger_count() > 0
                && let Err(err) = device.config_tap_set_enabled(true)
            {
                tracing::warn!(?err, "tap to click");
            }
            tracing::info!(name = device.name(), "input device");
        }
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
    /// Adds the screens plugged in, removes those unplugged.
    fn scan(&mut self, state: &mut State) {
        if !self.session.is_active() {
            return;
        }
        let Ok(resources) = self.device.resource_handles() else {
            tracing::warn!("cannot read the card's connectors");
            return;
        };
        let connected: Vec<connector::Info> = resources
            .connectors()
            .iter()
            .filter_map(|c| self.device.get_connector(*c, true).ok())
            .filter(|i| i.state() == connector::State::Connected && !i.modes().is_empty())
            .collect();
        let gone: Vec<crtc::Handle> = self
            .heads
            .iter()
            .filter(|(_, h)| !connected.iter().any(|i| i.handle() == h.connector))
            .map(|(crtc, _)| *crtc)
            .collect();
        for crtc in gone {
            if let Some(head) = self.heads.remove(&crtc) {
                tracing::info!(screen = head.output.name(), "screen unplugged");
                state.remove_screen(&head.output);
            }
        }
        for info in connected {
            if self.heads.values().any(|h| h.connector == info.handle()) {
                continue;
            }
            // A CRTC that one of its encoders can use, and that no screen drives yet.
            let crtc = info
                .encoders()
                .iter()
                .filter_map(|e| self.device.get_encoder(*e).ok())
                .flat_map(|e| resources.filter_crtcs(e.possible_crtcs()))
                .find(|crtc| !self.heads.contains_key(crtc));
            let Some(crtc) = crtc else {
                tracing::warn!(connector = ?info.handle(), "no CRTC left for this screen");
                continue;
            };
            if let Err(err) = self.add_head(state, &info, crtc) {
                tracing::warn!(connector = ?info.handle(), "screen not driven: {err}");
            }
        }
        self.render_all(state);
    }

    fn add_head(
        &mut self,
        state: &mut State,
        info: &connector::Info,
        crtc: crtc::Handle,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let modes = info.modes();
        let preferred = modes
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED));
        let Some(mode) = preferred.or(modes.first()).copied() else {
            return Ok(());
        };
        let name = format!("{}-{}", info.interface().as_str(), info.interface_id());
        let (w, h) = info.size().unwrap_or((0, 0));
        let output = Output::new(
            name.clone(),
            PhysicalProperties {
                size: (w as i32, h as i32).into(),
                subpixel: Subpixel::Unknown,
                make: "Unknown".into(),
                model: "Unknown".into(),
            },
        );
        let wl_mode = Mode::from(mode);
        output.change_current_state(Some(wl_mode), None, None, None);
        output.set_preferred(wl_mode);

        let surface = self.device.create_surface(crtc, mode, &[info.handle()])?;
        let flags = GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT;
        // The scene is built in frame pixels, at scale 1 (see "Rendering").
        let source = OutputModeSource::Static {
            size: wl_mode.size,
            scale: 1.0.into(),
            transform: Transform::Normal,
        };
        let compositor = Compositor::new(
            source,
            surface,
            None,
            GbmAllocator::new(self.gbm.clone(), flags),
            GbmFramebufferExporter::new(self.gbm.clone(), None),
            [Fourcc::Xrgb8888, Fourcc::Argb8888],
            self.renderer.dmabuf_formats(),
            self.device.cursor_size(),
            None,
        )?;
        let scale = self.options.scale(state.outputs().len());
        tracing::info!(screen = name, ?mode, scale, "screen plugged in");
        state.add_screen(output.clone(), wl_mode.size, scale);
        let head = Head {
            output,
            connector: info.handle(),
            compositor,
            queued: false,
        };
        self.heads.insert(crtc, head);
        Ok(())
    }

    /// The console was left: no more drawing nor input until it comes back. The keys held
    /// now are released for the clients: their releases will not come.
    fn pause(&mut self, state: &mut State) {
        tracing::info!("session paused");
        self.libinput.suspend();
        if let Some(keyboard) = state.seat.get_keyboard() {
            let time = state.start_time.elapsed().as_millis() as u32;
            for key in keyboard.pressed_keys() {
                state.keyboard_key(key, KeyState::Released, time);
            }
        }
        state.forget_intercepted_keys();
        self.device.pause();
    }

    fn activate(&mut self, state: &mut State) {
        tracing::info!("session active again");
        if self.libinput.resume().is_err() {
            tracing::warn!("libinput did not resume");
        }
        if let Err(err) = self.device.activate(false) {
            tracing::warn!("DRM: {err}");
        }
        for head in self.heads.values_mut() {
            if let Err(err) = head.compositor.reset_state() {
                tracing::warn!("DRM: {err}");
            }
            head.compositor.reset_buffers();
            head.queued = false;
        }
        // Screens may have changed while away.
        self.scan(state);
    }

    fn frame_done(&mut self, state: &mut State, crtc: crtc::Handle) {
        if let Some(head) = self.heads.get_mut(&crtc) {
            if let Err(err) = head.compositor.frame_submitted() {
                tracing::warn!("DRM: {err}");
            }
            head.queued = false;
        }
        self.render(state, crtc);
    }

    fn render_all(&mut self, state: &mut State) {
        let crtcs: Vec<_> = self.heads.keys().copied().collect();
        for crtc in crtcs {
            self.render(state, crtc);
        }
    }

    /// Draws the next frame of a screen, unless the previous one still waits for its blank.
    fn render(&mut self, state: &mut State, crtc: crtc::Handle) {
        let Some(head) = self.heads.get_mut(&crtc) else {
            return;
        };
        if head.queued || !self.session.is_active() {
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
        let now = state.start_time.elapsed();
        let preview = self.preview.frame(now, state.snap_preview());
        let screen = [(output.clone(), view)];
        let (mut screens, animating) = self.scene.screens(
            &mut self.renderer,
            state,
            &mut self.textures,
            &screen,
            preview,
            now,
        );
        let mut elements: Vec<SessionElement> = Vec::new();
        let pointer = state.seat.get_pointer().map(|p| p.current_location());
        if let (Some(icon), Some(at)) = (state.cursor_icon(), pointer)
            && state.space.output_under(at).any(|o| *o == output)
            && let Some(cursor) =
                self.cursors
                    .element(&mut self.renderer, icon, view.point(at), view.scale)
        {
            elements.push(SessionElement::Cursor(cursor));
        }
        elements.extend(
            screens
                .pop()
                .into_iter()
                .flatten()
                .map(SessionElement::Scene),
        );
        let clear = color::DESKTOP.premultiplied();
        let frame =
            head.compositor
                .render_frame(&mut self.renderer, &elements, clear, FrameFlags::empty());
        match frame {
            Ok(frame) if !frame.is_empty => match head.compositor.queue_frame(()) {
                Ok(()) => head.queued = true,
                Err(err) => tracing::warn!("DRM: {err}"),
            },
            Ok(_) => {}
            Err(err) => tracing::warn!("render: {err}"),
        }
        if let Some(stats) = state.frame_stats.as_mut()
            && let Some(period) = stats.record(now, started.elapsed(), animating)
        {
            tracing::info!(animation = animating, "measure: {period}");
        }
        send_frames(state, &output);
    }
}

/// Frame callbacks of the surfaces shown on `output`: windows and anchored surfaces.
fn send_frames(state: &State, output: &Output) {
    let time = state.start_time.elapsed();
    let throttle = Some(Duration::ZERO);
    for window in state.space.elements() {
        if state.window_output(window).as_ref() == Some(output) {
            window.send_frame(output, time, throttle, |_, _| Some(output.clone()));
        }
    }
    for layer in layer_map_for_output(output).layers() {
        layer.send_frame(output, time, throttle, |_, _| Some(output.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn mesa_is_the_only_egl_vendor() {
        let root = std::env::temp_dir().join(format!("compositor-egl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (etc, share) = (root.join("etc"), root.join("share"));
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::create_dir_all(&share).unwrap();
        for file in ["10_nvidia.json", "50_mesa.json"] {
            std::fs::write(share.join(file), "{}").unwrap();
        }
        let found = mesa_vendor(&[&etc, &share, &root.join("missing")]);
        assert_eq!(found, Some(share.join("50_mesa.json")));
        // Without Mesa's, nothing is restricted.
        std::fs::remove_file(share.join("50_mesa.json")).unwrap();
        assert_eq!(mesa_vendor(&[&etc, &share]), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
