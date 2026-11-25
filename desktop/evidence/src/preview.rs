//! Preview of the selected item: the start of a text, an image, or plain information.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
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

/// Decodes an image and scales it down to the panel, keeping its proportions.
fn decode_image(path: PathBuf) -> Option<Handle> {
    with_timeout(move || {
        let file = open_regular(&path)?;
        let reader = image::ImageReader::new(std::io::BufReader::new(file))
            .with_guessed_format()
            .ok()?;
        let side = metrics::PREVIEW_IMAGE_PIXELS;
        let picture = reader.decode().ok()?.thumbnail(side, side).to_rgba8();
        let (width, height) = picture.dimensions();
        Some(Handle::from_rgba(width, height, picture.into_raw()))
    })
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
