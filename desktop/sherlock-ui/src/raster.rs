//! Application icons drawn smooth on the GPU (UI-05).
//!
//! The author saw the icons of the dock, the launcher and the Applications view with rough,
//! « pixelisés » edges, on the GPU at scale 1. iced 0.14's wgpu renderer draws an SVG through
//! three steps that each harm a full-color icon:
//!
//! - it rasterizes it with resvg into a premultiplied pixmap and uploads those pixels as they
//!   are, then blends them as straight alpha (`SrcAlpha`, `OneMinusSrcAlpha`): an edge pixel at
//!   half alpha is darkened twice, and every antialiased edge gets a dark fringe;
//! - it rasterizes at the drawn size rounded up (`ceil`), then samples that raster with its
//!   nearest-neighbour sampler: at a size that is not a whole number of pixels (the dock
//!   magnifying, a fractional scale) rows and columns are doubled or dropped;
//! - it rasterizes again at every new size, so a magnifying dock rasterizes every frame.
//!
//! (A single-color line icon escapes the first: its color replaces the premultiplied color.)
//!
//! `icon` draws the SVG as an image instead: rasterized here by the same resvg, the alpha taken
//! out of the colors (straight alpha, what the image pipeline blends), at twice the logical size
//! rounded up to a step of `STEP` pixels, so that it is never enlarged on a screen up to scale 2
//! and every size of a magnification between two steps shares one raster; drawn with the linear
//! sampler. Each raster is kept, by document and size: no frame rasterizes what an earlier one
//! did.
//!
//! Icon files come from outside the program (any application installs one, `Icon=` may name
//! any file), so they are read with bounds (S-DOCK-2): a regular file of `MAX_SVG_BYTES` (SVG)
//! or `MAX_PICTURE_BYTES` (PNG) at most, a PNG of `MAX_PICTURE_SIDE` pixels a side at most, no
//! compressed SVG (`.svgz`, whose size is unknown until it is inflated), and no `<image>` that
//! names a file: only a PNG written inside the document (`data:`) is drawn. A program that calls
//! `draw_in_background` (the dock, the launcher) never rasterizes a file on its interface
//! thread: a worker thread does, and `drawn` tells the program when to draw again.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, mpsc};

use iced::Subscription;
use iced::futures::channel::mpsc as channel;

use iced::widget::image::{self, FilterMethod, Image};
use iced::widget::svg;

/// How much finer than its logical size an icon is rasterized: exact at scale 2, downsampled
/// by the linear sampler below.
pub const OVERSAMPLE: f32 = 2.0;
/// Raster sizes are whole multiples of this many pixels.
pub const STEP: u32 = 16;
/// The largest raster: 1.7 times the dock's largest icon, at scale 2, fits.
pub const MAX_SIDE: u32 = 512;
/// Rasters kept before the cache starts again from nothing.
const MAX_KEPT: usize = 1024;

/// The largest SVG icon file read: a real icon holds a few tens of kilobytes.
pub const MAX_SVG_BYTES: u64 = 1024 * 1024;
/// The largest picture (PNG) icon file read.
pub const MAX_PICTURE_BYTES: u64 = 4 * 1024 * 1024;
/// The largest side, in pixels, of a picture decoded: a file or a PNG inside an SVG.
pub const MAX_PICTURE_SIDE: u32 = 4096;

/// The bytes of the file at `path`: a regular file once its links are followed, of `max` bytes
/// at most. A link to `/dev/zero` or a huge file is refused rather than read.
fn read_bounded(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// The width and height a PNG's header declares, when it is a PNG of `MAX_PICTURE_SIDE` pixels
/// a side at most.
fn png_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.get(..8)? != PNG_MAGIC || data.get(12..16)? != b"IHDR" {
        return None;
    }
    let number = |at: usize| Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?));
    let (width, height) = (number(16)?, number(20)?);
    let sane = |side: u32| (1..=MAX_PICTURE_SIDE).contains(&side);
    (sane(width) && sane(height)).then_some((width, height))
}

/// Options that never read a file: an `<image>` is drawn only when it is a PNG written in the
/// document, of a sane size; `picture`, when given, answers the `href` « picture » (the PNG of
/// an icon file, wrapped in a document by `rasterize_file`).
fn options(picture: Option<Arc<Vec<u8>>>) -> resvg::usvg::Options<'static> {
    use resvg::usvg::{ImageHrefResolver, ImageKind, Options};
    let resolver = ImageHrefResolver {
        resolve_data: Box::new(|_, data: Arc<Vec<u8>>, _: &Options| {
            png_size(&data).map(|_| ImageKind::PNG(data))
        }),
        resolve_string: Box::new(move |href: &str, _: &Options| {
            (href == "picture")
                .then(|| picture.clone())
                .flatten()
                .map(ImageKind::PNG)
        }),
    };
    Options {
        resources_dir: None,
        image_href_resolver: resolver,
        ..Options::default()
    }
}

