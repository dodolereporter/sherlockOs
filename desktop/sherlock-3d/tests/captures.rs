//! Captures of the moods the lock screen uses, in the round bubble, dark and light, at two
//! sizes. They go to `target/tmp/sherlock-3d/captures/`, never into the repository.
//!
//! Picking the Vulkan driver changes the environment, which needs a single thread: this test
//! runs without libtest's harness (`harness = false`).

mod support;

use std::sync::Arc;

use sherlock_3d::{Animator, Camera, Cut, Frame, Mood, Renderer, headless};
use sherlock_ui::color::{Color, Palette};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Mood, seconds played, file name.
const STATES: [(Mood, f32, &str); 5] = [
    (Mood::Idle, 1.0, "repos"),
    // Mid-gesture: the hand up.
    (Mood::Wave, 0.7, "salut"),
    (Mood::EyesClosed, 1.0, "yeux-fermes"),
    (Mood::Peek, 1.0, "peek"),
    (Mood::Sorry, 1.0, "desole"),
];
/// Bubble sides in pixels: about the lock screen's at scale 1, and at scale 2.
const SIDES: [u32; 2] = [170, 340];

fn srgb(color: Color) -> [f32; 4] {
    let c = |v: u8| f32::from(v) / 255.0;
    [c(color.r), c(color.g), c(color.b), color.a]
}

fn main() -> Result {
    // SAFETY: no harness, so no other thread exists yet.
    let Some(driver) = (unsafe { support::pick_driver() }) else {
        support::skip("no Vulkan driver for the AMD chip, and no lavapipe");
        return Ok(());
    };
    let Some(model) = support::model() else {
        return Ok(());
    };
    let model = Arc::new(model);
    let (device, queue, _) = headless::device()?;
    support::say(&format!("captures with {driver}"));
    let dir = support::captures()?.join("captures");
    std::fs::create_dir_all(&dir)?;

    let mut renderer = Renderer::new(&device, &queue, &model, wgpu::TextureFormat::Rgba8Unorm);
    let mut eyes = Vec::new();
    for (mood, seconds, name) in STATES {
        let mut animator = Animator::new(model.clone());
        animator.set_mood(mood);
        for _ in 0..(seconds * 30.0).round() as usize {
            animator.advance(1.0 / 30.0);
        }
        let joints = animator.joints();
        for (theme, palette) in [("dark", Palette::DARK), ("light", Palette::LIGHT)] {
            for side in SIDES {
                let frame = Frame {
                    size: [side, side],
                    joints: &joints,
                    camera: Camera::UPPER_BODY,
                    cut: Cut::Circle { radius: 1.0 },
                    background: srgb(palette.system_discs[1]),
                };
                let image =
                    renderer.capture(&device, &queue, &frame, srgb(palette.system_screen))?;
                image.save(dir.join(format!("{name}-{theme}-{side}.png")))?;
                // The screen's background outside the bubble, opaque everywhere.
                let corner = image.get_pixel(1, 1).0;
                let screen = palette.system_screen;
                assert_eq!(
                    corner,
                    [screen.r, screen.g, screen.b, 255],
                    "{name} {theme}"
                );
                if theme == "dark" && side == SIDES[1] {
                    eyes.push((name, image));
                }
            }
        }
    }

    // The eyes differ: open at rest, both closed, one closed in Peek.
    let region = |image: &image::RgbaImage| -> Vec<u8> {
        let (w, h) = image.dimensions();
        let mut bytes = Vec::new();
        for y in h * 30 / 100..h * 50 / 100 {
            for x in w * 20 / 100..w * 80 / 100 {
                bytes.extend_from_slice(&image.get_pixel(x, y).0);
            }
        }
        bytes
    };
    let find = |wanted: &str| {
        eyes.iter()
            .find(|(name, _)| *name == wanted)
            .map(|(_, image)| region(image))
            .ok_or(format!("no capture {wanted}"))
    };
    let (rest, closed, peek) = (find("repos")?, find("yeux-fermes")?, find("peek")?);
    assert_ne!(rest, closed);
    assert_ne!(rest, peek);
    assert_ne!(closed, peek);
    support::say(&format!("captures in {}", dir.display()));
    Ok(())
}
