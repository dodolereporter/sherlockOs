//! Screenshots (lots D-35c, D-35e). Print Screen takes the screen under the pointer, Alt + Print
//! Screen the window that has the keyboard; Shift + Print Screen and Super + Shift + S open the
//! screenshot tool (`screenshot_tool`).
//! The compositor draws them itself, offscreen, with the scene of the screen or the last image
//! of a window (`window_anim::snapshot`), through the backend, which has the renderer
//! (`Capturer`). A worker thread writes the PNG into the screenshots folder of the pictures
//! folder and sends a notification; the image then goes to the clipboard. Real session only:
//! nested mode leaves the key to the host session. See `docs/screenshots.md`.

use std::{
    cell::Cell,
    ffi::OsString,
    fs::File,
    io::Write as _,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, mpsc},
    time::Duration,
};

use sherlock_ui::{
    color, screenshot as ui,
    screenshot_tool::{self as tool_ui, Destination, Folder, Options},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind as _, ExportMem as _, Texture as _, TextureMapping as _,
            gles::{GlesRenderer, GlesTexture},
        },
    },
    desktop::Window,
    input::keyboard::{Keysym, ModifiersState},
    output::Output,
    reexports::{
        calloop::{
            Interest, Mode, PostAction,
            channel::{self, Event},
            generic::Generic,
        },
        rustix,
    },
    utils::{Physical, Rectangle, Size},
    wayland::selection::{SelectionTarget, data_device::set_data_device_selection},
};

use crate::{
    State,
    damage::Ids,
    output::View,
    render::Scene,
    screenshot_tool::{Pending, Tool},
    wallpaper, window_anim,
};

/// What a key takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Screen,
    Area,
    Window,
}

impl Kind {
    /// Print Screen alone, with Shift, with Alt (xkb gives `Sys_Req` for Alt + Print Screen).
    pub fn of(sym: Keysym, mods: &ModifiersState) -> Option<Self> {
        match sym {
            Keysym::Sys_Req => Some(Self::Window),
            Keysym::Print if mods.alt => Some(Self::Window),
            Keysym::Print if mods.shift => Some(Self::Area),
            Keysym::Print => Some(Self::Screen),
            _ => None,
        }
    }
}

/// What the backend draws.
pub enum Target {
    Screen(Output),
    Window(Window),
}

/// An image, straight RGBA, rows from the top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    /// The part of the image in `rect`, in its pixels, cut to the image.
    pub fn crop(&self, rect: Rectangle<i32, Physical>) -> Option<Self> {
        let whole = Rectangle::from_size((self.width as i32, self.height as i32).into());
        let rect = rect.intersection(whole)?;
        let (x, y, w, h) = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        let row = self.width as usize * 4;
        let rgba = (y..y + h)
            .flat_map(|r| {
                let start = r as usize * row + x as usize * 4;
                self.rgba[start..start + w as usize * 4].iter().copied()
            })
            .collect();
        Some(Self {
            width: w as u32,
            height: h as u32,
            rgba,
        })
    }
}

/// Draws a target into an image: given by the backend, which has the renderer.
pub type Capturer = Box<dyn FnMut(&mut State, &Target) -> Option<Image>>;

/// Where the screenshots go and how they are confirmed. The tests give folders and a bus of
/// their own.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// The pictures folder, or `None` for the user's (`XDG_PICTURES_DIR`).
    pub pictures: Option<PathBuf>,
    /// Address of the session bus for the notification, or `None` for the session's.
    pub session_bus: Option<String>,
}

/// An image for the worker, and where it goes.
struct Job {
    image: Image,
    destination: Destination,
    folder: Folder,
}

#[derive(Default)]
pub struct Capture {
    jobs: Option<mpsc::Sender<Job>>,
    capturer: Option<Capturer>,
    /// The screenshot tool, open or going (lot D-35e), its choices, and the image its timer
    /// waits for.
    pub(crate) tool: Option<Tool>,
    pub(crate) options: Options,
    pub(crate) pending: Option<Pending>,
    /// Pastes of a screenshot being written.
    pastes: Rc<Cell<usize>>,
}

