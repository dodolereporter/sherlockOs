//! « Ouvrir à l'ouverture de session » (lot D-21i): an application starts with the session when
//! its desktop file is in the XDG autostart folder, `$XDG_CONFIG_HOME/autostart`, which the
//! session reads (lot D-44). The dock copies the application's own desktop file there, or takes
//! it out. Every read and write runs on a thread of its own, never on the interface's.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

/// Largest desktop file copied: the bound of `mime_apps::read_entry`.
pub const MAX_ENTRY_BYTES: u64 = 256 * 1024;

/// The autostart folder, without a home or `XDG_CONFIG_HOME`: none.
pub fn folder() -> Option<PathBuf> {
    Some(dock_config::config_home()?.join("autostart"))
}

/// The ids whose file is in `folder` and not `Hidden` (a user's way of turning one off).
pub fn read(folder: &Path) -> BTreeSet<String> {
    let Ok(dir) = std::fs::read_dir(folder) else {
        return BTreeSet::new();
    };
    dir.flatten()
        .take(1024)
        .filter_map(|file| {
            let path = file.path();
            let id = path
                .file_name()?
                .to_str()?
                .strip_suffix(".desktop")?
                .to_owned();
            let text = mime_apps::entry::read_entry(&path)?;
            let hidden = text.lines().any(|l| l.trim() == "Hidden=true");
            (!hidden).then_some(id)
        })
        .collect()
}

/// Puts the application `id`, whose desktop file is `entry`, in `folder` (`on`), or takes it out.
pub fn set(folder: &Path, id: &str, entry: &Path, on: bool) -> io::Result<()> {
    let target = folder.join(format!("{id}.desktop"));
    if !on {
        return match std::fs::remove_file(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        };
    }
    let text = mime_apps::read_bounded(entry, MAX_ENTRY_BYTES)?;
    std::fs::create_dir_all(folder)?;
    mime_apps::write_atomically(&target, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_application_is_put_in_the_autostart_folder_and_taken_out() {
        let dir = std::env::temp_dir().join(format!("dock-autostart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join("autostart");
        let entry = dir.join("org.example.Files.desktop");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &entry,
            "[Desktop Entry]\nType=Application\nName=Fichiers\nExec=files\n",
        )
        .unwrap();
        assert!(read(&folder).is_empty());
        set(&folder, "org.example.Files", &entry, true).unwrap();
        assert_eq!(
            read(&folder),
            BTreeSet::from([String::from("org.example.Files")])
        );
        // Turned off by the user's own way: Hidden=true.
        std::fs::write(folder.join("off.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();
        assert!(!read(&folder).contains("off"));
        set(&folder, "org.example.Files", &entry, false).unwrap();
        set(&folder, "org.example.Files", &entry, false).unwrap();
        assert!(!read(&folder).contains("org.example.Files"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
