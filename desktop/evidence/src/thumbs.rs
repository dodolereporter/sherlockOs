//! Thumbnails (lot E-14): pictures (PNG, JPEG, WebP, GIF, through the bounded decoding of the
//! preview), the first page of a PDF (`pdftoppm`, when installed), a frame of a video
//! (`ffmpegthumbnailer`, when installed). They are kept in the freedesktop cache shared with
//! the other file managers, `$XDG_CACHE_HOME/thumbnails/large`, under the MD5 of the file's
//! URI, with the `Thumb::URI` and `Thumb::MTime` keys that tell whether one is still good.
//! Everything here runs off the interface thread, bounded in size and in time.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use iced::widget::image::Handle;

/// Side of a « large » thumbnail of the specification.
pub const SIDE: u32 = 256;
/// Largest thumbnail file read from the cache.
const MAX_CACHED: u64 = 4 * 1024 * 1024;
/// Time an outside tool gets to make one.
const TOOL_TIME: Duration = Duration::from_secs(5);

/// What a thumbnail is made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Picture,
    Pdf,
    Video,
}

/// The source of a file's thumbnail, from its extension; none for anything else.
pub fn source(path: &Path) -> Option<Source> {
    let extension = path.extension()?.to_str()?.to_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" | "gif" => Some(Source::Picture),
        "pdf" => Some(Source::Pdf),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "ogv" => Some(Source::Video),
        _ => None,
    }
}

// The URI and its MD5, as the specification names the files.