/// Most pastes of a screenshot written at the same time: past it, a paste is refused (its
/// pipe closed), so that a client that asks without reading holds no more.
pub const MAX_PASTES: usize = 8;

/// The pictures folder: `XDG_PICTURES_DIR` of `user-dirs.dirs`, or `Images` in the home folder.
pub fn pictures(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    user_dir(("XDG_PICTURES_DIR", ui::PICTURES), var)
}

/// The folder a screenshot is saved in: in the pictures folder, or the desktop or documents
/// folder of `user-dirs.dirs`; under `base` instead when the tests give one.
pub fn folder_of(
    folder: Folder,
    base: Option<&Path>,
    var: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let (key, default) = folder.user_dir();
    let dir = match (folder, base) {
        (Folder::Screenshots, Some(base)) => base.to_owned(),
        (_, Some(base)) => base.join(default),
        _ => user_dir((key, default), var)?,
    };
    Some(match folder {
        Folder::Screenshots => dir.join(ui::FOLDER),
        _ => dir,
    })
}

/// The folder of `key` in `user-dirs.dirs`, or `default` in the home folder.
fn user_dir(
    (key, default): (&str, &str),
    var: impl Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let home = PathBuf::from(var("HOME")?);
    let config = var("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
    let dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).unwrap_or_default();
    let set = dirs.lines().find_map(|line| {
        let value = line
            .trim()
            .strip_prefix(key)?
            .strip_prefix('=')?
            .trim_matches('"');
        let path = match value.strip_prefix("$HOME") {
            Some(rest) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(value),
        };
        // `$HOME/` alone means no such folder: the home folder itself is not one.
        (path.is_absolute() && path != home).then_some(path)
    });
    Some(set.unwrap_or_else(|| home.join(default)))
}