/// The pixels of the icon file at `path`, an SVG or a PNG, rasterized `side` by `side` as
/// `rasterize` does. `None` when it is refused (see the module) or cannot be read.
pub fn rasterize_file(path: &Path, side: u32) -> Option<Vec<u8>> {
    let picture = path.extension().is_some_and(|e| e == "png");
    let max = if picture {
        MAX_PICTURE_BYTES
    } else {
        MAX_SVG_BYTES
    };
    let bytes = read_bounded(path, max)?;
    let Some((width, height)) = png_size(&bytes) else {
        return rasterize(&bytes, side);
    };
    let document = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         width=\"{width}\" height=\"{height}\"><image width=\"{width}\" height=\"{height}\" \
         xlink:href=\"picture\"/></svg>"
    );
    rasterize_with(document.as_bytes(), side, &options(Some(Arc::new(bytes))))
}

/// The side of the raster of an icon drawn `side` logical pixels wide.
pub fn raster_side(side: f32) -> u32 {
    let wanted = (side.max(1.0) * OVERSAMPLE / STEP as f32).ceil() as u32 * STEP;
    wanted.clamp(STEP, MAX_SIDE)
}

/// The pixels of `document` rasterized `side` by `side`, its aspect kept and centred, in
/// straight-alpha RGBA. `None` when it cannot be read, or is refused: larger than
/// `MAX_SVG_BYTES`, compressed, or drawing nothing but a picture it would have to read from a
/// file (such an `<image>` is left out).
pub fn rasterize(document: &[u8], side: u32) -> Option<Vec<u8>> {
    rasterize_with(document, side, &options(None))
}

fn rasterize_with(document: &[u8], side: u32, options: &resvg::usvg::Options) -> Option<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    // A compressed document is inflated whole by usvg, whatever its size once inflated.
    if document.len() as u64 > MAX_SVG_BYTES || document.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    let tree = usvg::Tree::from_data(document, options).ok()?;
    let mut pixmap = tiny_skia::Pixmap::new(side, side)?;
    let size = tree.size();
    let fit = (side as f32 / size.width()).min(side as f32 / size.height());
    let (dx, dy) = (
        (side as f32 - size.width() * fit) / 2.0,
        (side as f32 - size.height() * fit) / 2.0,
    );
    let transform = tiny_skia::Transform::from_scale(fit, fit).post_translate(dx, dy);
    // resvg can panic on a malformed file: that icon is left out, the program goes on.
    let drawn = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resvg::render(&tree, transform, &mut pixmap.as_mut());
    }));
    if drawn.is_err() {
        return None;
    }
    let mut pixels: Vec<u8> = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    bleed(&mut pixels, side as usize);
    Some(pixels)
}

/// Gives every transparent pixel the color of its visible neighbours, its alpha left at 0. The
/// linear sampler mixes the colors of neighbouring pixels before their alpha is applied: a
/// transparent pixel left black would darken the edge it borders.
fn bleed(pixels: &mut [u8], side: usize) {
    let source = pixels.to_vec();
    for y in 0..side {
        for x in 0..side {
            let at = (y * side + x) * 4;
            if source[at + 3] != 0 {
                continue;
            }
            let mut sum = [0u32; 3];
            let mut weight = 0u32;
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (nx, ny) = (x as isize + dx, y as isize + dy);
                if nx < 0 || ny < 0 || nx >= side as isize || ny >= side as isize {
                    continue;
                }
                let near = (ny as usize * side + nx as usize) * 4;
                let alpha = u32::from(source[near + 3]);
                for (total, channel) in sum.iter_mut().zip(&source[near..near + 3]) {
                    *total += u32::from(*channel) * alpha;
                }
                weight += alpha;
            }
            for (channel, total) in pixels[at..at + 3].iter_mut().zip(sum) {
                if let Some(mean) = total.checked_div(weight) {
                    *channel = u8::try_from(mean).unwrap_or(u8::MAX);
                }
            }
        }
    }
}

/// The pixels of `handle` at `px`: a document in memory as it is, a file with the bounds of
/// `rasterize_file`.
fn pixels(handle: &svg::Handle, px: u32) -> Option<Vec<u8>> {
    match handle.data() {
        iced::advanced::svg::Data::Bytes(bytes) => rasterize(bytes, px),
        iced::advanced::svg::Data::Path(path) => rasterize_file(path, px),
    }
}

