//! The pinned applications as the dock shows them, read and changed by the other programs of the
//! shell (the applications view, the launcher): the list of the configuration file, or, without
//! one, the defaults the dock chooses (`config::defaults`). Changing it writes the file, which
//! the dock reads again at once (`watch`).

use std::io;
use std::path::Path;

use dock_config::DockConfig;

use crate::config;
use crate::entry::{self, Catalog};

/// The pinned list of `config`, or the dock's defaults for the installed applications.
pub fn current(config: &DockConfig) -> Vec<String> {
    config.pinned.clone().unwrap_or_else(defaults)
}

/// The dock's defaults: what it pins without a list in its file.
fn defaults() -> Vec<String> {
    let catalog = Catalog::load(&entry::data_dirs());
    let mimeapps = dock_config::config_home()
        .as_deref()
        .map(config::mimeapps)
        .unwrap_or_default();
    config::defaults(&catalog, &mimeapps)
}

/// Whether `id` (a desktop file id, with or without `.desktop`) is pinned in the file at
/// `path`. An unreadable file reads as the defaults.
pub fn is_pinned(path: &Path, id: &str) -> bool {
    let config = DockConfig::load(path).map(|(c, _)| c).unwrap_or_default();
    current(&config).iter().any(|p| p == bare(id))
}

/// Pins `id` at the end of the list, or unpins it, in the file at `path`. The file is written
/// only when the list changes; the list written is the one the dock showed, defaults included,
/// Sherlock's token (`dock_config::SHERLOCK`) kept where it is.
/// Returns whether it changed.
pub fn set(path: &Path, id: &str, pinned: bool) -> io::Result<bool> {
    set_with(path, id, pinned, defaults)
}

/// The same, the defaults given by `defaults` (the tests).
pub fn set_with(
    path: &Path,
    id: &str,
    pinned: bool,
    defaults: impl FnOnce() -> Vec<String>,
) -> io::Result<bool> {
    let (mut config, _) = DockConfig::load(path)?;
    let mut list = config.pinned.clone().unwrap_or_else(defaults);
    let id = bare(id);
    let before = list.clone();
    if pinned {
        if !list.iter().any(|p| p == id) {
            list.push(id.to_owned());
        }
    } else {
        list.retain(|p| p != id);
    }
    if list == before {
        return Ok(false);
    }
    config.pinned = Some(list);
    config.save(path)?;
    Ok(true)
}

/// Writes the pinned list and whether Sherlock is shown to the file at `path`, read again
/// just before: what another program changed in it since the dock last read it (a setting
/// of Control Room, a comment) is kept. The dock's list wins: it is the order on screen.
pub fn write(
    path: &Path,
    pinned: &[String],
    sherlock: bool,
    stack: dock_config::stack::Stack,
) -> io::Result<()> {
    DockConfig::update(path, |config| {
        config.pinned = Some(pinned.to_vec());
        config.sherlock = sherlock;
        // The settings of the downloads stack, from its menu (lot D-21k2).
        config.stack = stack;
    })
    .map(|_| ())
}

/// A desktop file id without `.desktop`, as the file lists it.
fn bare(id: &str) -> &str {
    id.strip_suffix(".desktop").unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Option<Vec<String>> {
        DockConfig::load(path).ok().and_then(|(c, _)| c.pinned)
    }

    fn list(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| String::from(*id)).collect()
    }

    #[test]
    fn pinning_writes_the_list_the_dock_shows() {
        let dir = std::env::temp_dir().join(format!("dock-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock/dock.conf");
        let defaults = || list(&["evidence", "firefox"]);
        // No file: the defaults, and the new one at their end.
        assert!(set_with(&path, "kitty.desktop", true, defaults).unwrap());
        assert_eq!(read(&path), Some(list(&["evidence", "firefox", "kitty"])));
        // Pinned already: nothing written.
        assert!(!set_with(&path, "kitty", true, defaults).unwrap());
        // Unpinned: the others stay in their order.
        assert!(set_with(&path, "firefox", false, defaults).unwrap());
        assert_eq!(read(&path), Some(list(&["evidence", "kitty"])));
        assert!(!set_with(&path, "firefox", false, defaults).unwrap());
        // The other keys of the file are kept.
        std::fs::write(&path, "[Dock]\nPinned=evidence;\nIconSize=64\nFuture=1\n").unwrap();
        assert!(set_with(&path, "kitty", true, defaults).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "[Dock]\nPinned=evidence;kitty;\nIconSize=64\nFuture=1\n"
        );
        assert!(is_pinned(&path, "kitty.desktop") && !is_pinned(&path, "firefox"));
        // Sherlock's token stays where the dock put it.
        std::fs::write(&path, "[Dock]\nPinned=sherlock;evidence;\n").unwrap();
        assert!(set_with(&path, "kitty", true, defaults).unwrap());
        assert_eq!(read(&path), Some(list(&["sherlock", "evidence", "kitty"])));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_is_read_again_before_it_is_written() {
        let dir = std::env::temp_dir().join(format!("dock-pins-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock/dock.conf");
        std::fs::create_dir_all(dir.join("dock")).unwrap();
        // Another program changed the size since the dock read the file.
        std::fs::write(&path, "[Dock]\n# mine\nPinned=a;b;\nIconSize=64\n").unwrap();
        write(
            &path,
            &list(&["b", "sherlock", "a"]),
            true,
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Dock]\n# mine\nPinned=b;sherlock;a;\nIconSize=64\n"
        );
        write(&path, &list(&["b", "a"]), false, Default::default()).unwrap();
        let (config, _) = DockConfig::load(&path).unwrap();
        assert_eq!((config.icon_size, config.sherlock), (64.0, false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_file_unpinning_a_default_keeps_the_others() {
        let dir = std::env::temp_dir().join(format!("dock-pins-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("dock/dock.conf");
        let defaults = || list(&["evidence", "firefox"]);
        assert!(set_with(&path, "evidence", false, defaults).unwrap());
        assert_eq!(read(&path), Some(list(&["firefox"])));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
