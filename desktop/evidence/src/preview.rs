//! Preview of the selected item: the start of a text, an image, or plain information.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use iced::widget::image::Handle;
use sherlock_ui::component::CodeRole;
use sherlock_ui::metrics;

use crate::listing::{Entry, Kind};

/// Maximum bytes read for a text preview: enough to fill the panel.
const READ_LIMIT: u64 = 16 * 1024;
/// Width of a tab in the preview.
const TAB: &str = "    ";
/// Longest a preview may take: a slow disk or a stuck mount gives up rather than holding a
/// thread of the pool.
const TIMEOUT: Duration = Duration::from_secs(2);

/// What the preview panel shows.
#[derive(Debug, Clone)]
pub enum Preview {
    /// First lines of a text file, split into coloured pieces.
    Text(Vec<Vec<(String, CodeRole)>>),
    /// An image, already decoded and scaled to the panel off the interface thread.
    Image(Handle),
    /// Nothing to show beyond the file information.
    Info,
}

/// Runs `work` on its own thread and waits at most `TIMEOUT` for it. A read that never
/// returns (a dead network mount) is left behind instead of blocking the caller.
fn with_timeout<T: Send + 'static>(work: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver.recv_timeout(TIMEOUT).ok().flatten()
}

/// Opens a file for the preview only if it is a regular file. `O_NONBLOCK`: even if it was
/// swapped for a named pipe since it was listed, opening it does not wait for a writer.
fn open_regular(path: &Path) -> Option<File> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = File::options()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    file.metadata().ok()?.is_file().then_some(file)
}

fn read_head(path: PathBuf) -> Option<Vec<u8>> {
    with_timeout(move || {
        let mut bytes = Vec::new();
        open_regular(&path)?
            .take(READ_LIMIT)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(bytes)
    })
}

/// Largest picture the preview decodes, in pixels, and on each side: a 420 KB PNG can hold
/// 12000 × 12000 pixels, half a gigabyte once decoded (audit I-2). Beyond, only its
/// information shows.
pub const MAX_IMAGE_PIXELS: u64 = 32_000_000;
pub const MAX_IMAGE_SIDE: u32 = 16_384;
/// Memory a decoder may hold at once.
const MAX_IMAGE_ALLOC: u64 = 192 * 1024 * 1024;

/// The number of the last image asked for: a decode that is no longer the last one stops at
/// its next read, so that images run through with the arrows never pile up in memory.
static LATEST_IMAGE: AtomicU64 = AtomicU64::new(0);

/// A file read that fails once its decode is superseded or given up.
struct Cancellable<R> {
    inner: R,
    ticket: u64,
    given_up: Arc<AtomicBool>,
}

impl<R: Read> Read for Cancellable<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.given_up.load(Ordering::Relaxed)
            || LATEST_IMAGE.load(Ordering::Relaxed) != self.ticket
        {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for Cancellable<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// Decodes an image and scales it down to the panel, keeping its proportions. Its size is
/// read from its header first; a picture too large is never decoded.
fn decode_image(path: PathBuf) -> Option<Handle> {
    let ticket = LATEST_IMAGE.fetch_add(1, Ordering::Relaxed) + 1;
    let given_up = Arc::new(AtomicBool::new(false));
    let flag = given_up.clone();
    let handle = with_timeout(move || {
        let reader = |file| {
            image::ImageReader::new(std::io::BufReader::new(Cancellable {
                inner: file,
                ticket,
                given_up: flag.clone(),
            }))
            .with_guessed_format()
            .ok()
        };
        let (width, height) = reader(open_regular(&path)?)?.into_dimensions().ok()?;
        if !image_fits(width, height) {
            return None;
        }
        let mut reader = reader(open_regular(&path)?)?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_SIDE);
        limits.max_image_height = Some(MAX_IMAGE_SIDE);
        limits.max_alloc = Some(MAX_IMAGE_ALLOC);
        reader.limits(limits);
        let side = metrics::PREVIEW_IMAGE_PIXELS;
        let picture = reader.decode().ok()?.thumbnail(side, side).to_rgba8();
        let (width, height) = picture.dimensions();
        Some(Handle::from_rgba(width, height, picture.into_raw()))
    });
    // Given up after the time limit: the decode stops at its next read.
    given_up.store(true, Ordering::Relaxed);
    handle
}

/// Whether a picture of this size may be decoded.
fn image_fits(width: u32, height: u32) -> bool {
    width <= MAX_IMAGE_SIDE
        && height <= MAX_IMAGE_SIDE
        && u64::from(width) * u64::from(height) <= MAX_IMAGE_PIXELS
}

