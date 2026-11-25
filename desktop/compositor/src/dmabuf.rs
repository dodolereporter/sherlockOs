//! linux-dmabuf (`zwp_linux_dmabuf_v1`): clients that draw on the GPU hand their buffers over as
//! dmabufs, GPU memory shared by file descriptor, instead of copying pixels into `wl_shm`.
//!
//! Mesa's Vulkan driver presents on Wayland only through this protocol, and Mesa's EGL wants its
//! version 4 (default feedback) to find the chip to allocate on. Without it, the shell drawn with
//! iced on Vulkan (ADR 0018) falls back to software rendering, whose buffers have no alpha: black
//! around the dock and the rounded corners. Games, browsers and XWayland's glamor need it too.

use std::{
    collections::{HashMap, HashSet},
    error::Error as StdError,
    os::{
        fd::{AsFd, BorrowedFd},
        unix::fs::MetadataExt,
    },
    path::PathBuf,
};

use smithay::{
    backend::{
        allocator::{Buffer as _, Format, Fourcc, dmabuf::Dmabuf},
        egl::EGLDevice,
        renderer::gles::GlesRenderer,
    },
    reexports::{
        rustix::fs::{SeekFrom, seek},
        wayland_protocols::wp::linux_dmabuf::zv1::server::{
            zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
            zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
            zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        },
        wayland_server::{
            Client, DataInit, Dispatch, DisplayHandle, Resource as _,
            backend::{ClientId, ObjectId},
            delegate_dispatch, delegate_global_dispatch,
            protocol::wl_buffer::WlBuffer,
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
    /// The formats announced, whatever their modifier.
    announced: HashSet<Fourcc>,
    /// The planes each `zwp_linux_buffer_params_v1` received, until it is destroyed.
    params: HashMap<ObjectId, Vec<Plane>>,
}

impl Default for Dmabufs {
    fn default() -> Self {
        Self {
            state: DmabufState::new(),
            global: None,
            import: None,
            announced: HashSet::new(),
            params: HashMap::new(),
        }
    }
}

/// One plane of a buffer, as the client describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plane {
    /// The index the client gives it (`plane_idx`).
    pub index: u32,
    pub offset: u32,
    pub stride: u32,
    /// Size of its file, from `lseek`. `None` when the file cannot be seeked.
    pub size: Option<u64>,
}

impl Plane {
    /// Reads the size of `fd` with `lseek`, and puts its position back, as smithay does.
    pub fn new(fd: BorrowedFd<'_>, index: u32, offset: u32, stride: u32) -> Self {
        let size = seek(fd, SeekFrom::End(0)).ok();
        let _ = seek(fd, SeekFrom::Start(0));
        Self {
            index,
            offset,
            stride,
            size,
        }
    }
}

/// Why planes do not describe a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PlaneError {
    #[error("plane indices are not 0 to n - 1")]
    Holes,
    #[error("{0} planes given, {1} needed by the format")]
    Missing(usize, usize),
    #[error("plane {0} ends past its file")]
    OutOfBounds(u32),
    #[error("the size of the file of plane {0} is unknown")]
    UnknownSize(u32),
}

/// Colour planes of a format, and the vertical subsampling of the planes after the first. Some
/// modifiers add metadata planes after them (AMD's DCC): those are not counted here.
fn layout(format: Fourcc) -> (usize, u32) {
    use Fourcc::*;
    match format {
        Nv12 | Nv21 | Nv15 | P010 | P012 | P016 => (2, 2),
        Nv16 | Nv61 | Nv24 | Nv42 | P210 => (2, 1),
        Yuv410 | Yvu410 => (3, 4),
        Yuv420 | Yvu420 => (3, 2),
        Yuv411 | Yvu411 | Yuv422 | Yvu422 | Yuv444 | Yvu444 | Q410 | Q401 => (3, 1),
        _ => (1, 1),
    }
}

