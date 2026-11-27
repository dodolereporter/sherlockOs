//! The path bar in text mode (lot E-04): what a typed path means, the folders it may complete
//! to, and names shown without the control characters they may hold. No iced here.

use std::borrow::Cow;
use std::os::unix::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use sherlock_ui::evidence as more;

/// The folder a completion lists, and the names of its subfolders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    pub dir: PathBuf,
    /// Sorted, hidden ones included; never a name a text field cannot hold (not UTF-8, or with
    /// a control character).
    pub names: Vec<String>,
}

/// Longest text the field reads as a path, in bytes (`PATH_MAX`).
pub const MAX_PATH: usize = 4096;

/// `input` as a path: `~` is the home folder, a relative path starts from `cwd`, `.` and `..`
/// are folded without following links, a pasted `file://` link is decoded and the blanks a
/// paste leaves around it are dropped. None for an empty text, one longer than `MAX_PATH`, or
/// one with a control character.
pub fn resolve(input: &str, cwd: &Path, home: &Path) -> Option<PathBuf> {
    let input = input.trim();
    if input.is_empty() || input.len() > MAX_PATH || input.chars().any(char::is_control) {
        return None;
    }
    let raw = if let Some(link) = input.strip_prefix("file://") {
        // Only a local link: `file:///x` or `file://localhost/x`.
        let local = link.strip_prefix("localhost").unwrap_or(link);
        Path::new("/").join(std::ffi::OsString::from_vec(unescape(
            local.strip_prefix('/')?,
        )?))
    } else if input == "~" {
        home.to_path_buf()
    } else if let Some(rest) = input.strip_prefix("~/") {
        home.join(rest)
    } else {
        cwd.join(input)
    };
    let mut out = PathBuf::from("/");
    for part in raw.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => out.push(name),
            Component::RootDir | Component::CurDir | Component::Prefix(_) => {}
        }
    }
    Some(out)
}

/// The bytes of a link's path, its `%xx` decoded; None for a bad escape or a NUL.
fn unescape(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok().filter(|b| *b != 0)?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// The typed text cut before the name being typed: (`~/Doc`) → (`~/`, `Doc`). `~` alone reads
/// as `~/`.
pub fn split(input: &str) -> (Cow<'_, str>, &str) {
    if input == "~" {
        return (Cow::Borrowed("~/"), "");
    }
    match input.rfind('/') {
        Some(slash) => (Cow::Borrowed(&input[..=slash]), &input[slash + 1..]),
        None => (Cow::Borrowed(""), input),
    }
}

/// Subfolders of `folders` whose name starts with `prefix`, ignoring case, at most
/// `COMPLETION_MAX_SHOWN`. Hidden ones once a dot is typed, as in a shell, or when the
/// listing shows them.
pub fn matches(folders: &Folders, prefix: &str, hidden: bool) -> Vec<String> {
    let lower = prefix.to_lowercase();
    folders
        .names
        .iter()
        .filter(|name| hidden || prefix.starts_with('.') || !name.starts_with('.'))
        .filter(|name| name.to_lowercase().starts_with(&lower) && name.as_str() != prefix)
        .take(more::COMPLETION_MAX_SHOWN)
        .cloned()
        .collect()
}

/// The longest start all `names` share, character by character.
pub fn common_prefix(names: &[String]) -> String {
    let Some((first, rest)) = names.split_first() else {
        return String::new();
    };
    let mut end = first.len();
    for name in rest {
        end = first
            .char_indices()
            .zip(name.chars())
            .find(|((_, a), b)| a != b)
            .map_or(end.min(name.len()), |((i, _), _)| i.min(end));
    }
    first[..end].to_owned()
}

/// Completion reads running now. Beyond `MAX_READS` (folders that hang), none starts.
static READS: AtomicUsize = AtomicUsize::new(0);
const MAX_READS: usize = 4;

/// Runs `job` on a thread of its own: a folder that hangs (a network share gone) holds that
/// thread, never the interface nor iced's pool. None when too many such jobs hang already.
async fn off_thread<T: Send + 'static>(
    job: impl FnOnce() -> Option<T> + Send + 'static,
) -> Option<T> {
    if READS.fetch_add(1, Ordering::SeqCst) >= MAX_READS {
        READS.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("evidence-path".into())
        .spawn(move || {
            let out = job();
            READS.fetch_sub(1, Ordering::SeqCst);
            let _ = sender.send(out);
        });
    if spawned.is_err() {
        READS.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    receiver.await.ok().flatten()
}

/// The subfolders of `dir`, at most `COMPLETION_MAX_ENTRIES` entries read within
/// `COMPLETION_TIMEOUT`, off the interface thread.
pub async fn read_folders(dir: PathBuf) -> Option<Folders> {
    off_thread(move || folders_in(&dir)).await
}

/// What a typed path leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    Folder,
    /// A file, a link to one, or a special file: its folder opens with it selected.
    File,
    Nothing,
    /// A folder on the way cannot be entered.
    Denied,
}

/// What `path` is, off the interface thread. `Nothing` when too many checks hang already.
pub async fn kind_of(path: PathBuf) -> Found {
    let found = off_thread(move || {
        Some(match std::fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => Found::Folder,
            Ok(_) => Found::File,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Found::Denied,
            Err(_) => Found::Nothing,
        })
    });
    found.await.unwrap_or(Found::Nothing)
}

