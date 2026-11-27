//! Pictures named by a path in a notification, read by the daemon itself: never a device or a
//! FIFO, never more than a few hundred kibibytes, and only PNG or an SVG that loads nothing
//! else. Iced then only ever gets bytes, never a path.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Longest path taken, in bytes: the `PATH_MAX` of Linux.
pub const MAX_PATH_BYTES: usize = 4096;
/// Largest PNG file read, and its largest side in pixels.
pub const MAX_PNG_BYTES: u64 = 512 * 1024;
pub const MAX_PNG_SIDE: u32 = 512;
/// Largest SVG file read.
pub const MAX_SVG_BYTES: u64 = 256 * 1024;
/// Elements an SVG may hold, and `<use>` among them (Q-38). resvg draws on the bubbles'
/// interface thread, and `<use>` copies what it names: 40 of them nested two by two make a
/// million shapes in a few kibibytes. Measured in `tests/svg_cost.rs`: 262 144 shapes took
/// 1.2 s; within these bounds, at most a few thousand shapes are drawn.
pub const MAX_SVG_ELEMENTS: usize = 4096;
pub const MAX_SVG_USES: usize = 16;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A picture checked and held in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Svg,
}

/// The picture at `path`, when it is a PNG or a self-contained SVG within the limits.
pub fn read(path: &Path) -> io::Result<(Kind, Vec<u8>)> {
    let svg = path.extension().is_some_and(|e| e == "svg");
    let limit = if svg { MAX_SVG_BYTES } else { MAX_PNG_BYTES };
    let bytes = read_regular(path, limit)?;
    if svg {
        check_svg(&bytes)?;
        Ok((Kind::Svg, bytes))
    } else {
        check_png(&bytes)?;
        Ok((Kind::Png, bytes))
    }
}

fn refused(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_owned())
}

/// The bytes of the file at `path`, when it is a regular file of at most `limit` bytes. It is
/// opened without blocking (a FIFO opens at once), then checked on the open descriptor, so that
/// a link swapped in between changes nothing.
pub fn read_regular(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    if path.as_os_str().len() > MAX_PATH_BYTES {
        return Err(refused("path too long"));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(refused("not a regular file"));
    }
    if metadata.len() > limit {
        return Err(refused("file too large"));
    }
    let mut bytes = Vec::new();
    // The file may grow after `metadata`: one byte past the limit is enough to know.
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(refused("file too large"));
    }
    Ok(bytes)
}

/// A PNG: its signature, then its header with both sides within `MAX_PNG_SIDE`, so that its
/// decoding stays small whatever its compression.
fn check_png(bytes: &[u8]) -> io::Result<()> {
    let side = |at: usize| {
        let field: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
        Some(u32::from_be_bytes(field))
    };
    let header = bytes.starts_with(PNG_SIGNATURE) && bytes.get(12..16) == Some(b"IHDR");
    match (header, side(16), side(20)) {
        (true, Some(w), Some(h))
            if (1..=MAX_PNG_SIDE).contains(&w) && (1..=MAX_PNG_SIDE).contains(&h) =>
        {
            Ok(())
        }
        _ => Err(refused("not a PNG within the limits")),
    }
}

/// An SVG that can load nothing: every `href` points inside the document (`#…`), and there is
/// no document type, so no entity either.
/// It also holds `MAX_SVG_ELEMENTS` elements and `MAX_SVG_USES` `<use>` at most, so that its
/// drawing stays short.
pub fn check_svg(bytes: &[u8]) -> io::Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| refused("SVG not in UTF-8"))?;
    if !text.contains("<svg") || text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(refused("not a plain SVG"));
    }
    // An element starts with `<` and a name: not a closing tag, a comment or a declaration.
    let elements = text
        .match_indices('<')
        .filter(|(at, _)| {
            text[at + 1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
        })
        .count();
    let uses = text.matches("<use").count();
    if elements > MAX_SVG_ELEMENTS || uses > MAX_SVG_USES {
        return Err(refused("SVG too costly to draw"));
    }
    for (at, _) in text.match_indices("href") {
        let value = text[at + 4..]
            .trim_start()
            .strip_prefix('=')
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix(['"', '\'']));
        if !value.is_some_and(|v| v.starts_with('#')) {
            return Err(refused("SVG with an outside reference"));
        }
    }
    Ok(())
}

/// An SVG document that shows the PNG `bytes`, embedded as a `data:` URI: resvg decodes PNG,
/// and nothing is read from the disk.
pub fn png_in_svg(bytes: &[u8]) -> Vec<u8> {
    let mut document = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" \
         viewBox=\"0 0 1 1\"><image width=\"1\" height=\"1\" xlink:href=\"data:image/png;base64,",
    );
    base64(bytes, &mut document);
    document.push_str("\"/></svg>");
    document.into_bytes()
}

fn base64(bytes: &[u8], out: &mut String) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "notifications-picture-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend([0, 0, 0, 13]);
        bytes.extend(b"IHDR");
        bytes.extend(width.to_be_bytes());
        bytes.extend(height.to_be_bytes());
        bytes.extend([8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn devices_fifos_and_links_to_them_are_refused_at_once() {
        let dir = dir("special");
        assert!(read(Path::new("/dev/zero")).is_err());
        let fifo = dir.join("fifo.png");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        // Opened without blocking: no writer is needed for this to return.
        assert!(read(&fifo).is_err());
        let link = dir.join("zero.svg");
        std::os::unix::fs::symlink("/dev/zero", &link).unwrap();
        assert!(read(&link).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sizes_and_formats_are_checked() {
        let dir = dir("sizes");
        let small = dir.join("small.png");
        std::fs::write(&small, png(48, 48)).unwrap();
        assert_eq!(read(&small).unwrap(), (Kind::Png, png(48, 48)));
        let wide = dir.join("wide.png");
        std::fs::write(&wide, png(40_000, 10)).unwrap();
        assert!(read(&wide).is_err());
        let big = dir.join("big.png");
        let mut bytes = png(48, 48);
        bytes.resize(MAX_PNG_BYTES as usize + 1, 0);
        std::fs::write(&big, bytes).unwrap();
        assert!(read(&big).is_err());
        let text = dir.join("text.png");
        std::fs::write(&text, "not a picture").unwrap();
        assert!(read(&text).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_svg_may_only_point_inside_itself() {
        let ok = br##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#a"/><use xlink:href = '#b'/></svg>"##;
        assert!(check_svg(ok).is_ok());
        for bad in [
            &br#"<svg><image href="/dev/zero"/></svg>"#[..],
            br#"<svg><image xlink:href="file:///dev/zero"/></svg>"#,
            br#"<svg><feImage href="data:image/png;base64,AAAA"/></svg>"#,
            br#"<svg><image href=/dev/zero /></svg>"#,
            br#"<!DOCTYPE svg [<!ENTITY a SYSTEM "/dev/zero">]><svg>&a;</svg>"#,
            b"<svg>\xff</svg>",
        ] {
            assert!(check_svg(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        let dir = dir("svg");
        let file = dir.join("zero.svg");
        std::fs::write(&file, r#"<svg><image href="/dev/zero"/></svg>"#).unwrap();
        assert!(read(&file).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn base64_matches_the_standard() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
        ] {
            let mut out = String::new();
            base64(input.as_bytes(), &mut out);
            assert_eq!(out, expected);
        }
        let document = String::from_utf8(png_in_svg(b"foo")).unwrap();
        assert!(document.contains("href=\"data:image/png;base64,Zm9v\""));
    }
}
