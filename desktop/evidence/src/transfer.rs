//! Files handed between applications, by drag and drop or by the clipboard (lot E-03): the
//! lists of `text/uri-list` (RFC 2483) and `x-special/gnome-copied-files` (GNOME's and KDE's
//! copy and cut), read and written here. No iced, no Wayland: tested on its own.
//!
//! What comes from another application is bounded: `MAX_FILES` files, `MAX_BYTES` bytes, local
//! `file://` addresses only (another host, `http://`, `smb://` are dropped), absolute paths
//! without `..`.

use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt as _, OsStringExt as _},
    path::{Component, Path, PathBuf},
};

/// The list of files, one address per line.
pub const URI_LIST: &str = "text/uri-list";
/// GNOME's list: `copy` or `cut`, then one address per line. KDE and many others read it.
pub const GNOME_FILES: &str = "x-special/gnome-copied-files";
/// The paths as text, for the applications that take only text (a terminal).
pub const TEXT: &str = "text/plain;charset=utf-8";
/// The types offered, from the most precise.
pub const OFFERED: [&str; 3] = [GNOME_FILES, URI_LIST, TEXT];

/// Most files taken from one list.
pub const MAX_FILES: usize = 10_000;
/// Most bytes read for one list.
pub const MAX_BYTES: usize = 4 << 20;

/// Copy or cut, as the list says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Cut,
}

/// Bytes kept as is in an address: RFC 3986's unreserved characters, and the slash.
fn kept(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte)
}

/// The `file://` address of an absolute path, its bytes percent-encoded.
pub fn uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if kept(byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `text/uri-list`: one address per line, ended by CR LF (RFC 2483).
pub fn uri_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut out = String::new();
    for path in paths {
        out.push_str(&uri(path));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// `x-special/gnome-copied-files`: the mode, then one address per line.
pub fn gnome_files(mode: Mode, paths: &[PathBuf]) -> Vec<u8> {
    let mut out = String::from(match mode {
        Mode::Copy => "copy",
        Mode::Cut => "cut",
    });
    for path in paths {
        out.push('\n');
        out.push_str(&uri(path));
    }
    out.into_bytes()
}

/// The paths, one per line, for `TEXT`.
pub fn text(paths: &[PathBuf]) -> Vec<u8> {
    let lines: Vec<&[u8]> = paths.iter().map(|p| p.as_os_str().as_bytes()).collect();
    lines.join(&b'\n')
}

/// What `mime_type` gives, for `paths` cut or copied.
pub fn encode(mime_type: &str, mode: Mode, paths: &[PathBuf]) -> Option<Vec<u8>> {
    match mime_type {
        URI_LIST => Some(uri_list(paths)),
        GNOME_FILES => Some(gnome_files(mode, paths)),
        TEXT | "text/plain" | "UTF8_STRING" => Some(text(paths)),
        _ => None,
    }
}

/// The local path of a `file://` address, `None` for any other.
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file:///a` or `file://localhost/a`; another host is not this machine.
    let rest = match rest.strip_prefix("localhost") {
        Some(rest) => rest,
        None => rest,
    };
    if !rest.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut chars = rest.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let hex = [chars.next()?, chars.next()?];
            let hex = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    if bytes.contains(&0) {
        return None;
    }
    let path = PathBuf::from(OsString::from_vec(bytes));
    let plain = path
        .components()
        .all(|c| matches!(c, Component::RootDir | Component::Normal(_)));
    plain.then_some(path)
}

/// The files of a list received as `mime_type`, and whether they were cut. `None` if the type
/// is not a list of files, or the list holds no local file.
pub fn decode(mime_type: &str, data: &[u8]) -> Option<(Mode, Vec<PathBuf>)> {
    if data.len() > MAX_BYTES {
        return None;
    }
    let text = String::from_utf8_lossy(data);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let mode = match mime_type {
        URI_LIST => Mode::Copy,
        GNOME_FILES => match lines.next()? {
            "cut" => Mode::Cut,
            "copy" => Mode::Copy,
            _ => return None,
        },
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines
        .filter(|l| !l.starts_with('#'))
        .filter_map(path_of)
        .take(MAX_FILES)
        .collect();
    (!paths.is_empty()).then_some((mode, paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_goes_and_comes_back_with_its_odd_characters() {
        let path = PathBuf::from("/home/me/Ünïcode 2026/photo #1 (100%).png");
        let address = uri(&path);
        assert_eq!(
            address,
            "file:///home/me/%C3%9Cn%C3%AFcode%202026/photo%20%231%20%28100%25%29.png"
        );
        assert_eq!(path_of(&address), Some(path));
    }

    #[test]
    fn the_lists_follow_their_formats() {
        let paths = [PathBuf::from("/a/b c"), PathBuf::from("/d")];
        assert_eq!(uri_list(&paths), b"file:///a/b%20c\r\nfile:///d\r\n");
        assert_eq!(
            gnome_files(Mode::Cut, &paths),
            b"cut\nfile:///a/b%20c\nfile:///d"
        );
        assert_eq!(text(&paths), b"/a/b c\n/d");
    }

    #[test]
    fn a_list_received_keeps_only_local_plain_paths() {
        let list = b"# comment\r\nfile:///ok%20one\r\nhttp://example.org/x\r\nfile://other/x\r\n\
                     file:///a/../etc\r\nfile:///bad%2\r\nfile:///nul%00\r\nfile://localhost/two\r\n";
        let (mode, paths) = decode(URI_LIST, list).unwrap();
        assert_eq!(mode, Mode::Copy);
        assert_eq!(paths, [PathBuf::from("/ok one"), PathBuf::from("/two")]);

        let (mode, paths) = decode(GNOME_FILES, b"cut\nfile:///x\n").unwrap();
        assert_eq!((mode, paths), (Mode::Cut, vec![PathBuf::from("/x")]));
        assert_eq!(decode(GNOME_FILES, b"steal\nfile:///x"), None);
        assert_eq!(decode(URI_LIST, b"http://example.org"), None);
        assert_eq!(decode(TEXT, b"file:///x"), None);
    }

    #[test]
    fn a_list_received_is_bounded() {
        let huge = vec![b'a'; MAX_BYTES + 1];
        assert_eq!(decode(URI_LIST, &huge), None);
        let many = "file:///f\n".repeat(MAX_FILES + 5);
        let (_, paths) = decode(URI_LIST, many.as_bytes()).unwrap();
        assert_eq!(paths.len(), MAX_FILES);
    }
}