/// `pixels`, each in the color `tint` when one is given, its alpha kept.
fn tinted_pixels(handle: &svg::Handle, px: u32, tint: Option<[u8; 3]>) -> Option<Vec<u8>> {
    let mut drawn = pixels(handle, px);
    if let (Some(drawn), Some(tint)) = (drawn.as_mut(), tint) {
        for pixel in drawn.as_chunks_mut::<4>().0 {
            pixel[..3].copy_from_slice(&tint);
        }
    }
    drawn
}

/// A raster: the document's id, the side in pixels, the tint.
type Key = (u64, u32, Option<[u8; 3]>);

type Cache = Mutex<HashMap<Key, image::Handle>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::default)
}

fn keep(key: Key, drawn: image::Handle) {
    let mut kept = cache().lock().unwrap_or_else(PoisonError::into_inner);
    if kept.len() >= MAX_KEPT {
        kept.clear();
    }
    kept.insert(key, drawn);
}

fn image_of(pixels: Option<Vec<u8>>, px: u32) -> image::Handle {
    match pixels {
        Some(pixels) => image::Handle::from_rgba(px, px, pixels),
        // A document resvg cannot read draws nothing, as iced's own SVG would.
        None => nothing(),
    }
}

/// A transparent pixel: what an icon not drawn yet, or refused, shows.
fn nothing() -> image::Handle {
    static NOTHING: OnceLock<image::Handle> = OnceLock::new();
    NOTHING
        .get_or_init(|| image::Handle::from_rgba(1, 1, vec![0; 4]))
        .clone()
}

/// Whether icon files are rasterized by the worker thread (`draw_in_background`).
static BACKGROUND: AtomicBool = AtomicBool::new(false);

/// From now on, icon files are rasterized on a worker thread, never on the thread that calls
/// `icon`: until it is done, the icon is transparent, then `drawn` sends a message. Documents in
/// memory (the program's own icons) are still drawn at once. Called by the dock and the
/// launcher before they start; tests and other programs keep drawing at once.
pub fn draw_in_background() {
    BACKGROUND.store(true, Ordering::Relaxed);
}

/// The icons asked of the worker, not drawn yet, and how to reach it.
struct Worker {
    jobs: mpsc::Sender<(svg::Handle, u32, Option<[u8; 3]>)>,
    pending: HashSet<Key>,
}

fn worker() -> &'static Mutex<Option<Worker>> {
    static WORKER: OnceLock<Mutex<Option<Worker>>> = OnceLock::new();
    WORKER.get_or_init(|| Mutex::new(None))
}

/// Who waits for icons drawn: one sender per `drawn` subscription.
fn waiters() -> &'static Mutex<Vec<channel::UnboundedSender<()>>> {
    static WAITERS: OnceLock<Mutex<Vec<channel::UnboundedSender<()>>>> = OnceLock::new();
    WAITERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Rasterizes the jobs one after the other, keeps each, then tells the waiters.
fn work(jobs: mpsc::Receiver<(svg::Handle, u32, Option<[u8; 3]>)>) {
    for (handle, px, tint) in jobs {
        let key = (handle.id(), px, tint);
        keep(key, image_of(tinted_pixels(&handle, px, tint), px));
        if let Some(worker) = worker()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_mut()
        {
            worker.pending.remove(&key);
        }
        waiters()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|waiter| waiter.unbounded_send(()).is_ok());
    }
}

/// Asks the worker for `handle` at `px`, once; starts it the first time. `false` when the
/// thread cannot be started (then the icon is drawn at once).
fn ask(handle: &svg::Handle, px: u32, tint: Option<[u8; 3]>) -> bool {
    let key = (handle.id(), px, tint);
    let mut slot = worker().lock().unwrap_or_else(PoisonError::into_inner);
    if slot.is_none() {
        let (jobs, receiver) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("icons".into())
            .spawn(move || work(receiver));
        if started.is_err() {
            return false;
        }
        *slot = Some(Worker {
            jobs,
            pending: HashSet::new(),
        });
    }
    let Some(worker) = slot.as_mut() else {
        return false;
    };
    if worker.pending.insert(key) && worker.jobs.send((handle.clone(), px, tint)).is_err() {
        worker.pending.remove(&key);
        return false;
    }
    true
}

fn waiting() -> channel::UnboundedReceiver<()> {
    let (sender, receiver) = channel::unbounded();
    waiters()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(sender);
    receiver
}

