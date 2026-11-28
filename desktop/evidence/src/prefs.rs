//! What Evidence remembers from one start to the next (lot E-08): whether the preview is shown.
//! One line, `preview=on` or `preview=off`, in `$XDG_CONFIG_HOME/evidence/settings`.

use std::io;
use std::path::{Path, PathBuf};

/// Longest settings file read: a few lines.
const MAX_SIZE: u64 = 4096;

/// The settings file under the configuration folder.
pub fn path(config_home: &Path) -> PathBuf {
    config_home.join("evidence").join("settings")
}

/// Whether the preview was shown, if the file says so. Read with a bound, as every settings
/// file of the desktop (`mime_apps::file`, lot Q-41).
pub fn read_preview(path: &Path) -> Option<bool> {
    let text = mime_apps::file::read_bounded(path, MAX_SIZE).ok()?;
    text.lines().find_map(|line| match line.trim() {
        "preview=on" => Some(true),
        "preview=off" => Some(false),
        _ => None,
    })
}

/// Remembers whether the preview is shown, through the desktop's one atomic writer: a crash
/// never leaves half a file.
pub fn write_preview(path: &Path, shown: bool) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let value = if shown { "on" } else { "off" };
    mime_apps::file::write_atomically(path, &format!("preview={value}\n"))
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
