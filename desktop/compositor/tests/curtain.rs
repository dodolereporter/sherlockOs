//! The boards of the curtain (lot D-24e), drawn for real without a screen, with Mesa's software
//! renderer as in `tests/screen_off.rs`: the lock (the desktop frozen, the curtain coming down
//! over it, 8 frames) and the opening of the session (from black to the picture of the lock,
//! then to the desktop, 6 frames), in `target/tmp/curtain/`. Without Mesa's EGL the test has
//! nothing to draw with: it says so, and fails on the CI, which installs it.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::time::Duration;

    use compositor::{State, capture, curtain::Motion, render::Scene, wallpaper};
    use desktop_config::lock_background::{self, Pictures};
    use sherlock_ui::lock::{CURTAIN, CURTAIN_BLACK, CURTAIN_REVEAL};
    use smithay::{
        backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::gles::GlesRenderer,
        },
        output::Output,
        reexports::calloop::EventLoop,
    };

    const SIZE: (u32, u32) = (640, 400);

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

    /// The pictures of the lock of the repository, for the screen of the board.
    fn pictures() -> Pictures {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lock");
        let fitted = |name| lock_background::fitted(&dir.join(name), SIZE).unwrap();
        Pictures {
            dark: fitted(lock_background::DARK),
            light: fitted(lock_background::LIGHT),
        }
    }

    struct Drawing {
        scene: Scene,
        renderer: GlesRenderer,
        textures: wallpaper::Textures,
    }

    fn frame(
        drawing: &RefCell<Drawing>,
        state: &mut State,
        output: &Output,
        now: Duration,
    ) -> image::RgbaImage {
        let mut drawing = drawing.borrow_mut();
        let Drawing {
            scene,
            renderer,
            textures,
        } = &mut *drawing;
        let frame = capture::screen_at(scene, renderer, textures, state, output, now).unwrap();
        image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba).unwrap()
    }

    fn board(frames: &[image::RgbaImage], columns: u32, name: &str) -> PathBuf {
        let (w, h) = SIZE;
        let rows = (frames.len() as u32).div_ceil(columns);
        let mut board = image::RgbaImage::new(columns * w, rows * h);
        for (i, picture) in frames.iter().enumerate() {
            let (x, y) = (i as u32 % columns, i as u32 / columns);
            image::imageops::overlay(&mut board, picture, i64::from(x * w), i64::from(y * h));
        }
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("curtain");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        board.save(&path).unwrap();
        println!("board: {}", path.display());
        path
    }

    /// The share of the rows of `picture` that `reference` has too, row by row, give or take
    /// the rounding of the renderer.
    fn same_rows(picture: &image::RgbaImage, reference: &image::RgbaImage) -> f32 {
        let close = |p: &image::Rgba<u8>, q: &image::Rgba<u8>| {
            p.0.iter().zip(q.0).all(|(a, b)| a.abs_diff(b) <= 3)
        };
        let rows = picture
            .rows()
            .zip(reference.rows())
            .filter(|(a, b)| a.clone().zip(b.clone()).all(|(p, q)| close(p, q)))
            .count();
        rows as f32 / picture.height() as f32
    }

    #[test]
    fn the_boards_of_the_curtain() {
        let Some(mut renderer) = software_renderer() else {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI has Mesa's EGL (libegl-mesa0): its software device must be found"
            );
            eprintln!("no software EGL device (Mesa's llvmpipe): no board is drawn");
            return;
        };
        let scene = Scene::new(&mut renderer).unwrap();
        let drawing = Rc::new(RefCell::new(Drawing {
            scene,
            renderer,
            textures: wallpaper::Textures::default(),
        }));

        // The session opens: black, the picture, then the desktop.
        let event_loop: EventLoop<'static, State> = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        let output = state.add_output((SIZE.0 as i32, SIZE.1 as i32).into(), 1.0);
        state.set_curtain_pictures(pictures());
        let first = Duration::from_secs(1);
        state.curtain_frame(first);
        let mut opening = Vec::new();
        for t in [0, 125, 250, 450, 650, 850] {
            let now = first + Duration::from_millis(t);
            opening.push(frame(&drawing, &mut state, &output, now));
        }
        board(&opening, 3, "board-opening");
        let lit = |p: &image::RgbaImage| p.pixels().filter(|px| px.0[..3] != [0, 0, 0]).count();
        assert_eq!(lit(&opening[0]), 0, "black first");
        let gone = first + CURTAIN_BLACK + CURTAIN_REVEAL;
        let desktop = frame(&drawing, &mut state, &output, gone);
        assert_eq!(
            same_rows(&opening[5], &desktop),
            1.0,
            "the desktop, at last"
        );
        let picture = frame(&drawing, &mut state, &output, first + CURTAIN_BLACK);
        assert!(
            same_rows(&picture, &desktop) < 0.1,
            "the picture of the lock between"
        );

        // The lock: the desktop frozen, the curtain coming down over it.
        state.end_curtain_opening();
        let held = drawing.clone();
        state.set_freezer(Box::new(move |state, output| {
            let mut drawing = held.borrow_mut();
            let Drawing {
                scene,
                renderer,
                textures,
            } = &mut *drawing;
            let now = state.now();
            capture::screen_texture(scene, renderer, textures, state, output, now)
        }));
        let command = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        state.set_lock_programs(compositor::lock::Programs {
            client: command(&["sleep", "5"]),
            rescue: None,
            swaylock: None,
        });
        state.lock_session();
        assert_eq!(
            state.frozen_frames(),
            1,
            "the last frame of the screen, frozen"
        );
        let Motion::Lowering(since) = state.curtain_motion() else {
            panic!("the curtain comes down");
        };
        let mut lock = Vec::new();
        for i in 0..8u32 {
            let now = since + CURTAIN.mul_f32(i as f32 / 7.0);
            lock.push(frame(&drawing, &mut state, &output, now));
        }
        board(&lock, 4, "board-lock");
        // The first frame is the desktop as it was; the bottom of the frozen frame stays while
        // the curtain is half down; the last one is the picture of the lock alone.
        assert!(same_rows(&lock[0], &desktop) > 0.95);
        let half = same_rows(&lock[3], &desktop);
        assert!(half > 0.05 && half < 0.6, "{half}");
        assert!(same_rows(&lock[7], &picture) > 0.95);
        state.refresh();
        std::thread::sleep(CURTAIN);
        state.refresh();
        assert_eq!(
            state.frozen_frames(),
            0,
            "the frozen frame goes with the curtain down"
        );
        state.shutdown();
    }
}
