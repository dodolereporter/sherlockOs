//! What starting the GPU renderer of a shell program costs, step by step (lot P-02): the
//! renderer choice (ADR 0018), the Vulkan instance, the adapter, the device, iced's pipelines,
//! and the system fonts iced reads. No window: the Wayland part (connection, surface, first
//! frame) is measured in a session (`docs/launcher.md`, Performance).
//!
//! `cargo run --release -p sherlock-ui --example startup`

use std::io::Write;
use std::time::Instant;

use iced::futures::executor::block_on;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let choice = sherlock_ui::gpu::Choice::detect();
    // SAFETY: first statement of `main` that touches the environment; no other thread exists.
    unsafe { choice.apply() };
    let mut out = std::io::stdout();
    let mut last = started;
    let mut step = |out: &mut std::io::Stdout, name: &str| -> std::io::Result<()> {
        let now = Instant::now();
        let took = now.duration_since(last).as_secs_f64() * 1000.0;
        last = now;
        writeln!(out, "{name:<32} {took:>8.1} ms")
    };
    writeln!(out, "renderer: {choice:?}")?;
    step(&mut out, "renderer chosen")?;
    if !matches!(choice, sherlock_ui::gpu::Choice::Vulkan { .. }) {
        writeln!(out, "no GPU path on this machine")?;
        return Ok(());
    }
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::default()
    });
    step(&mut out, "Vulkan instance")?;
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        ..wgpu::RequestAdapterOptions::default()
    }))?;
    writeln!(out, "adapter: {}", adapter.get_info().name)?;
    step(&mut out, "adapter")?;
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    step(&mut out, "device")?;
    let engine = iced_renderer::wgpu::Engine::new(
        &adapter,
        device,
        queue,
        wgpu::TextureFormat::Bgra8Unorm,
        None,
        iced_renderer::graphics::Shell::headless(),
    );
    step(&mut out, "iced pipelines")?;
    drop(engine);
    let fonts = iced_renderer::graphics::text::font_system();
    let faces = fonts.write().map(|mut f| f.raw().db().len()).unwrap_or(0);
    step(&mut out, &format!("system fonts ({faces} faces)"))?;
    writeln!(
        out,
        "{:<32} {:>8.1} ms",
        "total",
        started.elapsed().as_secs_f64() * 1000.0
    )?;
    Ok(())
}