/// `file://` and the path, every byte but the unreserved ones and `/` percent-encoded.
pub fn uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(byte));
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// MD5 (RFC 1321): only to name the cache's files, as the specification asks; never for
/// security.
pub fn md5(input: &[u8]) -> [u8; 16] {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let constants: Vec<u32> = (0..64)
        .map(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut message = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_le_bytes());
    let mut state: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for block in message.as_chunks::<64>().0 {
        let words: Vec<u32> = block
            .as_chunks::<4>()
            .0
            .iter()
            .map(|w| u32::from_le_bytes(*w))
            .collect();
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated = a
                .wrapping_add(f)
                .wrapping_add(constants[i])
                .wrapping_add(words[g])
                .rotate_left(SHIFTS[i]);
            (a, d, c) = (d, c, b);
            b = b.wrapping_add(rotated);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    let mut digest = [0u8; 16];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

/// Where the thumbnail of `uri` is kept in the cache `root`.
pub fn cached_path(root: &Path, uri: &str) -> PathBuf {
    let name: String = md5(uri.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    root.join("large").join(format!("{name}.png"))
}

// The PNG text chunks the specification keeps in a thumbnail.

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
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

/// `png` with `tEXt` chunks for `keys` put after its header chunk.
fn with_texts(png: &[u8], keys: &[(&str, &str)]) -> Option<Vec<u8>> {
    // The signature (8 bytes), then IHDR: length, type, 13 bytes, CRC.
    let header_end = 8 + 4 + 4 + 13 + 4;
    let mut out = png.get(..header_end)?.to_vec();
    for (key, value) in keys {
        let mut chunk = b"tEXt".to_vec();
        chunk.extend_from_slice(key.as_bytes());
        chunk.push(0);
        chunk.extend_from_slice(value.as_bytes());
        let length = u32::try_from(chunk.len() - 4).ok()?;
        out.extend_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&chunk);
        out.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    out.extend_from_slice(png.get(header_end..)?);
    Some(out)
}

/// The value of the `tEXt` chunk `key` of `png`, if any.
fn text_of<'a>(png: &'a [u8], key: &str) -> Option<&'a str> {
    let mut at = 8;
    while at + 8 <= png.len() {
        let length = u32::from_be_bytes(png.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = png.get(at + 4..at + 8)?;
        let data = png.get(at + 8..at + 8 + length)?;
        if kind == b"IDAT" || kind == b"IEND" {
            return None;
        }
        if kind == b"tEXt"
            && let Some(rest) = data.strip_prefix(key.as_bytes())
            && let Some(value) = rest.strip_prefix(&[0])
        {
            return std::str::from_utf8(value).ok();
        }
        at += 12 + length;
    }
    None
}

/// Seconds of a modification date, as `Thumb::MTime` holds them.
fn seconds(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A picture decoded with the bounds of the preview, made at most `SIDE` on its long side.
fn decode(bytes_or_path: &Path) -> Option<image::RgbaImage> {
    let file = crate::preview::open_regular(bytes_or_path)?;
    let reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if !crate::preview::image_fits(width, height) {
        return None;
    }
    let file = crate::preview::open_regular(bytes_or_path)?;
    let mut reader = image::ImageReader::new(std::io::BufReader::new(file))
        .with_guessed_format()
        .ok()?;
    reader.limits(crate::preview::image_limits());
    Some(reader.decode().ok()?.thumbnail(SIDE, SIDE).to_rgba8())
}

/// The thumbnail kept for `uri`, if it is still that of the file modified at `mtime`.
fn read_cached(path: &Path, uri: &str, mtime: u64) -> Option<image::RgbaImage> {
    use std::io::Read;
    let file = crate::preview::open_regular(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_CACHED).read_to_end(&mut bytes).ok()?;
    if text_of(&bytes, "Thumb::URI")? != uri
        || text_of(&bytes, "Thumb::MTime")?.parse::<u64>().ok()? != mtime
    {
        return None;
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(crate::preview::image_limits());
    Some(reader.decode().ok()?.to_rgba8())
}

/// Keeps a thumbnail: written beside, then renamed, readable by its owner only, as the
/// specification asks.
fn write_cached(path: &Path, uri: &str, mtime: u64, picture: &image::RgbaImage) -> Option<()> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let dir = path.parent()?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .ok()?;
    let mut png = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    let mtime = mtime.to_string();
    let png = with_texts(&png, &[("Thumb::URI", uri), ("Thumb::MTime", &mtime)])?;
    let temporary = dir.join(format!(".evidence-{}.png", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .ok()?;
    file.write_all(&png).ok()?;
    drop(file);
    std::fs::rename(&temporary, path).ok()
}

/// Runs an outside tool for `TOOL_TIME` at most; whether it ended well.
fn run(command: &mut Command) -> bool {
    let Ok(mut child) = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if start.elapsed() > TOOL_TIME => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(_) => return false,
        }
    }
}

/// A folder of its own for an outside tool's picture, removed after.
fn scratch() -> Option<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("evidence-thumb-{}-{n}", std::process::id()));
    std::fs::create_dir(&dir).ok()?;
    Some(dir)
}

/// Makes the thumbnail of `path`.
fn make(path: &Path, source: Source) -> Option<image::RgbaImage> {
    let side = SIDE.to_string();
    let (dir, made, done) = match source {
        Source::Picture => return decode(path),
        Source::Pdf => {
            let dir = scratch()?;
            let done = run(Command::new("pdftoppm")
                .args([
                    "-png",
                    "-singlefile",
                    "-f",
                    "1",
                    "-l",
                    "1",
                    "-scale-to",
                    &side,
                ])
                .arg(path)
                .arg(dir.join("page")));
            (dir.clone(), dir.join("page.png"), done)
        }
        Source::Video => {
            let dir = scratch()?;
            let done = run(Command::new("ffmpegthumbnailer")
                .arg("-i")
                .arg(path)
                .arg("-o")
                .arg(dir.join("frame.png"))
                .args(["-s", &side, "-c", "png"]));
            (dir.clone(), dir.join("frame.png"), done)
        }
    };
    let picture = done.then(|| decode(&made)).flatten();
    let _ = std::fs::remove_dir_all(dir);
    picture
}

/// The thumbnail of `path`, modified at `modified`: the one kept in `cache` if still good,
/// else made and kept there. Without a cache, or for a file of the cache itself, it is only
/// made.
pub fn thumbnail(
    path: &Path,
    modified: Option<SystemTime>,
    cache: Option<&Path>,
) -> Option<Handle> {
    let source = source(path)?;
    let uri = uri(path);
    let mtime = seconds(modified?);
    let cache = cache.filter(|root| !path.starts_with(root));
    let kept = cache.map(|root| cached_path(root, &uri));
    let picture = match kept
        .as_deref()
        .and_then(|file| read_cached(file, &uri, mtime))
    {
        Some(picture) => picture,
        None => {
            let picture = make(path, source)?;
            if let Some(file) = &kept
                && write_cached(file, &uri, mtime, &picture).is_none()
            {
                tracing::debug!("a thumbnail is not kept");
            }
            picture
        }
    };
    let (width, height) = picture.dimensions();
    Some(Handle::from_rgba(width, height, picture.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn md5_and_uri_name_the_files_as_the_specification() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        let long =
            b"12345678901234567890123456789012345678901234567890123456789012345678901234567890";
        assert_eq!(hex(md5(long)), "57edf4a22be3c955ac49da2e2107b67a");
        // The example of the specification.
        assert_eq!(
            uri(Path::new("/home/jens/photos/me.png")),
            "file:///home/jens/photos/me.png"
        );
        assert_eq!(
            hex(md5(b"file:///home/jens/photos/me.png")),
            "c6ee772d9e49320e97ec29a7eb5b1697"
        );
        assert_eq!(
            uri(Path::new("/tmp/a b/é.png")),
            "file:///tmp/a%20b/%C3%A9.png"
        );
    }

    #[test]
    fn a_thumbnail_is_kept_and_found_again_until_the_file_changes() {
        let tmp = TempDir::new("thumbs");
        let picture = tmp.path().join("photo.png");
        image::RgbaImage::from_pixel(600, 300, image::Rgba([200, 30, 30, 255]))
            .save(&picture)
            .expect("write");
        let cache = tmp.path().join("cache/thumbnails");
        let modified = std::fs::metadata(&picture).and_then(|m| m.modified()).ok();
        assert!(thumbnail(&picture, modified, Some(&cache)).is_some());
        let kept = cached_path(&cache, &uri(&picture));
        let bytes = std::fs::read(&kept).expect("kept");
        assert_eq!(text_of(&bytes, "Thumb::URI"), Some(uri(&picture).as_str()));
        let found = read_cached(&kept, &uri(&picture), seconds(modified.expect("date")));
        assert_eq!(found.map(|p| p.dimensions()), Some((SIDE, SIDE / 2)));
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&kept).expect("kept").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Another date: not that file any more.
        assert!(read_cached(&kept, &uri(&picture), 1).is_none());
        // Not a thumbnail source, or a missing tool: nothing, and no wait.
        let text = tmp.path().join("notes.txt");
        std::fs::write(&text, "x").expect("write");
        assert!(thumbnail(&text, modified, Some(&cache)).is_none());
        // A GIF too, without a cache.
        let gif = tmp.path().join("anim.gif");
        image::RgbaImage::from_pixel(40, 80, image::Rgba([0, 90, 200, 255]))
            .save(&gif)
            .expect("write");
        let shown = thumbnail(&gif, modified, None).expect("a thumbnail");
        let _ = shown;
        assert_eq!(source(Path::new("film.MKV")), Some(Source::Video));
        assert_eq!(source(Path::new("doc.pdf")), Some(Source::Pdf));
    }
}
