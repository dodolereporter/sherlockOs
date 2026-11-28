//! A stack of the dock (lot D-21k), as on macOS: the downloads folder, between the applications
//! and the trash; a click opens its newest items in a grid over the dock, each opened by its
//! default application, and « Ouvrir dans Evidence ». The folder is found and read on threads of
//! their own, with bounds: never on the interface thread.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use dock_config::stack::Sort;

use iced::futures::Stream;
use iced::futures::channel::mpsc;
use sherlock_ui::icon::Icon;

/// Entries of the folder looked at, at most, to find the newest ones.
pub const MAX_LOOKED_AT: usize = 4096;
/// Items the grid shows: four rows of four, the newest first.
pub const MAX_SHOWN: usize = 16;
/// Largest `user-dirs.dirs` read.
const MAX_USER_DIRS: u64 = 64 * 1024;

/// A stack: its folder, its name in the dock, its newest items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    pub folder: PathBuf,
    pub name: String,
    pub items: Vec<Shown>,
}

/// An item of a stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub path: PathBuf,
    pub name: String,
    pub folder: bool,
    /// Its picture in the grid, as Evidence draws it in its lists.
    pub icon: Icon,
}

/// The downloads stack, each time its folder changes; `None` without a downloads folder. The
/// thread stops when the stream is dropped.
pub fn watch(home: PathBuf, config_home: PathBuf, sort: Sort) -> impl Stream<Item = Option<Stack>> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name(String::from("watch-stack"))
        .spawn(move || {
            let Some(folder) = downloads(&home, &config_home) else {
                let _ = sender.unbounded_send(None);
                return;
            };
            let name = folder
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let read = || {
                Some(Stack {
                    folder: folder.clone(),
                    name: name.clone(),
                    items: newest(&folder, sort),
                })
            };
            if let Err(error) = crate::trash::follow(&folder, read, &sender) {
                tracing::warn!(%error, "cannot follow the downloads folder");
            }
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "cannot start the stack watcher");
    }
    receiver
}

/// The downloads folder of the user: `XDG_DOWNLOAD_DIR` of `$XDG_CONFIG_HOME/user-dirs.dirs`
/// (`$HOME/` replaced), else `~/Downloads`; only when it exists. Reads a file: off the
/// interface thread.
pub fn downloads(home: &Path, config_home: &Path) -> Option<PathBuf> {
    let text = mime_apps::read_bounded(&config_home.join("user-dirs.dirs"), MAX_USER_DIRS).ok();
    let named = text.as_deref().and_then(|text| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=")?;
            let value = value.trim().trim_matches('"');
            let path = match value.strip_prefix("$HOME") {
                Some(rest) => home.join(rest.trim_start_matches('/')),
                None => PathBuf::from(value),
            };
            // `$HOME` itself is no downloads folder; a relative path is refused.
            (path.is_absolute() && path != home).then_some(path)
        })
    });
    named
        .or_else(|| Some(home.join("Downloads")))
        .filter(|path| path.is_dir())
}

/// The first items of `folder` in the order `sort` (lot D-21k2: the dates newest first, the
/// name and the kind in alphabetical order), hidden ones left out, `MAX_SHOWN` at most among
/// the first `MAX_LOOKED_AT` entries.
pub fn newest(folder: &Path, sort: Sort) -> Vec<Shown> {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, Shown)> = dir
        .flatten()
        .take(MAX_LOOKED_AT)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let meta = entry.metadata().ok()?;
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let when = match sort {
                // Its status change: when it came into the folder (moved or written there).
                Sort::Added => {
                    let secs = u64::try_from(meta.ctime()).unwrap_or(0);
                    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)
                }
                Sort::Created => meta.created().unwrap_or(modified),
                _ => modified,
            };
            let icon = icon_of(&name, meta.is_dir());
            Some((
                when,
                Shown {
                    path: entry.path(),
                    name,
                    folder: meta.is_dir(),
                    icon,
                },
            ))
        })
        .collect();
    let kind = |name: &str| {
        let extension = Path::new(name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        extension.unwrap_or_default()
    };
    match sort {
        Sort::Name => found.sort_by_key(|a| a.1.name.to_lowercase()),
        Sort::Kind => found.sort_by(|a, b| {
            (!a.1.folder, kind(&a.1.name), a.1.name.to_lowercase()).cmp(&(
                !b.1.folder,
                kind(&b.1.name),
                b.1.name.to_lowercase(),
            ))
        }),
        _ => found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name))),
    }
    found
        .into_iter()
        .take(MAX_SHOWN)
        .map(|(_, shown)| shown)
        .collect()
}

