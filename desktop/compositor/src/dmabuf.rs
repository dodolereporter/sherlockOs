//! linux-dmabuf (`zwp_linux_dmabuf_v1`): clients that draw on the GPU hand their buffers over as
//! dmabufs, GPU memory shared by file descriptor, instead of copying pixels into `wl_shm`.
//!
//! Mesa's Vulkan driver presents on Wayland only through this protocol, and Mesa's EGL wants its
//! version 4 (default feedback) to find the chip to allocate on. Without it, the shell drawn with
//! iced on Vulkan (ADR 0018) falls back to software rendering, whose buffers have no alpha: black
//! around the dock and the rounded corners. Games, browsers and XWayland's glamor need it too.

use std::{error::Error as StdError, os::unix::fs::MetadataExt, path::PathBuf};

use smithay::{
    backend::{
        allocator::{Buffer as _, Format, dmabuf::Dmabuf},
        egl::EGLDevice,
        renderer::gles::GlesRenderer,
    },
    reexports::{
        wayland_protocols::wp::linux_dmabuf::zv1::server::{
            zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
            zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
            zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, Resource as _, backend::ClientId,
            delegate_dispatch, delegate_global_dispatch, protocol::wl_buffer::WlBuffer,
        },
    },
    wayland::dmabuf::{
        DmabufData, DmabufFeedbackBuilder, DmabufFeedbackData, DmabufGlobal, DmabufGlobalData,
        DmabufHandler, DmabufParamsData, DmabufState, ImportNotifier,
    },
};

use crate::State;

/// Imports a client's dmabuf into the renderer, which checks that it can draw it.
pub type Importer = Box<dyn FnMut(&Dmabuf) -> Result<(), Box<dyn StdError>>>;

/// The dmabuf global and what imports its buffers.
pub(crate) struct Dmabufs {
    state: DmabufState,
    global: Option<DmabufGlobal>,
    import: Option<Importer>,
}

impl Default for Dmabufs {
    fn default() -> Self {
        Self {
            state: DmabufState::new(),
            global: None,
            import: None,
        }
    }
}

/// DRM node of the chip a renderer draws on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// `/dev/dri/renderD128`, or the primary node when EGL does not name the render node.
    pub path: PathBuf,
    /// Its device number, which the default feedback gives clients as the main device.
    pub dev: u64,
}

/// Why linux-dmabuf is not offered.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("EGL does not name its device: {0}")]
    Device(#[from] smithay::backend::egl::Error),
    #[error("software rendering: no GPU to share buffers with")]
    Software,
    #[error("cannot read the device node {0}: {1}")]
    Stat(PathBuf, std::io::Error),
}

/// Node of the chip `renderer` draws on, from its EGL display. Software rendering (llvmpipe) has
/// none: clients keep `wl_shm`.
pub fn render_node(renderer: &GlesRenderer) -> Result<Node, NodeError> {
    let device = EGLDevice::device_for_display(renderer.egl_context().display())?;
    if device.is_software() {
        return Err(NodeError::Software);
    }
    let path = device
        .render_device_path()
        .or_else(|_| device.drm_device_path())?;
    let dev = std::fs::metadata(&path)
        .map_err(|err| NodeError::Stat(path.clone(), err))?
        .rdev();
    Ok(Node { path, dev })
}

/// Kernel driver of a device node, read from sysfs (`amdgpu`, `i915`, `nvidia`): only for the
/// log. Reading sysfs never wakes a card.
pub fn driver(dev: u64) -> Option<String> {
    let (major, minor) = (major(dev), minor(dev));
    let link = std::fs::read_link(format!("/sys/dev/char/{major}:{minor}/device/driver")).ok()?;
    Some(link.file_name()?.to_string_lossy().into_owned())
}

// Split of a Linux device number, as glibc's `major` and `minor` do.
fn major(dev: u64) -> u64 {
    ((dev >> 32) & 0xffff_f000) | ((dev >> 8) & 0x0000_0fff)
}

fn minor(dev: u64) -> u64 {
    ((dev >> 12) & 0xffff_ff00) | (dev & 0x0000_00ff)
}