fn folders_in(dir: &Path) -> Option<Folders> {
    let start = Instant::now();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)
        .ok()?
        .take(more::COMPLETION_MAX_ENTRIES)
    {
        if start.elapsed() > more::COMPLETION_TIMEOUT {
            break;
        }
        let Ok(entry) = entry else { continue };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.chars().any(unsafe_char) {
            continue;
        }
        // A link counts when it leads to a folder.
        let folder = match entry.file_type() {
            Ok(kind) if kind.is_symlink() => entry.path().is_dir(),
            Ok(kind) => kind.is_dir(),
            Err(_) => false,
        };
        if folder {
            names.push(name);
        }
    }
    names.sort_by_key(|n| n.to_lowercase());
    Some(Folders {
        dir: dir.to_path_buf(),
        names,
    })
}

/// A character that must not reach the screen as it is: controls (a line feed, an escape),
/// and the marks that reverse the direction of a text (a name that shows « exe.txt » for an `.exe`).
fn unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `name` safe to show: a control character becomes its picture (␊ for a line feed), a
/// direction mark becomes �.
pub fn visible(name: &str) -> Cow<'_, str> {
    if !name.chars().any(unsafe_char) {
        return Cow::Borrowed(name);
    }
    Cow::Owned(
        name.chars()
            .map(|c| match c {
                '\0'..='\u{1F}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{FFFD}'),
                '\u{7F}' => '\u{2421}',
                c if unsafe_char(c) => '\u{FFFD}',
                c => c,
            })
            .collect(),
    )
}

