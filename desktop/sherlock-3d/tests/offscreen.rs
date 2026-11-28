//! Renders the mascot off screen with wgpu on Vulkan and checks a few pixels.
//!
//! Picking the Vulkan driver changes the environment, which needs a single thread: this test
//! runs without libtest's harness (`harness = false`).

mod support;

use sherlock_3d::{Camera, Cut, Frame, Pose, Renderer, headless};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result {
    if support::listed("offscreen") {
        return Ok(());
    }
    // SAFETY: no harness, so no other thread exists yet.
    let Some(driver) = (unsafe { support::pick_driver() }) else {
        support::skip("no Vulkan driver for the AMD chip, and no lavapipe");
        return Ok(());
    };
    let Some(model) = support::model() else {
        return Ok(());
    };
    let (device, queue, adapter) = headless::device()?;
    support::say(&format!(
        "rendering with {driver}: {} ({})",
        adapter.name, adapter.driver
    ));

    let mut renderer = Renderer::new(&device, &queue, &model, wgpu::TextureFormat::Rgba8Unorm);
    let repos = model.clip("repos").ok_or("no clip repos")?;
    let joints = Pose::sampled(&model, repos, 0.0).joint_matrices(&model);
    let frame = |cut, background| Frame {
        size: [256, 256],
        joints: &joints,
        camera: Camera::UPPER_BODY,
        cut,
        background,
    };
    let dir = support::captures()?;

    // Transparent around the mascot, opaque on it.
    let image = renderer.capture(&device, &queue, &frame(Cut::None, [0.0; 4]), [0.0; 4])?;
    image.save(dir.join("offscreen-transparent.png"))?;
    assert_eq!(image.get_pixel(2, 2).0[3], 0, "the corner is transparent");
    let center = image.get_pixel(128, 128).0;
    assert_eq!(
        center[3], 255,
        "the middle of the face is opaque: {center:?}"
    );
    let opaque = image.pixels().filter(|p| p.0[3] == 255).count();
    assert!(
        opaque > 256 * 256 / 4,
        "the mascot fills the frame: {opaque}"
    );
    // Antialiased edges: some pixels are partly covered.
    let partial = image
        .pixels()
        .filter(|p| (1..255).contains(&p.0[3]))
        .count();
    assert!(partial > 50, "multisampling smooths the edges: {partial}");

    // In the bubble, over a background: nothing outside the disc, the background inside.
    let background = [0.2, 0.4, 0.6, 1.0];
    let bubble = frame(Cut::Circle { radius: 1.0 }, background);
    let image = renderer.capture(&device, &queue, &bubble, [0.0; 4])?;
    image.save(dir.join("offscreen-bubble.png"))?;
    assert_eq!(image.get_pixel(6, 6).0[3], 0, "outside the disc");
    assert_eq!(
        image.get_pixel(250, 30).0[3],
        0,
        "outside the disc, top right"
    );
    let inside = image.get_pixel(128, 253).0;
    assert_eq!(inside[3], 255, "inside the disc, bottom: {inside:?}");
    // The background, where the mascot is not: 0.2, 0.4, 0.6 in sRGB.
    let side = image.get_pixel(6, 128).0;
    assert!(
        side[0].abs_diff(51) <= 2 && side[1].abs_diff(102) <= 2 && side[2].abs_diff(153) <= 2,
        "background inside the disc: {side:?}"
    );
    support::say(&format!("captures in {}", dir.display()));
    Ok(())
}