/// The picture of an item, as in Evidence's lists (`listing::Entry::icon`).
fn icon_of(name: &str, folder: bool) -> Icon {
    let extension = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase());
    match extension.as_deref() {
        _ if folder => Icon::Folder,
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") => Icon::Image,
        _ => Icon::Document,
    }
}

/// Opens `path` with its default application, a folder in Evidence, on a thread of its own.
pub fn open(path: PathBuf, folder: bool) {
    let spawned = std::thread::Builder::new()
        .name(String::from("dock-open"))
        .spawn(move || {
            let result = if folder {
                crate::launch::open_in_evidence(&path.to_string_lossy())
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            } else {
                open_file(&path)
            };
            if let Err(error) = result {
                tracing::warn!(%error, "cannot open the item of the stack");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cannot start a thread to open the item");
    }
}

fn open_file(path: &Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let dirs = mime_apps::Dirs::from_env(&home);
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .map(str::to_lowercase)
        .collect();
    let catalog = mime_apps::Catalog::load(dirs, &desktops, &[]);
    let mime = catalog.mime.type_of(path, false);
    let app = catalog
        .default_for(&mime)
        .ok_or_else(|| format!("no application opens {mime}"))?;
    let (program, args) = catalog
        .command(app, &[path.to_path_buf()])
        .ok_or_else(|| format!("{} has no command", app.id))?;
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut child| {
            let _ = std::thread::Builder::new()
                .name(String::from("dock-child"))
                .spawn(move || child.wait());
        })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_downloads_folder_is_found_and_its_newest_items_listed() {
        let root = std::env::temp_dir().join(format!("dock-stack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, config) = (root.join("home"), root.join("home/.config"));
        std::fs::create_dir_all(home.join("Downloads-fr")).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        assert_eq!(downloads(&home, &config), None, "no file, no ~/Downloads");
        std::fs::write(
            config.join("user-dirs.dirs"),
            "# written by xdg-user-dirs-update\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads-fr\"\n",
        )
        .unwrap();
        let folder = home.join("Downloads-fr");
        assert_eq!(downloads(&home, &config), Some(folder.clone()));
        // `$HOME` itself is refused.
        std::fs::write(
            config.join("user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"$HOME\"\n",
        )
        .unwrap();
        assert_eq!(downloads(&home, &config), None);

        for (i, name) in ["old.pdf", "photo.png", ".cache", "new.zip"]
            .iter()
            .enumerate()
        {
            let path = folder.join(name);
            std::fs::write(&path, "x").unwrap();
            let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + i as u64);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        std::fs::create_dir(folder.join("folder")).unwrap();
        let names: Vec<String> = newest(&folder, Sort::Modified)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names[..4], ["folder", "new.zip", "photo.png", "old.pdf"]);
        let icons: Vec<Icon> = newest(&folder, Sort::Modified)
            .into_iter()
            .map(|s| s.icon)
            .collect();
        assert_eq!(
            icons,
            [Icon::Folder, Icon::Document, Icon::Image, Icon::Document]
        );
        // By name, by kind (the folders first, then by extension).
        let by =
            |sort| -> Vec<String> { newest(&folder, sort).into_iter().map(|s| s.name).collect() };
        assert_eq!(
            by(Sort::Name),
            ["folder", "new.zip", "old.pdf", "photo.png"]
        );
        assert_eq!(
            by(Sort::Kind),
            ["folder", "old.pdf", "photo.png", "new.zip"]
        );
        assert_eq!(names.len(), 4, "the hidden one is left out");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
