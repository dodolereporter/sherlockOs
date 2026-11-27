//! What the history reads of a new selection, in a thread of its own: the one type worth
//! keeping, read with a size limit and a deadline, then sorted (text, link, files, picture)
//! and, for a picture, made into a thumbnail. Nothing here runs on the compositor's loop.

use std::{
    io::{self, Read as _},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::model::{Content, Held, Kind};

/// Longest wait for the owner of the selection to write it all.
const DEADLINE: Duration = Duration::from_secs(3);

/// Pause between two looks at an empty pipe.
const TICK: Duration = Duration::from_millis(5);

/// Reads under way at most: a client that never writes holds one until the deadline.
pub(super) const MAX_READS: usize = 4;

/// Longest text kept, in bytes: a longer one is not kept at all, never cut.
pub const MAX_TEXT: usize = 1 << 20;

/// Longest list of files, in bytes.
const MAX_URIS: usize = 256 << 10;

/// Largest picture kept, encoded.
pub const MAX_IMAGE: usize = 16 << 20;

/// Largest picture decoded for its thumbnail, in bytes of pixels.
const MAX_DECODED: u64 = 256 << 20;

/// Longest side of a thumbnail, in pixels: sharp at scale 2 in the panel.
pub const THUMBNAIL: u32 = 160;

/// Characters of the preview at most, and lines.
pub const PREVIEW_CHARS: usize = 300;
const PREVIEW_LINES: usize = 4;

/// Files named in the preview at most.
const PREVIEW_FILES: usize = 20;

const TEXT_TYPES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];
const URI_LIST: &str = "text/uri-list";
const GNOME_FILES: &str = "x-special/gnome-copied-files";
const IMAGE_TYPES: [&str; 2] = ["image/png", "image/jpeg"];

/// The type worth reading among those offered: files first, then a picture, then text.
pub fn choose(types: &[String]) -> Option<&'static str> {
    let offered = |t: &str| types.iter().any(|o| o == t);
    std::iter::once(URI_LIST)
        .chain(IMAGE_TYPES)
        .chain(TEXT_TYPES)
        .find(|t| offered(t))
}

/// The applications that copy secrets, by app_id or X11 class, lowercase: what they copy is
/// never kept, even without the hint (an X11 application cannot pass it, see `is_secret`).
const PASSWORD_MANAGERS: [&str; 9] = [
    "keepass",
    "bitwarden",
    "1password",
    "enpass",
    "org.gnome.world.secrets",
    "gnome-passwordsafe",
    "protonpass",
    "proton-pass",
    "lastpass",
];

/// `app` is a password manager.
pub fn is_password_manager(app: &str) -> bool {
    let app = app.to_lowercase();
    PASSWORD_MANAGERS.iter().any(|p| app.contains(p))
}

/// Reads `pipe` in a thread, then sorts what came as `mime`; `done` gets the content, or
/// `None` when nothing is to be kept. Nothing is read when `MAX_READS` reads are under way.
pub(super) fn spawn(
    pipe: io::PipeReader,
    mime: &'static str,
    reads: Arc<AtomicUsize>,
    done: impl FnOnce(Option<Content>) + Send + 'static,
) {
    if reads.fetch_add(1, Ordering::AcqRel) >= MAX_READS {
        reads.fetch_sub(1, Ordering::AcqRel);
        tracing::debug!("clipboard history: too many reads under way, copy not kept");
        return;
    }
    let ended = reads.clone();
    let spawned = thread::Builder::new()
        .name("clipboard-history".into())
        .spawn(move || {
            let content = read_all(pipe, limit(mime)).and_then(|bytes| sort(mime, bytes));
            ended.fetch_sub(1, Ordering::AcqRel);
            done(content);
        });
    if let Err(err) = spawned {
        reads.fetch_sub(1, Ordering::AcqRel);
        tracing::warn!("clipboard history: no thread to read the copy: {err}");
    }
}

fn limit(mime: &str) -> usize {
    match mime {
        URI_LIST => MAX_URIS,
        m if IMAGE_TYPES.contains(&m) => MAX_IMAGE,
        _ => MAX_TEXT,
    }
}

