//! What Evidence remembers from one start to the next (lot E-08): whether the preview is shown.
//! One line, `preview=on` or `preview=off`, in `$XDG_CONFIG_HOME/evidence/settings`.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Longest settings file read: a few lines.
const MAX_SIZE: u64 = 4096;

/// The settings file under the configuration folder.
pub fn path(config_home: &Path) -> PathBuf {
    config_home.join("evidence").join("settings")
}

/// Whether the preview was shown, if the file says so.
pub fn read_preview(path: &Path) -> Option<bool> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_SIZE)
        .read_to_string(&mut text)
        .ok()?;
    text.lines().find_map(|line| match line.trim() {
        "preview=on" => Some(true),
        "preview=off" => Some(false),
        _ => None,
    })
}

/// Remembers whether the preview is shown: written whole to a temporary file, then renamed, so
/// that a crash never leaves half a file.
pub fn write_preview(path: &Path, shown: bool) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let temporary = path.with_extension("new");
    let mut file = std::fs::File::create(&temporary)?;
    let value = if shown { "on" } else { "off" };
    writeln!(file, "preview={value}")?;
    file.sync_all()?;
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_preview_choice_is_remembered() {
        let tmp = TempDir::new("prefs");
        let file = path(tmp.path());
        assert_eq!(read_preview(&file), None);
        write_preview(&file, false).expect("write");
        assert_eq!(read_preview(&file), Some(false));
        write_preview(&file, true).expect("write");
        assert_eq!(read_preview(&file), Some(true));
        std::fs::write(&file, "autre=1\n").expect("write");
        assert_eq!(read_preview(&file), None);
    }
}