/// Prepares the preview of an item, off the interface thread. Only regular files are ever
/// opened: a named pipe, a socket or a device only gets its information. An unreadable,
/// binary or empty file too.
pub fn load(entry: &Entry) -> Preview {
    if entry.kind != Kind::File {
        return Preview::Info;
    }
    if entry.is_image() {
        return decode_image(entry.path.clone()).map_or(Preview::Info, Preview::Image);
    }
    let Some(bytes) = read_head(entry.path.clone()) else {
        return Preview::Info;
    };
    if bytes.is_empty() || bytes.contains(&0) {
        return Preview::Info;
    }
    // The read may have split a character in two: only the end can be invalid.
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) if bytes.len() - error.valid_up_to() < 4 => {
            std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap_or_default()
        }
        Err(_) => return Preview::Info,
    };
    let language = Language::of(entry);
    let lines = text
        .lines()
        .take(metrics::PREVIEW_LINES)
        .map(|line| {
            let line: String = line
                .replace('\t', TAB)
                .chars()
                .take(metrics::PREVIEW_COLUMNS)
                .collect();
            language.highlight(&line)
        })
        .collect();
    Preview::Text(lines)
}

/// What a file really is, read from its first bytes, when that can disguise itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Real {
    /// An ELF or Windows binary, or a script with `#!`.
    Program,
    Png,
    Jpeg,
    Gif,
    Pdf,
    Zip,
}

impl Real {
    /// From the first bytes of a file.
    pub fn of(head: &[u8]) -> Option<Self> {
        let starts = |magic: &[u8]| head.starts_with(magic);
        Some(if starts(b"\x7fELF") || starts(b"MZ") || starts(b"#!") {
            Self::Program
        } else if starts(b"\x89PNG\r\n\x1a\n") {
            Self::Png
        } else if starts(b"\xff\xd8\xff") {
            Self::Jpeg
        } else if starts(b"GIF87a") || starts(b"GIF89a") {
            Self::Gif
        } else if starts(b"%PDF-") {
            Self::Pdf
        } else if starts(b"PK\x03\x04") {
            Self::Zip
        } else {
            return None;
        })
    }

    /// What a file named with this extension should be, for the extensions that say it.
    fn expected(extension: &str) -> Option<Self> {
        Some(match extension {
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpeg,
            "gif" => Self::Gif,
            "pdf" => Self::Pdf,
            "zip" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "jar" | "apk" => {
                Self::Zip
            }
            _ => return None,
        })
    }

    /// Whether a file named with `extension` but holding this hides what it is: a program
    /// with the extension of a document, or an image of another kind than its name says.
    pub fn disguised_by(self, extension: Option<&str>) -> bool {
        let Some(extension) = extension else {
            return false;
        };
        match self {
            Self::Program => !matches!(
                extension,
                "sh" | "bash"
                    | "zsh"
                    | "fish"
                    | "py"
                    | "pl"
                    | "rb"
                    | "lua"
                    | "js"
                    | "exe"
                    | "bin"
                    | "run"
                    | "appimage"
                    | "so"
                    | "out"
                    | "elf"
                    | "com"
                    | "dll"
                    | "msi"
            ),
            real => Self::expected(extension).is_some_and(|expected| expected != real),
        }
    }
}

/// Whether opening `path` without a chosen application could run it: an executable file, a
/// launcher (`.desktop`), or a binary or a script by its first bytes (audit I-14). A file that
/// cannot be read in time counts as one.
pub fn is_program(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let launcher = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("desktop"));
    let path = path.to_path_buf();
    let read = with_timeout(move || {
        let file = open_regular(&path)?;
        let executable = file.metadata().ok()?.permissions().mode() & 0o111 != 0;
        let mut head = Vec::new();
        file.take(16).read_to_end(&mut head).ok()?;
        Some(executable || Real::of(&head) == Some(Real::Program))
    });
    launcher || read.unwrap_or(true)
}

/// The real kind of a file whose name hides it (audit I-3), read off the interface thread.
pub fn disguise(entry: &Entry) -> Option<Real> {
    if entry.kind != Kind::File {
        return None;
    }
    let path = entry.path.clone();
    let head = with_timeout(move || {
        let mut head = Vec::new();
        open_regular(&path)?.take(16).read_to_end(&mut head).ok()?;
        Some(head)
    })?;
    Real::of(&head).filter(|real| real.disguised_by(entry.extension().as_deref()))
}

/// Light highlighting of the preview: keywords and comments of a few languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Docker,
    Rust,
    Toml,
    Shell,
    Python,
    Plain,
}