/// Checks the planes of a buffer of `height` rows, as wlroots and weston do: indices from 0 to
/// n - 1, at least the colour planes of the format, and each colour plane ending inside its file
/// (`offset + stride * rows`, subsampled rows after the first plane). A metadata plane is only
/// checked for its first row, as smithay does for every plane.
///
/// Smithay 0.7 checks the full height of the plane it receives with index 0 only, then numbers
/// the planes in the order they came: a single plane sent with index 1 escaped every check.
pub fn check_planes(format: Fourcc, height: u32, planes: &[Plane]) -> Result<(), PlaneError> {
    let mut indices: Vec<u32> = planes.iter().map(|p| p.index).collect();
    indices.sort_unstable();
    if indices.is_empty() || indices.iter().zip(0..).any(|(&got, want)| got != want) {
        return Err(PlaneError::Holes);
    }
    let (colour, subsampling) = layout(format);
    if planes.len() < colour {
        return Err(PlaneError::Missing(planes.len(), colour));
    }
    for plane in planes {
        let rows = match plane.index {
            0 => height,
            i if (i as usize) < colour => height.div_ceil(subsampling),
            _ => 1,
        };
        // In u64, nothing overflows: u32 * u32 + u32.
        let end = u64::from(plane.offset) + u64::from(plane.stride) * u64::from(rows);
        match plane.size {
            None => return Err(PlaneError::UnknownSize(plane.index)),
            Some(size) if end > size => return Err(PlaneError::OutOfBounds(plane.index)),
            Some(_) => {}
        }
    }
    Ok(())
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
        let formats: Vec<Format> = formats.into_iter().collect();
        self.dmabuf.announced = formats.iter().map(|f| f.code).collect();
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
        // Checked again here, on what smithay built: the renderer never gets a plane larger
        // than its file, even if the check of the parameters missed a way around it.
        let planes: Vec<Plane> = dmabuf
            .handles()
            .zip(dmabuf.offsets().zip(dmabuf.strides()))
            .zip(0..)
            .map(|((fd, (offset, stride)), index)| Plane::new(fd, index, offset, stride))
            .collect();
        let height = dmabuf.height();
        let result = match check_planes(dmabuf.format().code, height, &planes) {
            Err(err) => Err(err.into()),
            Ok(()) => match self.dmabuf.import.as_mut() {
                Some(import) => import(&dmabuf),
                None => Err("no renderer".into()),
            },
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

// What smithay 0.7.0's `delegate_dmabuf!` writes, except for the buffer parameters, checked
// first below. To review at each smithay update.
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
        // Two holes in smithay 0.7's checks, answered here first. It sends `invalid_dimensions`
        // for a width or a height below 1, but goes on checking the planes: with a height of 0,
        // an offset plus a stride overflows (a panic in a debug build), and the buffer still
        // reaches the importer. And it checks the full height of plane 0 only, by the index the
        // client gives, before numbering the planes again (see `check_planes`).
        use zwp_linux_buffer_params_v1::{Error, Request};
        match &request {
            Request::Add {
                fd,
                plane_idx,
                offset,
                stride,
                ..
            } => {
                let plane = Plane::new(fd.as_fd(), *plane_idx, *offset, *stride);
                state
                    .dmabuf
                    .params
                    .entry(params.id())
                    .or_default()
                    .push(plane);
            }
            Request::Create {
                width,
                height,
                format,
                ..
            }
            | Request::CreateImmed {
                width,
                height,
                format,
                ..
            } => {
                if *width < 1 || *height < 1 {
                    params.post_error(
                        Error::InvalidDimensions,
                        format!("invalid size {width} x {height}"),
                    );
                    return;
                }
                // A format not announced is smithay's to answer (`invalid_format`).
                let code = Fourcc::try_from(*format).ok();
                if let Some(code) = code.filter(|c| state.dmabuf.announced.contains(c)) {
                    let planes = state.dmabuf.params.get(&params.id());
                    let planes = planes.map_or(&[][..], Vec::as_slice);
                    // Height checked above: at least 1.
                    let height = u32::try_from(*height).unwrap_or(0);
                    let error = match check_planes(code, height, planes) {
                        Err(err @ (PlaneError::Holes | PlaneError::Missing(..))) => {
                            Some((Error::Incomplete, err))
                        }
                        Err(err @ PlaneError::OutOfBounds(_)) => Some((Error::OutOfBounds, err)),
                        // Smithay lets it through too; `dmabuf_imported` refuses it.
                        Err(PlaneError::UnknownSize(_)) | Ok(()) => None,
                    };
                    if let Some((error, err)) = error {
                        params.post_error(error, err.to_string());
                        return;
                    }
                }
            }
            _ => {}
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
        state.dmabuf.params.remove(&params.id());
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

    fn plane(index: u32, offset: u32, stride: u32, size: u64) -> Plane {
        Plane {
            index,
            offset,
            stride,
            size: Some(size),
        }
    }

    #[test]
    fn planes_cover_the_buffer_and_stay_in_their_file() {
        let argb = Fourcc::Argb8888;
        // 64 x 48 in ARGB: 256 bytes a row, 12288 in all.
        assert_eq!(check_planes(argb, 48, &[plane(0, 0, 256, 12288)]), Ok(()));
        assert_eq!(
            check_planes(argb, 48, &[plane(0, 0, 256, 12287)]),
            Err(PlaneError::OutOfBounds(0))
        );
        // Index 1 alone: the hole smithay left open.
        assert_eq!(
            check_planes(argb, 48, &[plane(1, 0, 256, 256)]),
            Err(PlaneError::Holes)
        );
        assert_eq!(check_planes(argb, 48, &[]), Err(PlaneError::Holes));
        // A metadata plane after the colour plane (AMD's DCC): its first row only.
        let dcc = [plane(0, 0, 256, 12288), plane(1, 12288, 64, 12352)];
        assert_eq!(check_planes(argb, 48, &dcc), Ok(()));
        // NV12: two planes, the second half as high, in the same file.
        let nv12 = Fourcc::Nv12;
        let both = [plane(0, 0, 64, 4608), plane(1, 3072, 64, 4608)];
        assert_eq!(check_planes(nv12, 48, &both), Ok(()));
        assert_eq!(
            check_planes(nv12, 48, &both[..1]),
            Err(PlaneError::Missing(1, 2))
        );
        let short = [plane(0, 0, 64, 4607), plane(1, 3072, 64, 4607)];
        assert_eq!(
            check_planes(nv12, 48, &short),
            Err(PlaneError::OutOfBounds(1))
        );
        // No overflow, even at the limits.
        assert_eq!(
            check_planes(argb, u32::MAX, &[plane(0, u32::MAX, u32::MAX, u64::MAX)]),
            Ok(())
        );
        let unknown = Plane {
            size: None,
            ..plane(0, 0, 256, 0)
        };
        assert_eq!(
            check_planes(argb, 48, &[unknown]),
            Err(PlaneError::UnknownSize(0))
        );
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