/// How many segments of the breadcrumb to fold behind « … », after the first, so that the
/// others fit `room`: `widths` of each segment, `chevron` between two, `fold` the width of
/// « … » and its chevron. The first and the last stay.
pub fn fold(widths: &[f32], chevron: f32, fold: f32, room: f32) -> usize {
    let total: f32 = widths.iter().sum::<f32>() + chevron * widths.len().saturating_sub(1) as f32;
    let mut width = total;
    let mut folded = 0;
    while width > room && folded + 2 < widths.len() {
        width -= widths[folded + 1] + chevron;
        if folded == 0 {
            width += fold;
        }
        folded += 1;
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_typed_path_expands_the_home_and_starts_from_the_folder() {
        let (cwd, home) = (Path::new("/srv/data"), Path::new("/home/ada"));
        let at = |s: &str| resolve(s, cwd, home);
        assert_eq!(at("~"), Some(PathBuf::from("/home/ada")));
        assert_eq!(
            at("~/Documents/"),
            Some(PathBuf::from("/home/ada/Documents"))
        );
        assert_eq!(at("/etc"), Some(PathBuf::from("/etc")));
        assert_eq!(at("logs"), Some(PathBuf::from("/srv/data/logs")));
        assert_eq!(at("../x/./y"), Some(PathBuf::from("/srv/x/y")));
        assert_eq!(at("/../.."), Some(PathBuf::from("/")));
        assert_eq!(at("~ada"), Some(PathBuf::from("/srv/data/~ada")));
        assert_eq!(at("   "), None);
        // A paste: blanks around, a link, an escape, controls and lengths refused.
        assert_eq!(at("  /etc/ \n"), Some(PathBuf::from("/etc")));
        assert_eq!(
            at("file:///home/ada/Mes%20docs"),
            Some(PathBuf::from("/home/ada/Mes docs"))
        );
        assert_eq!(at("file://localhost/tmp"), Some(PathBuf::from("/tmp")));
        assert_eq!(at("file://serveur/x"), None);
        assert_eq!(at("file:///a%2"), None);
        assert_eq!(at("file:///a%00b"), None);
        assert_eq!(at("/a\u{1b}b"), None);
        assert_eq!(at(&"a".repeat(MAX_PATH + 1)), None);
        use std::os::unix::ffi::OsStrExt;
        let raw = at("file:///x%FF").expect("bytes");
        assert_eq!(raw.as_os_str().as_bytes(), b"/x\xFF");
    }

    #[test]
    fn the_text_splits_before_the_name_being_typed() {
        assert_eq!(split("~/Doc"), (Cow::Borrowed("~/"), "Doc"));
        assert_eq!(split("~"), (Cow::Borrowed("~/"), ""));
        assert_eq!(split("/"), (Cow::Borrowed("/"), ""));
        assert_eq!(split("src"), (Cow::Borrowed(""), "src"));
    }

    #[test]
    fn completion_offers_folders_only_and_hidden_ones_after_a_dot() {
        let tmp = TempDir::new("completion");
        for dir in ["Documents", "Downloads", "desktop", ".config", "bad\nname"] {
            std::fs::create_dir(tmp.path().join(dir)).expect("folder");
        }
        std::fs::write(tmp.path().join("Doc.txt"), "").expect("file");
        let folders = folders_in(tmp.path()).expect("read");
        assert_eq!(
            folders.names,
            [".config", "desktop", "Documents", "Downloads"]
        );
        assert_eq!(matches(&folders, "do", false), ["Documents", "Downloads"]);
        assert_eq!(
            matches(&folders, "D", false),
            ["desktop", "Documents", "Downloads"]
        );
        assert_eq!(matches(&folders, ".", false), [".config"]);
        assert_eq!(matches(&folders, "", true)[0], ".config");
        assert_eq!(matches(&folders, "Documents", false), Vec::<String>::new());
        assert_eq!(common_prefix(&matches(&folders, "Do", false)), "Do");
        assert_eq!(common_prefix(&matches(&folders, "Doc", false)), "Documents");
        assert!(folders_in(&tmp.path().join("missing")).is_none());
        let read = iced::futures::executor::block_on(read_folders(tmp.path().to_path_buf()));
        assert_eq!(read.map(|f| f.names.len()), Some(4));
    }

    #[test]
    fn control_characters_and_direction_marks_never_show() {
        assert_eq!(visible("notes.txt"), "notes.txt");
        assert_eq!(visible("a\nb\x1b[31m"), "a␊b␛[31m");
        assert_eq!(visible("photo\u{202E}gpj.exe"), "photo\u{FFFD}gpj.exe");
        assert_eq!(visible("del\x7f"), "del␡");
    }

    #[test]
    fn segments_fold_from_the_second_until_the_rest_fits() {
        let widths = [60.0, 80.0, 80.0, 80.0, 100.0];
        assert_eq!(fold(&widths, 10.0, 30.0, 500.0), 0);
        assert_eq!(fold(&widths, 10.0, 30.0, 400.0), 1);
        assert_eq!(fold(&widths, 10.0, 30.0, 250.0), 3);
        // The first and the last always stay.
        assert_eq!(fold(&widths, 10.0, 30.0, 10.0), 3);
    }
}