impl State {
    /// Announces `zwp_linux_dmabuf_v1`, in version 5 with a default feedback: `main_device` is
    /// the node of the chip the compositor renders on, `formats` the formats and modifiers its
    /// renderer can import. Each buffer a client creates goes through `import` first: refused,
    /// the client gets `failed` and may fall back to `wl_shm`.
    ///
    /// Without a call (tests, software rendering), the protocol is not offered.
    pub fn enable_dmabuf(
        &mut self,
        main_device: u64,
        formats: impl IntoIterator<Item = Format>,
        import: Importer,
    ) -> std::io::Result<()> {
        let feedback = DmabufFeedbackBuilder::new(main_device, formats).build()?;
        let global = self
            .dmabuf
            .state
            .create_global_with_default_feedback::<Self>(&self.display_handle, &feedback);
        self.dmabuf.global = Some(global);
        self.dmabuf.import = Some(import);
        Ok(())
    }
}

impl DmabufHandler for State {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf.state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let result = match self.dmabuf.import.as_mut() {
            Some(import) => import(&dmabuf),
            None => Err("no renderer".into()),
        };
        match result {
            Ok(()) => {
                // The client may have left in the meantime: nothing to answer.
                if notifier.successful::<Self>().is_err() {
                    tracing::debug!("dmabuf imported for a client that is gone");
                }
            }
            Err(err) => {
                // A buffer the renderer cannot draw: the client is told (`failed`), not killed.
                tracing::debug!(
                    format = ?dmabuf.format(),
                    size = ?dmabuf.size(),
                    "dmabuf refused: {err}"
                );
                notifier.failed();
            }
        }
    }
}

// What smithay's `delegate_dmabuf!` writes, except for the buffer parameters, checked first below.
delegate_global_dispatch!(State: [ZwpLinuxDmabufV1: DmabufGlobalData] => DmabufState);
delegate_dispatch!(State: [ZwpLinuxDmabufV1: DmabufData] => DmabufState);
delegate_dispatch!(State: [WlBuffer: Dmabuf] => DmabufState);
delegate_dispatch!(State: [ZwpLinuxDmabufFeedbackV1: DmabufFeedbackData] => DmabufState);

impl Dispatch<ZwpLinuxBufferParamsV1, DmabufParamsData> for State {
    fn request(
        state: &mut Self,
        client: &Client,
        params: &ZwpLinuxBufferParamsV1,
        request: zwp_linux_buffer_params_v1::Request,
        data: &DmabufParamsData,
        dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        // Smithay 0.7 sends `invalid_dimensions` for a width or a height below 1, but goes on
        // checking the planes: with a height of 0, an offset plus a stride overflows (a panic in
        // a debug build), and the buffer still reaches the importer. Answered here, first.
        use zwp_linux_buffer_params_v1::Request;
        if let Request::Create { width, height, .. } | Request::CreateImmed { width, height, .. } =
            &request
            && (*width < 1 || *height < 1)
        {
            params.post_error(
                zwp_linux_buffer_params_v1::Error::InvalidDimensions,
                format!("invalid size {width} x {height}"),
            );
            return;
        }
        <DmabufState as Dispatch<ZwpLinuxBufferParamsV1, DmabufParamsData, Self>>::request(
            state, client, params, request, data, dh, data_init,
        );
    }

    fn destroyed(
        state: &mut Self,
        client: ClientId,
        params: &ZwpLinuxBufferParamsV1,
        data: &DmabufParamsData,
    ) {
        <DmabufState as Dispatch<ZwpLinuxBufferParamsV1, DmabufParamsData, Self>>::destroyed(
            state, client, params, data,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// glibc's `makedev`.
    fn makedev(major: u64, minor: u64) -> u64 {
        ((major & 0xffff_f000) << 32)
            | ((major & 0xfff) << 8)
            | ((minor & 0xffff_ff00) << 12)
            | (minor & 0xff)
    }

    #[test]
    fn device_numbers_split_like_glibc() {
        // The first render node, and numbers that use the high bits.
        for (ma, mi) in [(226, 128), (0x1234, 0x5678), (0xfffff, 0xffff_ffff)] {
            let dev = makedev(ma, mi);
            assert_eq!((major(dev), minor(dev)), (ma, mi));
        }
        assert_eq!(makedev(226, 128), (226 << 8) | 128);
    }
}