/// A message each time the worker has drawn an icon: the program draws again, and the icon,
/// transparent until then, appears.
pub fn drawn() -> Subscription<()> {
    Subscription::run(waiting)
}

/// The image of `handle` at the raster side of `side`, rasterized once. In the background for
/// a file when `draw_in_background` was called: transparent until it is drawn.
pub fn raster(handle: &svg::Handle, side: f32) -> image::Handle {
    raster_in(handle, side, None)
}

/// The image of a single-color `handle` (a symbolic icon of a theme) in the color `tint`, its
/// own color left out and its alpha kept: the icon follows the theme, light or dark (lot D-45).
pub fn tinted(handle: &svg::Handle, side: f32, tint: crate::color::Color) -> image::Handle {
    raster_in(handle, side, Some([tint.r, tint.g, tint.b]))
}

fn raster_in(handle: &svg::Handle, side: f32, tint: Option<[u8; 3]>) -> image::Handle {
    let px = raster_side(side);
    let key = (handle.id(), px, tint);
    if let Some(found) = cache()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
    {
        return found.clone();
    }
    let file = matches!(handle.data(), iced::advanced::svg::Data::Path(_));
    if file && BACKGROUND.load(Ordering::Relaxed) && ask(handle, px, tint) {
        return nothing();
    }
    let drawn = image_of(tinted_pixels(handle, px, tint), px);
    keep(key, drawn.clone());
    drawn
}

