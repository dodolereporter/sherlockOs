//! Cursor of the real session, drawn by the compositor in the frame (in nested mode, the host
//! window's cursor is used instead).
//!
//! The images come from the user's cursor theme, as other Wayland sessions read it:
//! `XCURSOR_THEME` (`default` otherwise, which points to the system theme) and `XCURSOR_SIZE`
//! (24 otherwise), multiplied by the screen's scale.

use std::collections::HashMap;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
            },
            gles::GlesRenderer,
        },
    },
    input::pointer::CursorIcon,
    utils::{Physical, Point, Transform},
};

/// Size of the cursor at scale 1, in pixels, when `XCURSOR_SIZE` does not set it.
const DEFAULT_SIZE: u32 = 24;

/// An image of the theme and the pixel of its tip.
struct Image {
    buffer: MemoryRenderBuffer,
    hotspot: Point<f64, Physical>,
}

/// The theme and the images already read, by shape and size.
pub struct Cursors {
    theme: xcursor::CursorTheme,
    size: u32,
    images: HashMap<(CursorIcon, u32), Option<Image>>,
}

impl Cursors {
    pub fn load() -> Self {
        let name = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "default".into());
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|s| (1..=256).contains(s))
            .unwrap_or(DEFAULT_SIZE);
        tracing::info!(theme = name, size, "cursor theme");
        Self {
            theme: xcursor::CursorTheme::load(&name),
            size,
            images: HashMap::new(),
        }
    }

    /// The cursor `icon` with its tip at `at`, in frame pixels, on a screen at `scale`.
    /// `None` if the theme has no image for it, not even the arrow.
    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        icon: CursorIcon,
        at: Point<f64, Physical>,
        scale: f64,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        let size = (f64::from(self.size) * scale).round() as u32;
        let theme = &self.theme;
        let image = self
            .images
            .entry((icon, size))
            .or_insert_with(|| {
                let image =
                    read(theme, icon, size).or_else(|| read(theme, CursorIcon::Default, size));
                if image.is_none() {
                    tracing::warn!(?icon, "no cursor image in the theme");
                }
                image
            })
            .as_ref()?;
        let element = MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            at - image.hotspot,
            &image.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        );
        element
            .inspect_err(|err| tracing::warn!("cursor image: {err}"))
            .ok()
    }
}

/// First image of the cursor `icon` (or of one of its other names), the closest to `size`.
fn read(theme: &xcursor::CursorTheme, icon: CursorIcon, size: u32) -> Option<Image> {
    let path = std::iter::once(icon.name())
        .chain(icon.alt_names().iter().copied())
        .find_map(|name| theme.load_icon(name))?;
    let bytes = std::fs::read(path).ok()?;
    let images = xcursor::parser::parse_xcursor(&bytes)?;
    let image = images
        .iter()
        .min_by_key(|image| image.size.abs_diff(size))?;
    // Xcursor pixels are premultiplied ARGB words in the machine's order, as `Argb8888`.
    let buffer = MemoryRenderBuffer::from_slice(
        &image.pixels_rgba,
        Fourcc::Argb8888,
        (image.width as i32, image.height as i32),
        1,
        Transform::Normal,
        None,
    );
    let hotspot = (f64::from(image.xhot), f64::from(image.yhot)).into();
    Some(Image { buffer, hotspot })
}
