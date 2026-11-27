//! The icons of the notification area kept in the bar (lot D-45a), by their `Id`, in their
//! order, in the bar's settings file, `$XDG_CONFIG_HOME/bar/bar.conf`:
//!
//! ```text
//! [Tray]
//! Pinned=discord;steam
//! ```
//!
//! An `Id` is the application's: a `;` in it is written `\;`, as in a desktop entry, so that
//! it never splits the list (lot D-45c). A file written before reads the same: its ids had no
//! escape, and the next change writes it in this form.
//!
//! A change reads the file again just before it writes it, so that it starts from what is there
//! (another bar, the user's editor), and keeps its other lines. The text goes to a temporary file
//! renamed over the old one: a reader never sees half a file.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use dock_config::KeyFile;

const GROUP: &str = "Tray";
const PINNED: &str = "Pinned";

/// Path of the file: `$XDG_CONFIG_HOME/bar/bar.conf`.
pub fn path() -> Option<PathBuf> {
    Some(dock_config::config_home()?.join("bar/bar.conf"))
}

fn read(path: &Path) -> io::Result<KeyFile> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(KeyFile::parse(&text)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(KeyFile::default()),
        Err(error) => Err(error),
    }
}

/// The ids kept in the bar, in their order. An absent file keeps none.
pub fn load(path: &Path) -> io::Result<Vec<String>> {
    let file = read(path)?;
    Ok(file.list(GROUP, PINNED))
}

/// What a drop does to the icons kept in the bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The icon `id` goes into the bar, at `index` among those kept there, or moves there.
    Pin { id: String, index: usize },
    /// The icon `id` goes back into the popup.
    Unpin(String),
}

impl Change {
    /// Applies it to `pinned`.
    pub fn apply(&self, pinned: &mut Vec<String>) {
        match self {
            Self::Pin { id, index } => {
                let before = pinned.iter().position(|p| p == id);
                pinned.retain(|p| p != id);
                // Moved to the right, the icon left its place before the index.
                let index = match before {
                    Some(at) if at < *index => index - 1,
                    _ => *index,
                };
                pinned.insert(index.min(pinned.len()), id.clone());
            }
            Self::Unpin(id) => pinned.retain(|p| p != id),
        }
    }
}

/// Applies `change` to the file at `path`, read just before, and returns the icons kept now.
pub fn change(path: &Path, change: &Change) -> io::Result<Vec<String>> {
    let mut file = read(path)?;
    let mut pinned = file.list(GROUP, PINNED);
    change.apply(&mut pinned);
    file.set_list(GROUP, PINNED, &pinned);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let count = SAVES.fetch_add(1, Ordering::Relaxed);
    let temporary = dir.join(format!(".{name}.{}.{count}.tmp", std::process::id()));
    let written = (|| {
        let mut out = std::fs::File::create(&temporary)?;
        out.write_all(file.to_text().as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written.map(|()| pinned)
}

/// Saves made by this process, for unique temporary names.
static SAVES: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    fn pin(id: &str, index: usize) -> Change {
        Change::Pin {
            id: id.to_owned(),
            index,
        }
    }

    #[test]
    fn a_drop_pins_moves_and_unpins() {
        let mut pinned = Vec::new();
        pin("discord", 0).apply(&mut pinned);
        pin("steam", 1).apply(&mut pinned);
        pin("vpn", 0).apply(&mut pinned);
        assert_eq!(pinned, ["vpn", "discord", "steam"]);
        // Moved to the right, past « discord »: it lands just after it.
        pin("vpn", 2).apply(&mut pinned);
        assert_eq!(pinned, ["discord", "vpn", "steam"]);
        pin("steam", 0).apply(&mut pinned);
        assert_eq!(pinned, ["steam", "discord", "vpn"]);
        pin("other", 40).apply(&mut pinned);
        assert_eq!(pinned.last().map(String::as_str), Some("other"));
        Change::Unpin(String::from("discord")).apply(&mut pinned);
        assert_eq!(pinned, ["steam", "vpn", "other"]);
    }

    #[test]
    fn the_file_is_read_just_before_it_is_written_and_keeps_its_other_lines() {
        let dir = std::env::temp_dir().join(format!("bar-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("bar/bar.conf");
        assert_eq!(load(&path).unwrap(), Vec::<String>::new());
        assert_eq!(change(&path, &pin("discord", 0)).unwrap(), ["discord"]);
        // Someone else writes the file meanwhile: the next change starts from it.
        let text = std::fs::read_to_string(&path).unwrap();
        let edited = format!(
            "# kept\n[Other]\nKey=1\n{}",
            text.replace("discord", "steam;discord")
        );
        std::fs::write(&path, edited).unwrap();
        assert_eq!(
            change(&path, &pin("vpn", 1)).unwrap(),
            ["steam", "vpn", "discord"]
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Key=1") && text.contains("# kept"), "{text}");
        assert_eq!(load(&path).unwrap(), ["steam", "vpn", "discord"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
