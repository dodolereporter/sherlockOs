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
    }

    #[test]
    fn shift_print_screen_takes_the_area_drawn_with_the_mouse() {
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
        let picked = h.state.picked_area().unwrap();
        assert_eq!((picked.size.w, picked.size.h), (200.0, 150.0));
        h.state.pointer_button(BTN_LEFT, ButtonState::Released, 5);
        assert!(!h.state.picking_area());
        h.run_until("the notification", |h| !h.sent.lock().unwrap().is_empty());
        let shots = h.shots();
        assert_eq!(shots.len(), 1, "{shots:?}");
        let image = image::open(&shots[0]).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (200, 150));
    }
}