/// An application icon, `side` logical pixels wide, smooth at any size and scale: see the
/// module. Its opacity is set like any image's (`.opacity(..)`).
pub fn icon(handle: &svg::Handle, side: f32) -> Image<image::Handle> {
    iced::widget::image(raster(handle, side))
        .width(side)
        .height(side)
        .filter_method(FilterMethod::Linear)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TILE: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\
        <rect x=\"2\" y=\"2\" width=\"20\" height=\"20\" rx=\"5\" fill=\"#F4EBE3\"/></svg>";

    #[test]
    fn rasters_are_twice_the_size_in_steps() {
        assert_eq!(raster_side(52.0), 112);
        // A magnifying dock: every size between two steps shares one raster.
        assert_eq!(raster_side(52.5), raster_side(55.0));
        assert_eq!(raster_side(88.4), 192);
        assert_eq!(raster_side(0.0), STEP);
        assert_eq!(raster_side(10_000.0), MAX_SIDE);
    }

    #[test]
    fn edges_are_straight_alpha_without_a_dark_fringe() {
        let Some(pixels) = rasterize(TILE.as_bytes(), 64) else {
            panic!("the tile rasterizes");
        };
        // Every pixel with some alpha keeps the cream of the tile, however transparent: the
        // alpha is not multiplied into the color.
        let mut edges = 0;
        for pixel in pixels.chunks(4) {
            let alpha = pixel[3];
            if alpha > 8 && alpha < 247 {
                edges += 1;
                assert!(pixel[0] >= 0xF0 && pixel[1] >= 0xE4, "{pixel:?}");
            }
        }
        assert!(edges > 20, "the edges are antialiased: {edges}");
    }

    /// A folder of its own for a test, removed when dropped.
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("sherlock-ui-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("folder");
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A 2 by 1 PNG, red then transparent, written by hand (stored deflate blocks).
    fn png(width: u32, height: u32) -> Vec<u8> {
        fn crc(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
            out.extend((data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend(data);
            out.extend(&body);
            out.extend(crc(&body).to_be_bytes());
        }
        // Each row: its filter byte (none), then red for the first pixel, transparent after.
        let row: Vec<u8> = std::iter::once(0)
            .chain((0..width).flat_map(|x| {
                if x == 0 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 0, 0]
                }
            }))
            .collect();
        let raw: Vec<u8> = (0..height).flat_map(|_| row.iter().copied()).collect();
        let (mut a, mut b) = (1u32, 0u32);
        for byte in &raw {
            a = (a + u32::from(*byte)) % 65521;
            b = (b + a) % 65521;
        }
        let mut zlib = vec![0x78, 0x01, 0x01];
        let len = raw.len() as u16;
        zlib.extend(len.to_le_bytes());
        zlib.extend((!len).to_le_bytes());
        zlib.extend(&raw);
        zlib.extend(((b << 16) | a).to_be_bytes());
        let mut out = PNG_MAGIC.to_vec();
        let mut header = Vec::new();
        header.extend(width.to_be_bytes());
        header.extend(height.to_be_bytes());
        header.extend([8, 6, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &zlib);
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn icon_files_are_bounded_and_read_nothing_else() {
        let folder = Folder::new("icons");
        let picture = folder.0.join("app.png");
        std::fs::write(&picture, png(2, 1)).expect("written");
        // A PNG file: drawn in its proportions, the red half on the left.
        let pixels = rasterize_file(&picture, 16).expect("drawn");
        let at = |x: usize, y: usize| &pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(at(2, 8), [255, 0, 0, 255]);
        // The right half, transparent (smoothed at the middle by the scaling).
        assert!(at(14, 8)[3] < 16, "{:?}", at(14, 8));
        assert_eq!(at(2, 1)[3], 0, "letterboxed");
        // An SVG that names that file, or any other, draws nothing of it.
        let naming = folder.0.join("naming.svg");
        let href = picture.to_string_lossy();
        std::fs::write(
            &naming,
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
                 width=\"4\" height=\"4\"><image width=\"4\" height=\"4\" xlink:href=\"{href}\"/>\
                 <image width=\"4\" height=\"4\" xlink:href=\"picture\"/></svg>"
            ),
        )
        .expect("written");
        let drawn = rasterize_file(&naming, 16).unwrap_or_default();
        assert!(
            drawn.chunks(4).all(|p| p[3] == 0),
            "nothing read from a file"
        );
        // A PNG written in the document is drawn; one claiming a huge size is not.
        assert!(png_size(&png(2, 1)).is_some());
        assert_eq!(png_size(&png(MAX_PICTURE_SIDE + 1, 1)), None);
        // Files too large, compressed documents, devices: refused.
        let large = folder.0.join("large.svg");
        let padding = " ".repeat(MAX_SVG_BYTES as usize);
        std::fs::write(&large, format!("{TILE}{padding}")).expect("written");
        assert!(rasterize_file(&large, 16).is_none());
        assert!(rasterize(&[0x1f, 0x8b, 8, 0], 16).is_none());
        let zero = folder.0.join("zero.svg");
        std::os::unix::fs::symlink("/dev/zero", &zero).expect("link");
        assert!(rasterize_file(&zero, 16).is_none());
    }

    #[test]
    fn files_are_drawn_in_the_background_once_asked() {
        let folder = Folder::new("background");
        let path = folder.0.join("tile.svg");
        std::fs::write(&path, TILE).expect("written");
        draw_in_background();
        let mut drawn = waiting();
        let handle = svg::Handle::from_path(&path);
        // At first, nothing: the worker draws it.
        assert_eq!(raster(&handle, 30.0).id(), nothing().id());
        let mut waited = 0;
        while drawn.try_recv().is_err() && waited < 500 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
        }
        let image = raster(&handle, 30.0);
        assert_ne!(image.id(), nothing().id(), "drawn by the worker");
        assert_eq!(image.id(), raster(&handle, 30.0).id());
        // Documents in memory are still drawn at once.
        let memory = svg::Handle::from_memory(TILE.as_bytes());
        assert_ne!(raster(&memory, 70.0).id(), nothing().id());
    }

    #[test]
    fn a_raster_is_made_once_per_size() {
        let handle = svg::Handle::from_memory(TILE.as_bytes());
        let first = raster(&handle, 52.0);
        assert_eq!(first.id(), raster(&handle, 52.0).id());
        assert_eq!(first.id(), raster(&handle, 53.0).id(), "the same step");
        assert_ne!(first.id(), raster(&handle, 90.0).id());
        assert!(rasterize(b"not an svg", 32).is_none());
    }

    #[test]
    fn a_symbolic_icon_takes_the_tint_and_keeps_its_alpha() {
        let handle = svg::Handle::from_memory(TILE.as_bytes());
        let tint = crate::color::Color::rgb(0x14, 0x23, 0x3B);
        let plain = raster(&handle, 20.0);
        let tinted = tinted(&handle, 20.0, tint);
        assert_ne!(plain.id(), tinted.id());
        assert_eq!(tinted.id(), super::tinted(&handle, 20.0, tint).id());
        let Some(pixels) = rasterize(TILE.as_bytes(), raster_side(20.0)) else {
            panic!("the tile rasterizes");
        };
        let iced::advanced::image::Handle::Rgba { pixels: drawn, .. } = tinted else {
            panic!("pixels");
        };
        for (shown, source) in drawn.chunks(4).zip(pixels.chunks(4)) {
            assert_eq!(shown[3], source[3]);
            assert_eq!(&shown[..3], &[0x14, 0x23, 0x3B]);
        }
    }
}
