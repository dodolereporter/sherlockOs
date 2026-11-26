//! A wgpu device without a window, for captures and tests.
//!
//! Only the Vulkan backend is compiled (ADR 0018). The driver the Vulkan loader may open is
//! decided before, by the caller, through the environment: `sherlock_ui::gpu::Choice` for the
//! AMD chip, or Mesa's software driver (lavapipe) where there is no usable chip.

use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

/// Why no device could be made.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Vulkan adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("the adapter refused a device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
}

/// A device on the low-power Vulkan adapter, with the adapter's description.
pub fn device() -> Result<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo), Error> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))?;
    let info = adapter.get_info();
    let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("sherlock-3d"),
        required_limits: wgpu::Limits::downlevel_defaults(),
        ..Default::default()
    }))?;
    Ok((device, queue, info))
}

/// Runs a future of wgpu to its end. On native backends they are ready at once; the loop only
/// guards against one that is not.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut context = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
            return output;
        }
        std::thread::yield_now();
    }
}
