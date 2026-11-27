//! Screenshots drawn for real, without a screen (lot D-35c): Mesa's software renderer
//! (llvmpipe), reached through an EGL device of its own, draws the scene of an 800 × 600 screen.
//! The keys go through `State::keyboard_key`, the area through the pointer, as the real session
//! sends them. The PNG goes into a pictures folder of the test's own, the notification to a fake
//! notification server on a private bus (its own `dbus-daemon`), the image into the clipboard of
//! the seat. Without Mesa's EGL the test has nothing to draw with: it says so, and fails on the
//! CI, which installs it.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use compositor::{
        State,
        capture::{self, Settings},
        render::Scene,
        wallpaper,
    };
    use sherlock_ui::color;
    use smithay::{
        backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            input::{ButtonState, KeyState},
            renderer::gles::GlesRenderer,
        },
        input::keyboard::Keycode,
        reexports::calloop::EventLoop,
        wayland::selection::data_device::current_data_device_selection_userdata,
    };

    // xkb codes: the evdev codes of linux/input-event-codes.h, plus 8.
    const PRINT: u32 = 99 + 8;
    const SHIFT: u32 = 42 + 8;
    const ESCAPE: u32 = 1 + 8;
    const ENTER: u32 = 28 + 8;
    const BTN_LEFT: u32 = 0x110;

    /// A renderer on Mesa's software device, if there is one.
    fn software_renderer() -> Option<GlesRenderer> {
        let software = |d: &EGLDevice| {
            d.extensions()
                .iter()
                .any(|e| e == "EGL_MESA_device_software")
        };
        let device = EGLDevice::enumerate().ok()?.find(software)?;
        // SAFETY: nothing else in this test creates EGL displays, so none of them can be
        // terminated behind smithay's back.
        let display = unsafe { EGLDisplay::new(device) }.ok()?;
        let context = EGLContext::new(&display).ok()?;
        // SAFETY: the context was just created and is used by this thread only.
        unsafe { GlesRenderer::new(context) }.ok()
    }

    /// A private bus: its own `dbus-daemon`, socket in the test folder, killed on drop.
    struct Bus {
        daemon: Child,
        address: String,
        _served: Option<zbus::blocking::Connection>,
    }

    impl Drop for Bus {
        fn drop(&mut self) {
            self._served = None;
            let _ = self.daemon.kill();
            let _ = self.daemon.wait();
        }
    }

    type Sent = Arc<Mutex<Vec<(String, String, String)>>>;

    struct Notifications(Sent);

    #[zbus::interface(name = "org.freedesktop.Notifications")]
    impl Notifications {
        #[allow(clippy::too_many_arguments)]
        fn notify(
            &self,
            app: String,
            _replaces: u32,
            _icon: String,
            summary: String,
            body: String,
            _actions: Vec<String>,
            _hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
            _timeout: i32,
        ) -> u32 {
            let mut sent = self.0.lock().unwrap();
            sent.push((app, summary, body));
            sent.len() as u32
        }
    }

    fn bus(dir: &Path, sent: Sent) -> Bus {
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                "<busconfig><type>session</type><listen>unix:dir={}</listen>\
                 <auth>EXTERNAL</auth><policy context=\"default\">\
                 <allow send_destination=\"*\" eavesdrop=\"true\"/><allow eavesdrop=\"true\"/>\
                 <allow own=\"*\"/></policy></busconfig>",
                dir.display()
            ),
        )
        .unwrap();
        let mut daemon = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .args(["--nofork", "--print-address"])
            .stdout(Stdio::piped())
            .spawn()
            .expect("dbus-daemon (package dbus-daemon) is needed by these tests");
        let mut address = String::new();
        BufReader::new(daemon.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_owned();
        let served = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .serve_at("/org/freedesktop/Notifications", Notifications(sent))
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .build()
            .unwrap();
        Bus {
            daemon,
            address,
            _served: Some(served),
        }
    }

    struct Harness {
        event_loop: EventLoop<'static, State>,
        state: State,
        dir: PathBuf,
        sent: Sent,
        _bus: Bus,
    }

    impl Harness {
        fn start(name: &str, renderer: GlesRenderer) -> Self {
            let dir =
                std::env::temp_dir().join(format!("screenshots-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let sent = Sent::default();
            let bus = bus(&dir, sent.clone());
            let event_loop = EventLoop::try_new().unwrap();
            let mut state = State::new(&event_loop).unwrap();
            state.add_output((800, 600).into(), 1.0);
            let mut renderer = renderer;
            let scene = Scene::new(&mut renderer).unwrap();
            let mut textures = wallpaper::Textures::default();
            state.set_capturer(Box::new(move |state, target| {
                capture::image(&scene, &mut renderer, &mut textures, state, target)
            }));
            state.enable_screenshots(Settings {
                pictures: Some(dir.join("pictures")),
                session_bus: Some(bus.address.clone()),
            });
            Self {
                event_loop,
                state,
                dir,
                sent,
                _bus: bus,
            }
        }

        fn key(&mut self, code: u32, state: KeyState) {
            self.state.keyboard_key(Keycode::new(code), state, 1);
        }

        fn run_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let end = Instant::now() + Duration::from_secs(20);
            while !done(self) {
                assert!(Instant::now() < end, "timed out waiting for: {what}");
                self.event_loop
                    .dispatch(Some(Duration::from_millis(10)), &mut self.state)
                    .unwrap();
            }
        }

        fn shots(&self) -> Vec<PathBuf> {
            let folder = self
                .dir
                .join("pictures")
                .join(sherlock_ui::screenshot::FOLDER);
            let mut shots: Vec<PathBuf> = std::fs::read_dir(folder)
                .map(|d| d.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            shots.sort();
            shots
        }

        fn clipboard(&self) -> Option<Arc<[u8]>> {
            current_data_device_selection_userdata(&self.state.seat)
                .and_then(|d| d.as_ref().and_then(|held| held.bytes("image/png")))
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn renderer_or_skip() -> Option<GlesRenderer> {
        let renderer = software_renderer();
        if renderer.is_none() {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI has Mesa's EGL (libegl-mesa0): its software device must be found"
            );
            eprintln!("no software EGL device (Mesa's llvmpipe): no screenshot is drawn");
        }
        renderer
    }

    #[test]
    fn print_screen_writes_the_screen_notifies_and_copies_it() {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut h = Harness::start("screen", renderer);
        h.key(PRINT, KeyState::Pressed);
        h.key(PRINT, KeyState::Released);
        h.run_until("the notification", |h| !h.sent.lock().unwrap().is_empty());
        let shots = h.shots();
        assert_eq!(shots.len(), 1, "{shots:?}");
        let image = image::open(&shots[0]).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (800, 600));
        // The desktop, its color or one of its discs, without a wallpaper: opaque everywhere.
        let palette = color::palette();
        let known: Vec<[u8; 3]> = std::iter::once(palette.desktop)
            .chain(palette.desktop_discs)
            .map(|c| [c.r, c.g, c.b])
            .collect();
        for (x, y) in [(2, 2), (400, 300), (797, 597)] {
            let p = image.get_pixel(x, y).0;
            assert_eq!(p[3], 255);
            let near = |k: &[u8; 3]| (0..3).all(|i| p[i].abs_diff(k[i]) <= 2);
            assert!(known.iter().any(near), "({x}, {y}): {p:?}");
        }
        let (app, summary, body) = h.sent.lock().unwrap()[0].clone();
        assert_eq!(app, sherlock_ui::screenshot::APP);
        assert_eq!(summary, sherlock_ui::screenshot::SAVED);
        let name = shots[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(body.starts_with(&name), "{body}");
        h.run_until("the clipboard", |h| h.clipboard().is_some());
        let png = h.clipboard().unwrap();
        assert_eq!(&*png, std::fs::read(&shots[0]).unwrap().as_slice());
        // Its thumbnail, in the bottom right corner once slid in (lot D-35e).
        std::thread::sleep(sherlock_ui::screenshot_tool::ENTER + Duration::from_millis(50));
        let thumb = h.state.thumbnail_rect().unwrap();
        let right = thumb.loc.x + thumb.size.w;
        let margin = f64::from(sherlock_ui::screenshot_tool::THUMBNAIL_MARGIN);
        assert!(
            thumb.loc.y > 300.0 && (800.0 - margin - right).abs() < 1.0,
            "{thumb:?}"
        );
        // Dragged to the right edge, it goes; short of it, it comes back.
        let middle = thumb.loc + thumb.size.downscale(2.0).to_point();
        h.state.pointer_motion(middle, 10);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 11);
        h.state
            .pointer_motion((middle.x + 20.0, middle.y + 2.0).into(), 12);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 13);
        assert!(h.state.thumbnail_rect().is_some());
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 14);
        h.state
            .pointer_motion((middle.x + 150.0, middle.y).into(), 15);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 16);
        assert!(h.state.thumbnail_rect().is_none());
    }

    #[test]
    fn shift_print_screen_takes_the_area_drawn_with_the_mouse_on_enter() {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut h = Harness::start("area", renderer);
        // Escape gives up an area.
        h.key(SHIFT, KeyState::Pressed);
        h.key(PRINT, KeyState::Pressed);
        h.key(PRINT, KeyState::Released);
        h.key(SHIFT, KeyState::Released);
        assert!(h.state.picking_area());
        h.key(ESCAPE, KeyState::Pressed);
        h.key(ESCAPE, KeyState::Released);
        assert!(!h.state.picking_area());
        // Then one drawn from (100, 100) to (300, 250).
        h.key(SHIFT, KeyState::Pressed);
        h.key(PRINT, KeyState::Pressed);
        h.key(PRINT, KeyState::Released);
        h.key(SHIFT, KeyState::Released);
        h.state.pointer_motion((100.0, 100.0).into(), 2);
        h.state.pointer_button(BTN_LEFT, ButtonState::Pressed, 3);
        h.state.pointer_motion((300.0, 250.0).into(), 4);
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 5);
        let picked = h.state.screenshot_area().unwrap();
        assert_eq!((picked.size.w, picked.size.h), (200.0, 150.0));
        // The area stays, to adjust, until Enter takes it (lot D-35e).
        assert!(h.state.picking_area());
        h.key(ENTER, KeyState::Pressed);
        h.key(ENTER, KeyState::Released);
        assert!(!h.state.picking_area());
        h.run_until("the notification", |h| !h.sent.lock().unwrap().is_empty());
        let shots = h.shots();
        assert_eq!(shots.len(), 1, "{shots:?}");
        let image = image::open(&shots[0]).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (200, 150));
    }

    /// The tool drawn for real over the desktop of a screen of `size` pixels at `scale`, at
    /// `stage`: « area », an area drawn from a third to two thirds of the screen; « menu », the
    /// same with the options menu open; « drawing », the area being drawn, with the loupe and
    /// its size. The image of the screen, as the renderer shows it, before the tool opens and
    /// with it.
    fn board(
        size: (i32, i32),
        scale: f64,
        stage: &str,
    ) -> Option<(image::RgbaImage, image::RgbaImage)> {
        use compositor::capture::Target;
        use sherlock_ui::screenshot_tool::Control;
        let mut renderer = software_renderer()?;
        let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        let output = state.add_output(size.into(), scale);
        let dir = std::env::temp_dir().join(format!("screenshots-board-{}", std::process::id()));
        state.enable_screenshots(Settings {
            pictures: Some(dir),
            session_bus: None,
        });
        let scene = Scene::new(&mut renderer).unwrap();
        let mut textures = wallpaper::Textures::default();
        state.set_capturer(Box::new(move |state, target| {
            capture::image(&scene, &mut renderer, &mut textures, state, target)
        }));
        let target = Target::Screen(output);
        let draw = |state: &mut State| {
            let shot = state.draw_target(&target)?;
            image::RgbaImage::from_raw(shot.width, shot.height, shot.rgba)
        };
        let before = draw(&mut state)?;
        let (w, h) = (f64::from(size.0) / scale, f64::from(size.1) / scale);
        state.pointer_motion((w / 3.0, h / 3.0).into(), 1);
        state.open_screenshot_tool();
        // The screen kept for the loupe, taken once the key's event is handled.
        event_loop
            .dispatch(Some(Duration::from_millis(10)), &mut state)
            .unwrap();
        state.pointer_button(BTN_LEFT, ButtonState::Pressed, 2);
        state.pointer_motion((w * 2.0 / 3.0, h * 2.0 / 3.0).into(), 3);
        if stage != "drawing" {
            state.pointer_button(BTN_LEFT, ButtonState::Released, 4);
        }
        if stage == "menu" {
            let button = state.screenshot_control(Control::Options).unwrap();
            let center = button.loc + button.size.downscale(2.0).to_point();
            state.pointer_motion(center, 5);
            state.pointer_button(BTN_LEFT, ButtonState::Pressed, 6);
            state.pointer_button(BTN_LEFT, ButtonState::Released, 7);
        }
        // Past the time of the entry.
        std::thread::sleep(Duration::from_millis(400));
        Some((before, draw(&mut state)?))
    }

    #[test]
    fn the_tool_draws_at_every_size() {
        if renderer_or_skip().is_none() {
            return;
        }
        let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        for (size, scale, stage) in [
            ((1920, 1080), 1.0, "area"),
            ((1920, 1080), 1.0, "menu"),
            ((1920, 1080), 1.0, "drawing"),
            ((1366, 768), 1.0, "menu"),
            ((1366, 768), 1.0, "drawing"),
            ((2560, 1600), 2.0, "area"),
            ((1920, 1200), 1.5, "menu"),
        ] {
            let (before, image) = board(size, scale, stage).unwrap();
            assert_eq!(image.dimensions(), (size.0 as u32, size.1 as u32));
            let name = format!("screenshot-tool-{}x{}@{scale}-{stage}.png", size.0, size.1);
            let path = out.join(name);
            image.save(&path).unwrap();
            eprintln!("{}", path.display());
            // Dimmed around the area, as it was inside it.
            let (w, h) = (size.0 as u32, size.1 as u32);
            let lum = |p: &image::Rgba<u8>| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
            let (x, y) = (w / 10, h / 10);
            let (dim, clear) = (image.get_pixel(x, y), before.get_pixel(x, y));
            assert!(lum(dim) * 3 < lum(clear) * 2, "{dim:?} against {clear:?}");
            let (x, y) = (w / 2, h / 2);
            let (inside, clear) = (image.get_pixel(x, y), before.get_pixel(x, y));
            assert!(
                lum(inside).abs_diff(lum(clear)) <= 6,
                "{inside:?} against {clear:?}"
            );
        }
    }
}