const DOCKER: &[&str] = &[
    "FROM",
    "AS",
    "RUN",
    "COPY",
    "ADD",
    "WORKDIR",
    "ENV",
    "ARG",
    "CMD",
    "ENTRYPOINT",
    "EXPOSE",
    "USER",
    "LABEL",
    "VOLUME",
    "HEALTHCHECK",
];
const RUST: &[&str] = &[
    "as", "async", "await", "const", "crate", "else", "enum", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "return", "self", "Self", "static",
    "struct", "trait", "type", "use", "where", "while",
];
const SHELL: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "case", "esac", "while",
    "function", "export", "local", "return",
];
const PYTHON: &[&str] = &[
    "def", "class", "import", "from", "as", "if", "elif", "else", "for", "in", "while", "return",
    "with", "try", "except", "None", "True", "False", "lambda", "yield",
];

impl Language {
    fn of(entry: &Entry) -> Self {
        if entry.name == "Dockerfile" || entry.name.starts_with("Dockerfile.") {
            return Self::Docker;
        }
        match entry.extension().as_deref() {
            Some("rs") => Self::Rust,
            Some("toml") => Self::Toml,
            Some("sh" | "bash" | "zsh") => Self::Shell,
            Some("py") => Self::Python,
            _ if entry.name == "justfile" || entry.name == "Makefile" => Self::Shell,
            _ => Self::Plain,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Docker => DOCKER,
            Self::Rust => RUST,
            Self::Shell => SHELL,
            Self::Python => PYTHON,
            Self::Toml | Self::Plain => &[],
        }
    }

