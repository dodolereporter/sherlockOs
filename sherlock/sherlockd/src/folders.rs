//! The folders the files server may reach (lot A-13, ADR 0033), and how a path asked by the
//! model is resolved inside one of them without ever leaving it.
//!
//! The folders come from a settings file, `$XDG_CONFIG_HOME/sherlock/files.json`
//! (`{"folders": ["~/Documents", …]}`), or by default from the user's documents, desktop and
//! downloads folders (`user-dirs.dirs`). A folder is refused if it holds, or lies inside, a
//! place of secrets: `~/.ssh`, `~/.gnupg`, the keyrings, Claude Code's files, Sherlock's own
//! settings, state and data (so `~` itself is refused). Each folder is opened once, at start;
//! every later path is resolved from that descriptor with `openat2(RESOLVE_BENEATH)`: a `..`,
//! an absolute symbolic link or one that climbs out is refused by the kernel, with no window
//! between a check and a use. Without `openat2` (Linux before 5.6), nothing is reached.

use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{CWD, Mode, OFlags, ResolveFlags};
use rustix::io::Errno;
use serde::Deserialize;

/// Bytes of the settings file at most.
const MAX_SETTINGS: u64 = 64 * 1024;
/// Folders kept at most.
const MAX_FOLDERS: usize = 32;

/// Where the user's own places are: from the environment of the session.
#[derive(Debug, Clone)]
pub struct Places {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
}

impl Places {
    /// From `HOME` and the `XDG_*_HOME` variables; `None` without an absolute `HOME`.
    pub fn of_session() -> Option<Self> {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        if !home.is_absolute() {
            return None;
        }
        let xdg = |name: &str, default: &str| {
            std::env::var_os(name)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        Some(Self {
            config: xdg("XDG_CONFIG_HOME", ".config"),
            data: xdg("XDG_DATA_HOME", ".local/share"),
            state: xdg("XDG_STATE_HOME", ".local/state"),
            home,
        })
    }

    /// The settings file: `$XDG_CONFIG_HOME/sherlock/files.json`.
    pub fn settings(&self) -> PathBuf {
        self.config.join("sherlock/files.json")
    }

    /// The places of secrets, which no folder may hold or lie in.
    fn secrets(&self) -> Vec<PathBuf> {
        let home = &self.home;
        let mut secrets: Vec<PathBuf> = [
            ".ssh",
            ".gnupg",
            ".pki",
            ".password-store",
            ".local/share/keyrings",
            ".local/share/kwalletd",
            ".claude",
            ".claude.json",
            ".config/claude",
        ]
        .iter()
        .map(|p| home.join(p))
        .collect();
        secrets.push(self.data.join("keyrings"));
        // A secret behind a link is also refused where the link leads.
        let real: Vec<PathBuf> = secrets
            .iter()
            .filter_map(|s| std::fs::canonicalize(s).ok())
            .collect();
        secrets.extend(real);
        secrets
    }

    /// The folders whose entries named `sherlock*` are Sherlock's own: its settings, its
    /// journal, its data.
    fn sherlock_parents(&self) -> [&Path; 3] {
        [&self.config, &self.data, &self.state]
    }

    /// `~` and `~/…` expanded; anything else as it is.
    fn expand(&self, asked: &str) -> PathBuf {
        match asked.strip_prefix('~') {
            Some("") => self.home.clone(),
            Some(rest) if rest.starts_with('/') => self.home.join(&rest[1..]),
            _ => PathBuf::from(asked),
        }
    }

    /// The user's documents, desktop and downloads folders, from `user-dirs.dirs`, or
    /// `~/Documents`, `~/Bureau` and `~/Téléchargements`.
    fn default_folders(&self) -> Vec<PathBuf> {
        let dirs = std::fs::read_to_string(self.config.join("user-dirs.dirs")).unwrap_or_default();
        let named = |key: &str| {
            dirs.lines().find_map(|line| {
                let value = line.trim().strip_prefix(key)?.strip_prefix('=')?;
                let value = value.trim().trim_matches('"');
                let path = match value.strip_prefix("$HOME") {
                    Some(rest) => self.home.join(rest.trim_start_matches('/')),
                    None => PathBuf::from(value),
                };
                path.is_absolute().then_some(path)
            })
        };
        [
            ("XDG_DOCUMENTS_DIR", "Documents"),
            ("XDG_DESKTOP_DIR", "Bureau"),
            ("XDG_DOWNLOAD_DIR", "Téléchargements"),
        ]
        .iter()
        .map(|(key, default)| named(key).unwrap_or_else(|| self.home.join(default)))
        .collect()
    }

    /// Why `path`, canonical, may not be a folder of Sherlock's, if it may not.
    fn forbidden(&self, path: &Path) -> Option<String> {
        if path == Path::new("/") {
            return Some(String::from("la racine contient des secrets"));
        }
        for secret in self.secrets() {
            if path.starts_with(&secret) || secret.starts_with(path) {
                return Some(format!("{} est un lieu de secrets", secret.display()));
            }
        }
        for parent in self.sherlock_parents() {
            if parent.starts_with(path) {
                return Some(format!(
                    "{} contient les réglages de Sherlock",
                    path.display()
                ));
            }
            let own = path
                .strip_prefix(parent)
                .ok()
                .and_then(|r| r.components().next());
            if own.is_some_and(|c| c.as_os_str().as_bytes().starts_with(b"sherlock")) {
                return Some(String::from("les fichiers de Sherlock lui sont interdits"));
            }
        }
        None
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    folders: Vec<String>,
    /// The trash, for the tests; the session's is `$XDG_DATA_HOME/Trash`.
    #[serde(default)]
    trash: Option<String>,
}

/// One allowed folder: the path the user wrote, expanded, its canonical path, and its
/// descriptor, opened once.
#[derive(Debug)]
pub struct Folder {
    pub shown: PathBuf,
    pub real: PathBuf,
    fd: OwnedFd,
}

/// The allowed folders, and the trash.
#[derive(Debug)]
pub struct Folders {
    pub list: Vec<Folder>,
    pub trash: PathBuf,
    /// Folders of the settings that were refused, and why: told to the model on request.
    pub refused: Vec<(String, String)>,
}

/// A path resolved inside a folder: the folder, and the path relative to it (empty for the
/// folder itself), made only of plain names.
#[derive(Debug)]
pub struct Inside<'a> {
    pub folder: &'a Folder,
    pub relative: PathBuf,
}

impl Folders {
    /// The folders of `settings` (the session's settings file when `None`), or the defaults
    /// when the session has none.
    pub fn load(places: &Places, settings: Option<&Path>) -> Self {
        let file = settings.map_or_else(|| places.settings(), Path::to_owned);
        let read = read_settings(&file);
        let (asked, trash) = match read {
            Some(settings) => (
                settings.folders.iter().map(|f| places.expand(f)).collect(),
                settings.trash.map(|t| places.expand(&t)),
            ),
            None => (places.default_folders(), None),
        };
        let mut folders = Self {
            list: Vec::new(),
            trash: trash.unwrap_or_else(|| places.data.join("Trash")),
            refused: Vec::new(),
        };
        for shown in asked.into_iter().take(MAX_FOLDERS) {
            match open_folder(places, &shown) {
                Ok(folder) => folders.list.push(folder),
                Err(why) => {
                    tracing::warn!(folder = ?shown, why, "folder refused");
                    folders.refused.push((shown.display().to_string(), why));
                }
            }
        }
        // The longest first: a folder inside another one answers for its own paths.
        folders
            .list
            .sort_by_key(|f| std::cmp::Reverse(f.real.components().count()));
        folders
    }