/// Everything the pipe gives before its end, at most `limit` bytes and before the deadline;
/// `None` past either.
fn read_all(pipe: io::PipeReader, limit: usize) -> Option<Vec<u8>> {
    let pipe = std::os::fd::OwnedFd::from(pipe);
    if let Err(err) = smithay::reexports::rustix::fs::fcntl_setfl(
        &pipe,
        smithay::reexports::rustix::fs::OFlags::NONBLOCK,
    ) {
        tracing::debug!("clipboard history: pipe left blocking: {err}");
        return None;
    }
    let mut file = std::fs::File::from(pipe);
    let deadline = Instant::now() + DEADLINE;
    let mut bytes = Vec::new();
    let mut piece = [0u8; 64 << 10];
    loop {
        match file.read(&mut piece) {
            Ok(0) => return Some(bytes),
            Ok(n) => {
                if bytes.len() + n > limit {
                    tracing::debug!(limit, "clipboard history: copy too large, not kept");
                    return None;
                }
                bytes.extend_from_slice(&piece[..n]);
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    tracing::debug!("clipboard history: the owner did not write in time");
                    return None;
                }
                thread::sleep(TICK);
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => {
                tracing::debug!("clipboard history: {err}");
                return None;
            }
        }
    }
}

/// What came as `mime`, sorted; `None` for nothing worth keeping (empty, not UTF-8, a picture
/// that does not decode).
pub fn sort(mime: &str, bytes: Vec<u8>) -> Option<Content> {
    if IMAGE_TYPES.contains(&mime) {
        return picture(mime, bytes);
    }
    let text = String::from_utf8(bytes).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    if mime == URI_LIST {
        return uris(&text);
    }
    Some(plain(text))
}

fn plain(text: String) -> Content {
    let trimmed = text.trim();
    let kind = match is_link(trimmed) {
        true => Kind::Link,
        false => Kind::Text,
    };
    let preview = preview(&text);
    let bytes: Arc<[u8]> = Arc::from(text.into_bytes());
    let held = Held::new(&TEXT_TYPES, bytes.clone());
    Content::new(kind, held, &bytes, preview)
}

/// One web address alone.
fn is_link(text: &str) -> bool {
    (text.starts_with("https://") || text.starts_with("http://"))
        && !text.chars().any(char::is_whitespace)
}

/// The first lines of `text`, without the blank ones before.
pub fn preview(text: &str) -> String {
    let mut out = String::new();
    let lines = text.trim_start().lines().take(PREVIEW_LINES);
    for (i, line) in lines.enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(line.trim_end());
    }
    match out.char_indices().nth(PREVIEW_CHARS) {
        Some((at, _)) => out[..at].to_owned(),
        None => out,
    }
}

/// A `text/uri-list`: files when every address is a local file, else text.
fn uris(text: &str) -> Option<Content> {
    let list: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if list.is_empty() {
        return None;
    }
    if !list.iter().all(|u| u.starts_with("file://")) {
        return Some(plain(list.join("\n")));
    }
    let names: Vec<String> = list
        .iter()
        .take(PREVIEW_FILES)
        .map(|u| decode(u.trim_start_matches("file://")))
        .collect();
    let joined = list.join("\r\n") + "\r\n";
    let uri_list: Arc<[u8]> = Arc::from(joined.as_bytes());
    let gnome: Arc<[u8]> = Arc::from(format!("copy\n{}", list.join("\n")).into_bytes());
    let paths: Arc<[u8]> = Arc::from(names.join("\n").into_bytes());
    let held = Held::new(&[URI_LIST], uri_list.clone())
        .with(GNOME_FILES, gnome)
        .with(TEXT_TYPES[0], paths);
    Some(Content::new(Kind::Files, held, &uri_list, names.join("\n")))
}

/// A percent-encoded path, decoded; what does not decode stays as it was.
fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| path.to_owned())
}

