//! The board of the screens going off (lot D-38), drawn for real without a screen, with Mesa's
//! software renderer as in `tests/screenshots.rs`: eight frames of the animation, from the whole
//! picture to black, side by side in `target/tmp/screen-off-board.png`, for the author to judge.
//! Without Mesa's EGL the test has nothing to draw with: it says so, and fails on the CI, which
//! installs it.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use compositor::{
        State, capture,
        power::{Phase, Then},
        render::Scene,
        wallpaper,
    };
    use smithay::{
        backend::{
            egl::{EGLContext, EGLDevice, EGLDisplay},
            renderer::gles::GlesRenderer,
        },
        reexports::calloop::EventLoop,
    };

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

    #[test]
    fn the_board_of_the_screens_going_off() {
        let Some(mut renderer) = software_renderer() else {
            assert!(
                std::env::var_os("CI").is_none(),
                "the CI has Mesa's EGL (libegl-mesa0): its software device must be found"
            );
            eprintln!("no software EGL device (Mesa's llvmpipe): no board is drawn");
            return;
        };
        let event_loop: EventLoop<'static, State> = EventLoop::try_new().unwrap();
        let mut state = State::new(&event_loop).unwrap();
        let (w, h) = (640u32, 400u32);
        let output = state.add_output((w as i32, h as i32).into(), 1.0);
        let scene = Scene::new(&mut renderer).unwrap();
        let mut textures = wallpaper::Textures::default();
        state.screens_off(Then::Stay);
        let Phase::Leaving { since, .. } = state.power_phase() else {
            panic!("the screens go off");
        };
        let duration = sherlock_ui::screen_off::DURATION;
        let mut board = image::RgbaImage::new(w * 4, h * 2);
        let mut frames = Vec::new();
        for i in 0..8u32 {
            let now = since + duration.mul_f32(i as f32 / 7.0) + Duration::from_micros(1);
            let frame = capture::screen_at(
                &scene,
                &mut renderer,
                &mut textures,
                &mut state,
                &output,
                now,
            )
            .unwrap();
            let picture =
                image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba).unwrap();
            image::imageops::overlay(
                &mut board,
                &picture,
                i64::from((i % 4) * w),
                i64::from((i / 4) * h),
            );
            frames.push(picture);
        }
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("screen-off-board.png");
        board.save(&path).unwrap();
        println!("board: {}", path.display());
        let lit = |p: &image::RgbaImage| p.pixels().filter(|px| px.0[..3] != [0, 0, 0]).count();
        // The first frame is the whole picture, the last one black. From the second one, the
        // glow lights the whole squashed picture (the desktop has black parts, without its
        // wallpaper), and the light only shrinks.
        assert!(lit(&frames[0]) > (w * h / 2) as usize);
        assert_eq!(lit(&frames[7]), 0, "the last frame is black");
        let counts: Vec<usize> = frames.iter().map(lit).collect();
        assert!(counts[1..].windows(2).all(|c| c[0] >= c[1]), "{counts:?}");
    }
}