    /// The folder `asked` lies in, and its path there, made only of plain names. `Err` is
    /// said to the model, and journaled.
    pub fn resolve(&self, places: &Places, asked: &str) -> Result<Inside<'_>, String> {
        let refused = || format!("refusé : {} est hors des dossiers autorisés", shown(asked));
        if asked.contains('\0') {
            return Err(refused());
        }
        let path = places.expand(asked);
        if !path.is_absolute() {
            return Err(format!(
                "refusé : {} n'est pas un chemin absolu",
                shown(asked)
            ));
        }
        let mut clean = PathBuf::new();
        for component in path.components() {
            match component {
                Component::RootDir => clean.push("/"),
                Component::CurDir => {}
                Component::Normal(name) => clean.push(name),
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(format!("refusé : « .. » dans {}", shown(asked)));
                }
            }
        }
        for folder in &self.list {
            for root in [&folder.real, &folder.shown] {
                if let Ok(relative) = clean.strip_prefix(root) {
                    return Ok(Inside {
                        folder,
                        relative: relative.to_owned(),
                    });
                }
            }
        }
        Err(refused())
    }
}

impl Inside<'_> {
    /// The path as the user knows it.
    pub fn shown(&self) -> PathBuf {
        self.folder.shown.join(&self.relative)
    }

    /// Opens the path with `flags`, never out of its folder.
    pub fn open(&self, flags: OFlags, mode: Mode) -> Result<OwnedFd, Errno> {
        let relative = if self.relative.as_os_str().is_empty() {
            Path::new(".")
        } else {
            &self.relative
        };
        beneath(&self.folder.fd, relative, flags, mode)
    }

    /// The folder that holds the path, opened, and the path's last name: what `*at` calls
    /// act on, without following a link in that name. The folder itself has none.
    pub fn parent(&self) -> Result<(OwnedFd, &OsStr), String> {
        let name = self
            .relative
            .file_name()
            .ok_or_else(|| String::from("refusé : c'est un dossier autorisé lui-même"))?;
        let parent = Inside {
            folder: self.folder,
            relative: self.relative.parent().unwrap_or(Path::new("")).to_owned(),
        };
        let fd = parent
            .open(OFlags::PATH | OFlags::DIRECTORY, Mode::empty())
            .map_err(|e| said(e, &parent.shown()))?;
        Ok((fd, name))
    }
}