    fn comment(self) -> Option<&'static str> {
        match self {
            Self::Rust => Some("//"),
            Self::Plain => None,
            _ => Some("#"),
        }
    }

    /// Splits a line into coloured pieces.
    fn highlight(self, line: &str) -> Vec<(String, CodeRole)> {
        let trimmed = line.trim_start();
        if self.comment().is_some_and(|c| trimmed.starts_with(c)) {
            return vec![(line.to_owned(), CodeRole::Comment)];
        }
        if self == Self::Toml && trimmed.starts_with('[') {
            return vec![(line.to_owned(), CodeRole::Keyword)];
        }
        let keywords = self.keywords();
        let mut pieces: Vec<(String, CodeRole)> = Vec::new();
        let mut push = |piece: &str, role: CodeRole| match pieces.last_mut() {
            Some((last, r)) if *r == role => last.push_str(piece),
            _ => pieces.push((piece.to_owned(), role)),
        };
        let mut word_start = None;
        for (i, c) in line
            .char_indices()
            .chain(std::iter::once((line.len(), ' ')))
        {
            let in_word = c.is_alphanumeric() || c == '_';
            match (in_word, word_start) {
                (true, None) => word_start = Some(i),
                (false, Some(start)) => {
                    let word = &line[start..i];
                    let role = if keywords.contains(&word) {
                        CodeRole::Keyword
                    } else {
                        CodeRole::Plain
                    };
                    push(word, role);
                    word_start = None;
                    if i < line.len() {
                        push(&line[i..i + c.len_utf8()], CodeRole::Plain);
                    }
                }
                (false, None) if i < line.len() => {
                    push(&line[i..i + c.len_utf8()], CodeRole::Plain)
                }
                _ => {}
            }
        }
        pieces
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn entry(path: PathBuf) -> Entry {
        Entry::read(&path, false).expect("item")
    }

    #[test]
    fn a_text_shows_its_first_lines() {
        let tmp = TempDir::new("preview");
        let path = tmp.path().join("Dockerfile");
        let content: String = (0..100).map(|i| format!("RUN\tétape {i}\n")).collect();
        std::fs::write(&path, content).expect("write");
        let Preview::Text(lines) = load(&entry(path)) else {
            panic!("text preview expected");
        };
        assert_eq!(lines.len(), metrics::PREVIEW_LINES);
        assert_eq!(
            lines[0],
            [
                ("RUN".to_owned(), CodeRole::Keyword),
                ("    étape 0".to_owned(), CodeRole::Plain)
            ]
        );
    }

    #[test]
    fn a_disguised_file_is_found_by_its_first_bytes() {
        let tmp = TempDir::new("disguise");
        let write = |name: &str, bytes: &[u8]| {
            let path = tmp.path().join(name);
            std::fs::write(&path, bytes).expect("write");
            entry(path)
        };
        assert_eq!(
            disguise(&write("facture.pdf", b"\x7fELF\x02\x01")),
            Some(Real::Program)
        );
        assert_eq!(
            disguise(&write("photo.jpg", b"\x89PNG\r\n\x1a\n....")),
            Some(Real::Png)
        );
        assert_eq!(
            disguise(&write("notes.txt", b"#!/bin/sh\nrm -rf ~")),
            Some(Real::Program)
        );
        assert_eq!(disguise(&write("vrai.pdf", b"%PDF-1.7")), None);
        assert_eq!(disguise(&write("lettre.docx", b"PK\x03\x04")), None);
        assert_eq!(disguise(&write("outil.sh", b"#!/bin/sh")), None);
        assert_eq!(disguise(&write("programme", b"\x7fELF")), None);
        // Programs and launchers are never handed to xdg-open.
        assert!(is_program(&tmp.path().join("programme")));
        assert!(is_program(
            &write("raccourci.desktop", b"[Desktop Entry]").path
        ));
        assert!(!is_program(&write("lettre.txt", b"Bonjour").path));
        assert!(is_program(&tmp.path().join("absent.txt")));
    }

    /// The start of a PNG: its signature and a header for this size, with its checksum.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut chunk = b"IHDR".to_vec();
        chunk.extend(width.to_be_bytes());
        chunk.extend(height.to_be_bytes());
        chunk.extend([8, 2, 0, 0, 0]);
        let mut crc = !0u32;
        for byte in &chunk {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(13u32.to_be_bytes());
        png.extend(&chunk);
        png.extend((!crc).to_be_bytes());
        png
    }

    #[test]
    fn a_picture_too_large_is_never_decoded() {
        assert!(image_fits(6000, 4000));
        assert!(!image_fits(12_000, 12_000));
        assert!(!image_fits(MAX_IMAGE_SIDE + 1, 10));
        // A PNG whose header says 12000 × 12000 gives only its information.
        let tmp = TempDir::new("bomb");
        let path = tmp.path().join("bombe.png");
        std::fs::write(&path, png_header(12_000, 12_000)).expect("write");
        assert!(decode_image(path).is_none());
    }

    #[test]
    fn highlighting_finds_keywords_and_comments() {
        let rust = Language::Rust.highlight("pub fn main() // fin");
        assert_eq!(rust[0], ("pub".to_owned(), CodeRole::Keyword));
        assert_eq!(rust[2], ("fn".to_owned(), CodeRole::Keyword));
        let comment = Language::Shell.highlight("  # commentaire");
        assert_eq!(comment, [("  # commentaire".to_owned(), CodeRole::Comment)]);
        let text: String = Language::Docker
            .highlight("COPY . .")
            .into_iter()
            .map(|(piece, _)| piece)
            .collect();
        assert_eq!(text, "COPY . .");
    }

    #[test]
    fn a_binary_an_image_and_a_folder_get_their_preview() {
        let tmp = TempDir::new("preview-others");
        let binary = tmp.path().join("programme");
        std::fs::write(&binary, [0x7f, b'E', b'L', b'F', 0, 1]).expect("write");
        assert!(matches!(load(&entry(binary)), Preview::Info));
        let image = tmp.path().join("photo.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([1, 2, 3, 255]))
            .save(&image)
            .expect("png");
        assert!(matches!(load(&entry(image)), Preview::Image(_)));
        let broken = tmp.path().join("broken.png");
        std::fs::write(&broken, b"not a png").expect("write");
        assert!(matches!(load(&entry(broken)), Preview::Info));
        assert!(matches!(
            load(&entry(tmp.path().to_path_buf())),
            Preview::Info
        ));
        // A character cut by the read limit does not spoil the preview.
        let cut = tmp.path().join("long.txt");
        let mut text = "a".repeat(READ_LIMIT as usize - 1).into_bytes();
        text.extend("é".as_bytes());
        std::fs::write(&cut, text).expect("write");
        assert!(matches!(load(&entry(cut)), Preview::Text(_)));
    }

    #[test]
    fn a_named_pipe_is_never_opened() {
        let tmp = TempDir::new("preview-fifo");
        let pipe = tmp.path().join("tube");
        let c = std::ffi::CString::new(pipe.as_os_str().as_encoded_bytes()).expect("path");
        // SAFETY: `c` is a NUL-terminated path alive for the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        let listed = entry(pipe.clone());
        assert_eq!(listed.kind, Kind::Special);
        // Even forced through as a regular file, opening it does not wait for a writer.
        let forced = Entry {
            kind: Kind::File,
            ..listed
        };
        let started = std::time::Instant::now();
        assert!(matches!(load(&forced), Preview::Info));
        assert!(started.elapsed() < TIMEOUT);
    }
}