/// A picture: kept as it came, with its size as preview and a thumbnail. The size is checked
/// before anything is decoded.
fn picture(mime: &str, bytes: Vec<u8>) -> Option<Content> {
    let format = match mime {
        "image/png" => image::ImageFormat::Png,
        _ => image::ImageFormat::Jpeg,
    };
    let mut reader = image::ImageReader::with_format(io::Cursor::new(&bytes[..]), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED);
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    reader.limits(limits);
    let decoded = match reader.decode() {
        Ok(decoded) => decoded,
        Err(err) => {
            tracing::debug!("clipboard history: picture not kept: {err}");
            return None;
        }
    };
    let (width, height) = (decoded.width(), decoded.height());
    // Made smaller only: a small picture stays as it is.
    let small = match width.max(height) > THUMBNAIL {
        true => decoded.thumbnail(THUMBNAIL, THUMBNAIL),
        false => decoded,
    };
    let mut thumbnail = Vec::new();
    if let Err(err) = small.write_to(
        &mut io::Cursor::new(&mut thumbnail),
        image::ImageFormat::Png,
    ) {
        tracing::debug!("clipboard history: no thumbnail: {err}");
        return None;
    }
    let bytes: Arc<[u8]> = Arc::from(bytes);
    let preview = format!("{width} × {height}");
    let content = Content::new(
        Kind::Image,
        Held::new(&[mime], bytes.clone()),
        &bytes,
        preview,
    );
    Some(content.with_thumbnail(Arc::from(thumbnail)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn files_come_before_a_picture_and_a_picture_before_text() {
        let offered = types(&["text/plain", "image/png", "text/html"]);
        assert_eq!(choose(&offered), Some("image/png"));
        let offered = types(&["text/plain;charset=utf-8", URI_LIST, "image/png"]);
        assert_eq!(choose(&offered), Some(URI_LIST));
        assert_eq!(choose(&types(&["text/plain"])), Some("text/plain"));
        assert_eq!(choose(&types(&["text/html", "application/x-foo"])), None);
    }

    #[test]
    fn text_links_and_files_are_told_apart() {
        let text = sort("text/plain", b"  hello\nworld".to_vec()).unwrap();
        assert_eq!(text.kind, Kind::Text);
        assert_eq!(text.preview, "hello\nworld");
        assert_eq!(
            text.held.bytes("text/plain;charset=utf-8").as_deref(),
            Some(&b"  hello\nworld"[..])
        );
        let link = sort("text/plain", b"https://example.org/a?b=c\n".to_vec()).unwrap();
        assert_eq!(link.kind, Kind::Link);
        let list = b"file:///home/u/My%20File.txt\r\nfile:///tmp/b.png\r\n".to_vec();
        let files = sort(URI_LIST, list).unwrap();
        assert_eq!(files.kind, Kind::Files);
        assert_eq!(files.preview, "/home/u/My File.txt\n/tmp/b.png");
        assert!(files.held.bytes(GNOME_FILES).is_some());
        let web = sort(URI_LIST, b"https://example.org\n".to_vec()).unwrap();
        assert_eq!(web.kind, Kind::Link);
    }

    #[test]
    fn nothing_empty_nor_broken_is_kept() {
        assert!(sort("text/plain", b"  \n ".to_vec()).is_none());
        assert!(sort("text/plain", vec![0xff, 0xfe]).is_none());
        assert!(sort("image/png", b"not a picture".to_vec()).is_none());
    }

    #[test]
    fn the_preview_is_short() {
        let long = "a".repeat(1000);
        assert_eq!(preview(&long).chars().count(), PREVIEW_CHARS);
        assert_eq!(preview("1\n2\n3\n4\n5\n6"), "1\n2\n3\n4");
        assert_eq!(
            preview("é".repeat(400).as_str()).chars().count(),
            PREVIEW_CHARS
        );
    }

    #[test]
    fn a_picture_gets_a_small_thumbnail() {
        let mut png = Vec::new();
        image::RgbaImage::new(640, 320)
            .write_to(&mut io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let content = sort("image/png", png.clone()).unwrap();
        assert_eq!(content.kind, Kind::Image);
        assert_eq!(content.preview, "640 × 320");
        assert_eq!(content.held.bytes("image/png").as_deref(), Some(&png[..]));
        let thumbnail = image::load_from_memory(content.thumbnail.as_deref().unwrap()).unwrap();
        assert_eq!(
            (thumbnail.width(), thumbnail.height()),
            (THUMBNAIL, THUMBNAIL / 2)
        );
    }

    #[test]
    fn password_managers_are_known() {
        for app in [
            "org.keepassxc.KeePassXC",
            "KeePassXC",
            "Bitwarden",
            "1Password",
        ] {
            assert!(is_password_manager(app), "{app}");
        }
        assert!(!is_password_manager("org.kde.kate"));
    }

    #[test]
    fn a_pipe_too_long_or_silent_gives_nothing() {
        let (reader, mut writer) = io::pipe().unwrap();
        std::io::Write::write_all(&mut writer, b"abc").unwrap();
        drop(writer);
        assert_eq!(read_all(reader, 10), Some(b"abc".to_vec()));
        let (reader, mut writer) = io::pipe().unwrap();
        std::io::Write::write_all(&mut writer, b"abcdef").unwrap();
        drop(writer);
        assert_eq!(read_all(reader, 4), None);
        // Never closed: the deadline ends the read.
        let (reader, _writer) = io::pipe().unwrap();
        let started = Instant::now();
        assert_eq!(read_all(reader, 4), None);
        assert!(started.elapsed() >= DEADLINE);
    }
}