/// Opens `relative` from `dir` without leaving it: `RESOLVE_BENEATH` refuses `..` above `dir`,
/// absolute paths and links, and links that climb out; magic links (`/proc/…/fd`) are refused.
pub fn beneath(
    dir: &OwnedFd,
    relative: &Path,
    flags: OFlags,
    mode: Mode,
) -> Result<OwnedFd, Errno> {
    // openat2 refuses with O_PATH any flag but these: no O_NOCTTY there.
    let flags = if flags.contains(OFlags::PATH) {
        flags | OFlags::CLOEXEC
    } else {
        flags | OFlags::CLOEXEC | OFlags::NOCTTY
    };
    let resolve = ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS;
    let mut tries = 0;
    loop {
        match rustix::fs::openat2(dir, relative, flags, mode, resolve) {
            // A rename elsewhere during the resolution: the kernel asks to try again.
            Err(Errno::AGAIN) if tries < 3 => tries += 1,
            other => return other,
        }
    }
}

/// What an error of the system means for the model.
pub fn said(error: Errno, path: &Path) -> String {
    let path = shown(&path.to_string_lossy());
    match error {
        Errno::XDEV => format!("refusé : un lien de {path} sort des dossiers autorisés"),
        Errno::LOOP => format!("refusé : {path} est un lien symbolique"),
        Errno::NOENT => format!("{path} n'existe pas"),
        Errno::EXIST => format!("{path} existe déjà"),
        Errno::NOTDIR => format!("{path} n'est pas un dossier"),
        Errno::ISDIR => format!("{path} est un dossier"),
        Errno::NOTEMPTY => format!("{path} n'est pas vide"),
        Errno::ACCESS | Errno::PERM => format!("{path} : accès refusé par le système"),
        Errno::NOSYS => String::from("refusé : le noyau ne sait pas confiner les chemins"),
        other => format!("{path} : erreur du système ({other})"),
    }
}

/// A path as given back to the model: one line, bounded.
fn shown(path: &str) -> String {
    crate::own_server::plain(path, 300)
}

fn read_settings(file: &Path) -> Option<Settings> {
    use std::io::Read as _;
    let opened = rustix::fs::openat(
        CWD,
        file,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .ok()?;
    let mut text = String::new();
    std::fs::File::from(opened)
        .take(MAX_SETTINGS)
        .read_to_string(&mut text)
        .ok()?;
    match serde_json::from_str(&text) {
        Ok(settings) => Some(settings),
        Err(error) => {
            // A broken file allows nothing rather than the defaults: the user wrote it.
            tracing::error!(%error, ?file, "files settings unreadable: no folder");
            Some(Settings {
                folders: Vec::new(),
                trash: None,
            })
        }
    }
}

fn open_folder(places: &Places, shown: &Path) -> Result<Folder, String> {
    if !shown.is_absolute() {
        return Err(String::from("pas un chemin absolu"));
    }
    let real = std::fs::canonicalize(shown).map_err(|_| String::from("introuvable"))?;
    for path in [shown, real.as_path()] {
        if let Some(why) = places.forbidden(path) {
            return Err(why);
        }
    }
    // The canonical path, which has no link left: opened as it was checked.
    let fd = rustix::fs::openat(
        CWD,
        &real,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| e.to_string())?;
    Ok(Folder {
        shown: shown.to_owned(),
        real,
        fd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            home: PathBuf::from("/home/camille"),
            config: PathBuf::from("/home/camille/.config"),
            data: PathBuf::from("/home/camille/.local/share"),
            state: PathBuf::from("/home/camille/.local/state"),
        }
    }

    #[test]
    fn places_of_secrets_are_never_folders() {
        let p = places();
        for path in [
            "/",
            "/home",
            "/home/camille",
            "/home/camille/.ssh",
            "/home/camille/.ssh/keys",
            "/home/camille/.gnupg",
            "/home/camille/.local",
            "/home/camille/.local/share",
            "/home/camille/.local/share/keyrings",
            "/home/camille/.config",
            "/home/camille/.config/sherlock",
            "/home/camille/.config/sherlock-files",
            "/home/camille/.local/state/sherlock",
            "/home/camille/.claude",
        ] {
            assert!(p.forbidden(Path::new(path)).is_some(), "{path}");
        }
        for path in [
            "/home/camille/Documents",
            "/home/camille/.config/autre",
            "/srv/partage",
        ] {
            assert!(p.forbidden(Path::new(path)).is_none(), "{path}");
        }
        assert_eq!(p.expand("~/Bureau"), Path::new("/home/camille/Bureau"));
        assert_eq!(p.expand("~camille"), Path::new("~camille"));
    }
}
