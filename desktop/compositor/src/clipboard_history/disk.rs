//! The history kept on disk, when `keep-on-disk = on` (lot D-42): one file readable by the
//! user alone, `$XDG_STATE_HOME/sherlockos/clipboard-history`, written whole after each change
//! by a thread, read back at start with the same limits as a copy. Turned off, the file goes.
//!
//! The format is ours, little-endian: `SHERLOCKOS-CLIPBOARD 1\n`, then per item, oldest
//! last: kind (u8), pinned (u8), time (u64), app, preview, thumbnail, the count of types
//! (u8), and each type with its bytes. A string or a buffer is its length (u32) then its bytes.

use std::{
    fs,
    io::{self, Write as _},
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::Path,
    sync::Arc,
};

use super::model::{Content, Entry, Held, Kind, MAX_BYTES, MAX_ENTRIES};

const MAGIC: &[u8] = b"SHERLOCKOS-CLIPBOARD 1\n";

/// Types of one item at most.
const MAX_TYPES: u8 = 8;

/// The items as the file holds them.
pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let put = |bytes: &[u8], out: &mut Vec<u8>| {
        out.extend_from_slice(&u32::try_from(bytes.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(bytes);
    };
    for entry in entries.iter().take(MAX_ENTRIES) {
        let content = &entry.content;
        out.push(content.kind as u8);
        out.push(u8::from(entry.pinned));
        out.extend_from_slice(&entry.time.to_le_bytes());
        put(entry.app.as_bytes(), &mut out);
        put(content.preview.as_bytes(), &mut out);
        put(content.thumbnail.as_deref().unwrap_or_default(), &mut out);
        let types = content.held.entries();
        let count = types.len().min(usize::from(MAX_TYPES));
        out.push(count as u8);
        for (mime, bytes) in types.iter().take(count) {
            put(mime.as_bytes(), &mut out);
            put(bytes, &mut out);
        }
    }
    out
}

/// An item read back: its content, application, time and pin.
pub type Restored = (Content, String, u64, bool);

/// Reads what `encode` wrote. Stops at the first item that does not read, keeping those
/// before; never more than the history's limits.
pub fn decode(bytes: &[u8]) -> Vec<Restored> {
    let Some(mut rest) = bytes.strip_prefix(MAGIC) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    let mut total = 0usize;
    while !rest.is_empty() && items.len() < MAX_ENTRIES {
        match item(&mut rest) {
            Some(restored) => {
                total += restored
                    .0
                    .held
                    .entries()
                    .iter()
                    .map(|(_, b)| b.len())
                    .sum::<usize>();
                if total > MAX_BYTES {
                    break;
                }
                items.push(restored);
            }
            None => break,
        }
    }
    items
}

fn take<'a>(rest: &mut &'a [u8], len: usize) -> Option<&'a [u8]> {
    let (head, tail) = rest.split_at_checked(len)?;
    *rest = tail;
    Some(head)
}

fn buffer<'a>(rest: &mut &'a [u8]) -> Option<&'a [u8]> {
    let len = u32::from_le_bytes(take(rest, 4)?.try_into().ok()?);
    take(rest, usize::try_from(len).ok()?)
}

fn string(rest: &mut &[u8]) -> Option<String> {
    String::from_utf8(buffer(rest)?.to_vec()).ok()
}

fn item(rest: &mut &[u8]) -> Option<Restored> {
    let kind = match take(rest, 1)?[0] {
        0 => Kind::Text,
        1 => Kind::Link,
        2 => Kind::Files,
        3 => Kind::Image,
        _ => return None,
    };
    let pinned = take(rest, 1)?[0] != 0;
    let time = u64::from_le_bytes(take(rest, 8)?.try_into().ok()?);
    let app = string(rest)?;
    let preview = string(rest)?;
    let thumbnail = buffer(rest)?;
    let count = take(rest, 1)?[0];
    if count == 0 || count > MAX_TYPES {
        return None;
    }
    let mut held = Held::default();
    let mut key: Option<Arc<[u8]>> = None;
    for _ in 0..count {
        let mime = string(rest)?;
        let bytes: Arc<[u8]> = Arc::from(buffer(rest)?);
        key.get_or_insert_with(|| bytes.clone());
        held = held.with(&mime, bytes);
    }
    let key = key?;
    let mut content = Content::new(kind, held, &key, preview);
    if !thumbnail.is_empty() {
        content = content.with_thumbnail(Arc::from(thumbnail));
    }
    Some((content, app, time, pinned))
}

/// Writes `bytes` to `path` for the user alone, through a file beside it renamed over it.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let partial = path.with_extension("partial");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(partial, path)
}

/// The file's items, if it is there and not larger than the history may be.
pub fn read(path: &Path) -> Vec<Restored> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size == 0 || size > (MAX_BYTES as u64) * 2 {
        return Vec::new();
    }
    fs::read(path)
        .map(|bytes| decode(&bytes))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard_history::History;

    #[test]
    fn what_is_written_reads_back() {
        let mut history = History::default();
        let text: Arc<[u8]> = Arc::from(&b"hello"[..]);
        let held = Held::new(&["text/plain;charset=utf-8", "text/plain"], text.clone());
        let a = Content::new(Kind::Text, held, &text, "hello".into());
        let png: Arc<[u8]> = Arc::from(&b"png"[..]);
        let b = Content::new(Kind::Image, Held::png(png.clone()), &png, "1 × 1".into())
            .with_thumbnail(Arc::from(&b"small"[..]));
        let id = history.push(a.clone(), "kate".into(), 7).unwrap();
        history.set_pinned(id, true);
        history.push(b.clone(), String::new(), 9);
        let read = decode(&encode(history.entries()));
        assert_eq!(read.len(), 2);
        assert_eq!(read[0], (b, String::new(), 9, false));
        assert_eq!(read[1], (a, "kate".into(), 7, true));
    }

    #[test]
    fn a_broken_file_gives_what_reads() {
        assert!(decode(b"something else").is_empty());
        let mut history = History::default();
        let text: Arc<[u8]> = Arc::from(&b"x"[..]);
        let held = Held::new(&["text/plain"], text.clone());
        history.push(
            Content::new(Kind::Text, held, &text, "x".into()),
            String::new(),
            1,
        );
        let mut bytes = encode(history.entries());
        bytes.extend_from_slice(&[9, 9, 9]);
        assert_eq!(decode(&bytes).len(), 1);
        bytes.truncate(bytes.len() - 6);
        assert!(decode(&bytes).is_empty());
    }

    #[test]
    fn the_file_is_the_users_alone() {
        let dir = std::env::temp_dir().join(format!("clipboard-disk-{}", std::process::id()));
        let path = dir.join("history");
        write(&path, MAGIC).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        assert!(read(&path).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }
}