/// Writes `png` into `folder` under the name of the moment, a copy number added if the name
/// is taken. Returns its path.
fn write_new(folder: &Path, png: &[u8]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let now = appearance_config::local(appearance_config::epoch_now())?;
    let date = (
        now.tm_year + 1900,
        (now.tm_mon + 1) as u32,
        now.tm_mday as u32,
    );
    let time = (now.tm_hour as u32, now.tm_min as u32, now.tm_sec as u32);
    for copy in std::iter::once(0).chain(2..100) {
        let path = folder.join(ui::file_name(date, time, copy));
        match File::create_new(&path) {
            Ok(mut file) => {
                file.write_all(png)?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::other("every name of this second is taken"))
}

fn encode(image: &Image) -> Result<Vec<u8>, image::ImageError> {
    use image::ImageEncoder as _;
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(
        &image.rgba,
        image.width,
        image.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png)
}

/// The worker: each image written, put in the clipboard by the loop, then confirmed.
fn work(settings: Settings, jobs: mpsc::Receiver<Job>, saved: channel::Sender<Arc<[u8]>>) {
    let notifier = notifier(settings.session_bus.clone());
    while let Ok(job) = jobs.recv() {
        let png = match encode(&job.image) {
            Ok(png) => png,
            Err(err) => {
                tracing::warn!("screenshot not encoded: {err}");
                continue;
            }
        };
        let (title, body) = match job.destination {
            Destination::CopyOnly => (tool_ui::COPIED, String::from(tool_ui::COPIED_BODY)),
            Destination::SaveAndCopy => {
                let base = settings.pictures.as_deref();
                let Some(folder) = folder_of(job.folder, base, |k| std::env::var_os(k)) else {
                    tracing::warn!("screenshot: no home folder");
                    continue;
                };
                let path = match write_new(&folder, &png) {
                    Ok(path) => path,
                    Err(err) => {
                        tracing::warn!(folder = %folder.display(), "screenshot not written: {err}");
                        continue;
                    }
                };
                tracing::info!(path = %path.display(), "screenshot written");
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
                (ui::SAVED, ui::saved_body(&name.unwrap_or_default()))
            }
        };
        if saved.send(Arc::from(png)).is_err() {
            return;
        }
        // Handed to the notifier: a notifications daemon that no longer answers never holds
        // the next screenshots back. Past a few waiting, the confirmation is left out.
        if let Some(notifier) = &notifier
            && notifier.try_send((title, body)).is_err()
        {
            tracing::warn!("screenshot notification left out: the notifier is behind");
        }
    }
}

/// Longest wait for the notifications daemon, to connect or to answer.
const NOTIFY_LIMIT: Duration = Duration::from_secs(5);

/// Confirmations waiting for the notifier, at most.
const NOTIFY_QUEUE: usize = 4;

/// A confirmation: its title and its text.
type Confirmation = (&'static str, String);

/// A thread of its own that sends the confirmations of the screenshots on the session bus at
/// `address` (the session's without one), each call limited to `NOTIFY_LIMIT`. Its connection
/// is made again after an error: a bus or a daemon that restarted is found again.
fn notifier(address: Option<String>) -> Option<mpsc::SyncSender<Confirmation>> {
    let (sender, confirmations) = mpsc::sync_channel::<Confirmation>(NOTIFY_QUEUE);
    let spawned = std::thread::Builder::new()
        .name(String::from("screenshot-notify"))
        .spawn(move || {
            let mut bus: Option<zbus::Connection> = None;
            while let Ok((title, body)) = confirmations.recv() {
                let sent = zbus::block_on(async {
                    if bus.is_none() {
                        let builder = match &address {
                            Some(address) => zbus::connection::Builder::address(address.as_str())?,
                            None => zbus::connection::Builder::session()?,
                        };
                        bus = Some(builder.method_timeout(NOTIFY_LIMIT).build().await?);
                    }
                    match &bus {
                        Some(bus) => system_services::notify::send(bus, ui::APP, title, &body)
                            .await
                            .map_err(|p| zbus::Error::Failure(p.to_string())),
                        None => Ok(0),
                    }
                });
                if let Err(err) = sent {
                    tracing::warn!("screenshot notification: {err}");
                    bus = None;
                }
            }
        });
    match spawned {
        Ok(_) => Some(sender),
        Err(err) => {
            tracing::warn!("cannot start the screenshot notifier: {err}");
            None
        }
    }
}

impl State {
    /// Takes the Print Screen key from now on. The real session calls it.
    pub fn enable_screenshots(&mut self, settings: Settings) {
        let (jobs, worker_jobs) = mpsc::channel();
        let (saved, pngs) = channel::channel();
        let inserted = self.loop_handle.insert_source(pngs, |event, _, state| {
            if let Event::Msg(png) = event {
                state.copy_png(png);
            }
        });
        if let Err(err) = inserted {
            tracing::warn!("no screenshots: {err}");
            return;
        }
        let spawned = std::thread::Builder::new()
            .name(String::from("screenshots"))
            .spawn(move || work(settings, worker_jobs, saved));
        match spawned {
            Ok(_) => self.capture.jobs = Some(jobs),
            Err(err) => tracing::warn!("no screenshots: {err}"),
        }
    }

    /// How the backend draws a screenshot.
    pub fn set_capturer(&mut self, capturer: Capturer) {
        self.capture.capturer = Some(capturer);
    }

    /// Draws `target` with the backend's capturer, for screen sharing (lot D-28b). `None`
    /// without a renderer.
    pub(crate) fn draw_capture(&mut self, target: &Target) -> Option<Image> {
        let mut capturer = self.capture.capturer.take()?;
        let image = capturer(self, target);
        self.capture.capturer = Some(capturer);
        image
    }

    pub(crate) fn takes_screenshots(&self) -> bool {
        self.capture.jobs.is_some()
    }

    /// The screenshot tool is open: it has the pointer and the keyboard.
    pub fn picking_area(&self) -> bool {
        self.screenshot_tool_open()
    }

    /// The key `kind` is pressed.
    pub(crate) fn screenshot(&mut self, kind: Kind) {
        let target = match kind {
            Kind::Screen => self.pointer_output().map(Target::Screen),
            Kind::Window => self.focused_window().map(Target::Window),
            Kind::Area => {
                if !self.screenshot_tool_open() {
                    self.capture.options.mode = tool_ui::Mode::Area;
                    self.open_screenshot_tool();
                }
                return;
            }
        };
        match target {
            Some(target) => {
                let options = Options::default();
                self.take_with(&target, None, &options);
            }
            None => tracing::info!(?kind, "screenshot: nothing to take"),
        }
    }

    /// Draws `target`, cut to `crop`, and hands it to the worker with the choices of
    /// `options`.
    pub(crate) fn take_with(
        &mut self,
        target: &Target,
        crop: Option<Rectangle<i32, Physical>>,
        options: &Options,
    ) {
        let Some(mut capturer) = self.capture.capturer.take() else {
            tracing::warn!("screenshot: no renderer");
            return;
        };
        let image = capturer(self, target);
        self.capture.capturer = Some(capturer);
        let image = match (image, crop) {
            (Some(image), Some(rect)) => image.crop(rect),
            (image, _) => image,
        };
        match (image, &self.capture.jobs) {
            (Some(image), Some(jobs)) => {
                let _ = jobs.send(Job {
                    image,
                    destination: options.destination,
                    folder: options.folder,
                });
            }
            (None, _) => tracing::warn!("screenshot: nothing drawn"),
            _ => {}
        }
    }

    /// The console is left: the tool and its timer go, at once.
    pub(crate) fn cancel_area(&mut self) {
        let open = self.capture.tool.take().is_some() || self.capture.pending.take().is_some();
        if open {
            self.queue_redraw();
        }
    }

    /// A client pastes the compositor's selection (a screenshot, an item of the clipboard
    /// history): `png`, whatever its type, is written into `fd` by the loop, as the pipe takes
    /// it, never waiting for the reader; no thread per paste.
    pub(crate) fn paste_png(&mut self, fd: OwnedFd, png: Arc<[u8]>) {
        let pastes = self.capture.pastes.clone();
        if pastes.get() >= MAX_PASTES {
            tracing::debug!("screenshot not pasted: {MAX_PASTES} pastes under way");
            return;
        }
        let flags = rustix::fs::OFlags::NONBLOCK;
        if let Err(err) = rustix::fs::fcntl_setfl(&fd, flags) {
            tracing::debug!("screenshot not pasted: {err}");
            return;
        }
        let mut written = 0;
        let ended = pastes.clone();
        let source = Generic::new(File::from(fd), Interest::WRITE, Mode::Level);
        let inserted = self.loop_handle.insert_source(source, move |_, file, _| {
            loop {
                match rustix::io::write(&**file, &png[written..]) {
                    Ok(0) => break,
                    Ok(n) => {
                        written += n;
                        if written == png.len() {
                            break;
                        }
                    }
                    Err(rustix::io::Errno::AGAIN) => return Ok(PostAction::Continue),
                    Err(rustix::io::Errno::INTR) => {}
                    Err(err) => {
                        tracing::debug!("screenshot not pasted: {err}");
                        break;
                    }
                }
            }
            ended.set(ended.get().saturating_sub(1));
            Ok(PostAction::Remove)
        });
        match inserted {
            Ok(_) => pastes.set(pastes.get() + 1),
            Err(err) => tracing::debug!("screenshot not pasted: {err}"),
        }
    }

    /// Pastes of a screenshot under way.
    pub fn pastes(&self) -> usize {
        self.capture.pastes.get()
    }

    /// The image goes to the clipboard, as `image/png`, for X11 applications too (lot D-29).
    pub fn copy_png(&mut self, png: Arc<[u8]>) {
        self.hold_selection(crate::clipboard_history::Held::png(png));
    }

    /// The compositor offers `held` as the clipboard, to Wayland and X11 applications: a
    /// screenshot, or an item of the clipboard history.
    pub(crate) fn hold_selection(&mut self, held: crate::clipboard_history::Held) {
        let mime = held.types();
        let origin = crate::handlers::SelectionOrigin::Compositor;
        self.selection_set(SelectionTarget::Clipboard, origin, &mime);
        set_data_device_selection(&self.display_handle, &self.seat, mime.clone(), Some(held));
        self.offer_to_xwayland(SelectionTarget::Clipboard, Some(mime));
    }
}

/// Reads `texture` back: straight RGBA, rows from the top.
fn read(renderer: &mut GlesRenderer, mut texture: GlesTexture) -> Option<Image> {
    let size = texture.size();
    let target = renderer.bind(&mut texture).ok()?;
    let region = Rectangle::from_size(size);
    let mapping = renderer
        .copy_framebuffer(&target, region, Fourcc::Abgr8888)
        .ok()?;
    drop(target);
    let mut rgba = renderer.map_texture(&mapping).ok()?.to_vec();
    let (w, h) = (size.w as usize, size.h as usize);
    // GL reads the rows from the bottom up.
    if !mapping.flipped() {
        rgba = rgba.chunks(w * 4).rev().flatten().copied().collect();
    }
    for pixel in rgba.chunks_mut(4) {
        let a = u32::from(pixel[3]);
        if a > 0 && a < 255 {
            for c in &mut pixel[..3] {
                *c = (u32::from(*c) * 255 / a).min(255) as u8;
            }
        }
    }
    (rgba.len() == w * h * 4).then_some(Image {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

/// Draws `target`: a screen as it shows now, without the cursor; a window as at rest, its title
/// bar, border and corners included. The backends call it from their `Capturer`.
pub fn image(
    scene: &Scene,
    renderer: &mut GlesRenderer,
    textures: &mut wallpaper::Textures,
    state: &mut State,
    target: &Target,
) -> Option<Image> {
    let texture = match target {
        Target::Window(window) => window_anim::snapshot(scene, renderer, state, window)?,
        Target::Screen(output) => {
            let (_, view) = state.views().into_iter().find(|(o, _)| o == output)?;
            let size: Size<i32, Physical> = view.frame.size;
            let view = View {
                frame: Rectangle::from_size(size),
                frame_size: size,
                ..view
            };
            let now = state.now();
            let views = [(output.clone(), view)];
            let mut ids = Ids::default();
            let (screens, _) =
                scene.screens(renderer, &mut ids, state, textures, &views, None, now);
            let elements = screens.into_iter().next()?;
            let desktop = color::palette().desktop.premultiplied();
            window_anim::draw(renderer, size, &elements, desktop)
                .inspect_err(|err| tracing::warn!("screenshot: drawing the screen: {err}"))
                .ok()?
        }
    };
    read(renderer, texture)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_daemon_that_never_answers_holds_no_screenshot_back() {
        // A bus that takes the connection and never says a word: the notifier waits there,
        // the screenshots go on, and past a few waiting the confirmations are left out.
        let dir = std::env::temp_dir().join(format!("capture-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("bus");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let notifier = notifier(Some(format!("unix:path={}", socket.display()))).unwrap();
        notifier
            .try_send((ui::SAVED, String::from("first")))
            .unwrap();
        // The notifier took the first one and waits on the silent bus.
        let _held = listener.accept().unwrap();
        let start = std::time::Instant::now();
        let taken = (0..20)
            .filter(|n| notifier.try_send((ui::SAVED, format!("shot {n}"))).is_ok())
            .count();
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(
            (NOTIFY_QUEUE - 1..=NOTIFY_QUEUE).contains(&taken),
            "{taken}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_keys_choose_what_is_taken() {
        let none = ModifiersState::default();
        let shift = ModifiersState {
            shift: true,
            ..none
        };
        let alt = ModifiersState { alt: true, ..none };
        assert_eq!(Kind::of(Keysym::Print, &none), Some(Kind::Screen));
        assert_eq!(Kind::of(Keysym::Print, &shift), Some(Kind::Area));
        assert_eq!(Kind::of(Keysym::Print, &alt), Some(Kind::Window));
        assert_eq!(Kind::of(Keysym::Sys_Req, &alt), Some(Kind::Window));
        assert_eq!(Kind::of(Keysym::a, &none), None);
    }

    #[test]
    fn the_pictures_folder_comes_from_user_dirs() {
        let dir = std::env::temp_dir().join(format!("capture-dirs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let home = dir.join("home");
        let var = |config: &Path| {
            let (home, config) = (home.clone(), config.to_owned());
            move |k: &str| match k {
                "HOME" => Some(home.clone().into_os_string()),
                "XDG_CONFIG_HOME" => Some(config.clone().into_os_string()),
                _ => None,
            }
        };
        assert_eq!(pictures(var(&dir)), Some(home.join("Images")));
        let dirs = "# written by xdg-user-dirs-update\nXDG_PICTURES_DIR=\"$HOME/Photos\"\n";
        std::fs::write(dir.join("user-dirs.dirs"), dirs).unwrap();
        assert_eq!(pictures(var(&dir)), Some(home.join("Photos")));
        std::fs::write(dir.join("user-dirs.dirs"), "XDG_PICTURES_DIR=\"$HOME/\"\n").unwrap();
        assert_eq!(pictures(var(&dir)), Some(home.join("Images")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pastes_are_written_by_the_loop_and_bounded() {
        use smithay::reexports::calloop::EventLoop;
        use std::io::Read as _;
        let event_loop = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        let mut event_loop = event_loop;
        // More than a pipe holds: written as the reader reads, the loop never waiting.
        let png: Arc<[u8]> = (0..1_000_000u32).map(|i| i as u8).collect();
        let (read, write) = rustix::pipe::pipe().unwrap();
        state.paste_png(write, png.clone());
        assert_eq!(state.pastes(), 1);
        let reader = std::thread::spawn(move || {
            let mut got = Vec::new();
            File::from(read).read_to_end(&mut got).unwrap();
            got
        });
        while state.pastes() > 0 {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(5)), &mut state)
                .unwrap();
        }
        assert_eq!(reader.join().unwrap(), &*png);
        // Clients that never read: at most `MAX_PASTES` at once, the next one refused.
        let mut readers = Vec::new();
        for _ in 0..=MAX_PASTES {
            let (read, write) = rustix::pipe::pipe().unwrap();
            state.paste_png(write, png.clone());
            readers.push(read);
        }
        event_loop
            .dispatch(Some(std::time::Duration::from_millis(5)), &mut state)
            .unwrap();
        assert_eq!(state.pastes(), MAX_PASTES);
        // Readers gone: the writes end.
        drop(readers);
        while state.pastes() > 0 {
            event_loop
                .dispatch(Some(std::time::Duration::from_millis(5)), &mut state)
                .unwrap();
        }
    }

    #[test]
    fn leaving_the_console_gives_up_the_tool() {
        let event_loop = smithay::reexports::calloop::EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        state.add_output((800, 600).into(), 1.0);
        let dir = std::env::temp_dir().join(format!("capture-console-{}", std::process::id()));
        state.enable_screenshots(Settings {
            pictures: Some(dir),
            session_bus: None,
        });
        state.screenshot(Kind::Area);
        assert!(state.picking_area());
        state.tool_button(true, true, (10.0, 10.0).into());
        state.tool_motion((50.0, 40.0).into());
        state.tool_button(true, false, (50.0, 40.0).into());
        assert!(state.screenshot_area().is_some());
        // What `udev` does when the session is paused (Ctrl + Alt + Fn).
        state.forget_intercepted_keys();
        assert!(!state.picking_area());
        assert!(state.screenshot_area().is_none());
    }

    #[test]
    fn a_crop_stays_in_the_image() {
        let image = Image {
            width: 4,
            height: 3,
            rgba: (0..48).collect(),
        };
        let part = image
            .crop(Rectangle::new((1, 1).into(), (2, 2).into()))
            .unwrap();
        assert_eq!((part.width, part.height), (2, 2));
        assert_eq!(&part.rgba[..4], &[20, 21, 22, 23]);
        let cut = image
            .crop(Rectangle::new((3, 2).into(), (5, 5).into()))
            .unwrap();
        assert_eq!((cut.width, cut.height), (1, 1));
        assert!(
            image
                .crop(Rectangle::new((9, 9).into(), (2, 2).into()))
                .is_none()
        );
    }

    #[test]
    fn the_image_is_written_under_a_new_name() {
        let dir = std::env::temp_dir().join(format!("capture-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let image = Image {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 0, 255, 128],
        };
        let png = encode(&image).unwrap();
        let first = write_new(&dir, &png).unwrap();
        let second = write_new(&dir, &png).unwrap();
        assert_ne!(first, second);
        for path in [&first, &second] {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            assert!(name.starts_with("Capture d'écran du "), "{name}");
        }
        let back = image::open(&first).unwrap().to_rgba8();
        assert_eq!(back.into_raw(), image.rgba);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
