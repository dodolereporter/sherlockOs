//! Installing the generated files in the user's folders, and taking them away. Nothing is
//! overwritten blindly: a file that was there first is renamed `<name>.bak-<date>` and
//! comes back on removal; a file of ours that the user changed since is kept the same way. A
//! manifest lists what was installed and the backup of each file.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use sherlock_ui::color::Theme;

use crate::gtk::{self, Gtk};
use crate::qt::{self, QT5_ROLES, QT6_ROLES};

/// What can go wrong.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },
    #[error("neither XDG_CONFIG_HOME nor HOME is set")]
    NoHome,
    #[error("manifest line {0} cannot be read")]
    Manifest(usize),
    #[error("path with a tab or a line break: {}", .0.display())]
    Path(PathBuf),
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_owned(),
        source,
    }
}

/// The two settings tools of Qt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qt {
    V5,
    V6,
}

impl Qt {
    pub const ALL: [Self; 2] = [Self::V5, Self::V6];

    pub fn tool(self) -> &'static str {
        match self {
            Self::V5 => "qt5ct",
            Self::V6 => "qt6ct",
        }
    }

    fn roles(self) -> usize {
        match self {
            Self::V5 => QT5_ROLES,
            Self::V6 => QT6_ROLES,
        }
    }
}

/// The user's folders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub home: PathBuf,
    /// `$XDG_CONFIG_HOME`.
    pub config: PathBuf,
    /// `$XDG_DATA_HOME`.
    pub data: PathBuf,
}

impl Layout {
    /// From `HOME`, `XDG_CONFIG_HOME` and `XDG_DATA_HOME`.
    pub fn from_env() -> Result<Self, Error> {
        let var = |name| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var("HOME").ok_or(Error::NoHome)?;
        Ok(Self {
            config: var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config")),
            data: var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share")),
            home,
        })
    }

    /// Our own folder: palettes, the dconf profile of the session, the manifest, the program.
    pub fn themes(&self) -> PathBuf {
        self.data.join("nom-os/themes")
    }

    pub fn manifest(&self) -> PathBuf {
        self.themes().join("manifest")
    }

    /// The dconf profile that the session script gives to `DCONF_PROFILE`.
    pub fn dconf_profile(&self) -> PathBuf {
        self.themes().join("dconf-profile")
    }

    /// The program itself, which the session runs to follow the theme.
    pub fn program(&self) -> PathBuf {
        self.themes().join("toolkit-theme")
    }

    pub fn qtct_palette(&self, qt: Qt, theme: Theme) -> PathBuf {
        self.themes()
            .join(qt.tool())
            .join(format!("{}.conf", crate::variant(theme)))
    }

    pub fn qtct_settings(&self, qt: Qt) -> PathBuf {
        self.config
            .join(qt.tool())
            .join(format!("{}.conf", qt.tool()))
    }

    /// Where adw-gtk3 may be, first found first: the user's themes, then the system's.
    fn theme_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.data.join("themes"), self.home.join(".themes")];
        let system = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
        dirs.extend(system.split(':').map(|d| Path::new(d).join("themes")));
        dirs
    }

    /// The `gtk.css` of adw-gtk3 for `theme`, as a `file://` URL, when it is installed.
    pub fn adw_gtk3(&self, theme: Theme) -> Option<String> {
        let name = match theme {
            Theme::Dark => "adw-gtk3-dark",
            Theme::Light => "adw-gtk3",
        };
        let file = self
            .theme_dirs()
            .into_iter()
            .map(|d| d.join(name).join("gtk-3.0/gtk.css"))
            .find(|f| f.is_file())?;
        let text = file.to_str()?;
        // Written as is in `url("…")`: anything that would need escaping is left out.
        let plain = text
            .bytes()
            .all(|b| b.is_ascii_graphic() && !b"\"'()\\%".contains(&b));
        plain.then(|| format!("file://{text}"))
    }
}

/// A file to install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: PathBuf,
    pub content: Vec<u8>,
    pub executable: bool,
}

impl File {
    fn text(path: PathBuf, content: String) -> Self {
        Self {
            path,
            content: content.into_bytes(),
            executable: false,
        }
    }
}

/// Every generated file, at its place in `layout`. The settings of qt5ct and qt6ct name the
/// dark palette: the session switches it to the theme of the moment (`apply`).
pub fn files(layout: &Layout) -> Vec<File> {
    let mut files = Vec::new();
    for theme in Theme::ALL {
        let dir = layout
            .data
            .join("themes")
            .join(crate::gtk_theme_name(theme));
        let base3 = layout
            .adw_gtk3(theme)
            .unwrap_or_else(|| gtk::builtin_base(Gtk::V3, theme).to_owned());
        let base4 = gtk::builtin_base(Gtk::V4, theme);
        files.push(File::text(dir.join("index.theme"), gtk::index(theme)));
        files.push(File::text(
            dir.join("gtk-3.0/gtk.css"),
            gtk::stylesheet(&base3, theme),
        ));
        files.push(File::text(
            dir.join("gtk-4.0/gtk.css"),
            gtk::stylesheet(base4, theme),
        ));
        let colors = layout
            .data
            .join("color-schemes")
            .join(qt::kde_file_name(theme));
        files.push(File::text(colors, qt::kde_colors(theme)));
        let libadwaita = layout
            .themes()
            .join(format!("libadwaita/{}.css", crate::variant(theme)));
        files.push(File::text(libadwaita, gtk::libadwaita(theme)));
        for tool in Qt::ALL {
            let palette = qt::qtct_palette(theme, tool.roles());
            files.push(File::text(layout.qtct_palette(tool, theme), palette));
        }
    }
    for tool in Qt::ALL {
        let settings = qt::qtct_settings(&layout.qtct_palette(tool, Theme::Dark));
        files.push(File::text(layout.qtct_settings(tool), settings));
    }
    // The session's own database first, written to; the user's after it, read only: the
    // theme set in the session never reaches the user's database, so neither Plasma.
    let profile = "user-db:nom_os\nuser-db:user\n".to_owned();
    files.push(File::text(layout.dconf_profile(), profile));
    files
}

/// What `install` and `uninstall` did, for the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Wrote(PathBuf),
    Unchanged(PathBuf),
    /// A file that was there, renamed before ours took its place.
    BackedUp {
        path: PathBuf,
        backup: PathBuf,
    },
    Removed(PathBuf),
    Restored {
        path: PathBuf,
        backup: PathBuf,
    },
    /// The backup listed in the manifest is gone: nothing to put back.
    BackupMissing(PathBuf),
}

/// An installed file: its backup, and the fingerprint of what was written.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    backup: Option<PathBuf>,
    fingerprint: u64,
}

/// FNV-1a of the content, the palette line of qt5ct and qt6ct left out: the session switches
/// it, and the file stays ours.
fn fingerprint(content: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for line in content.split_inclusive(|&b| b == b'\n') {
        if line.starts_with(qt::SCHEME_KEY.as_bytes()) {
            continue;
        }
        for &b in line {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

fn read_manifest(path: &Path) -> Result<BTreeMap<PathBuf, Entry>, Error> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(io_error(path)(e)),
    };
    let mut entries = BTreeMap::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with('#'))
    {
        let mut fields = line.split('\t');
        let (Some(file), Some(backup), Some(hash), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(Error::Manifest(n + 1));
        };
        let fingerprint = u64::from_str_radix(hash, 16).map_err(|_| Error::Manifest(n + 1))?;
        let backup = (backup != "-").then(|| PathBuf::from(backup));
        entries.insert(
            PathBuf::from(file),
            Entry {
                backup,
                fingerprint,
            },
        );
    }
    Ok(entries)
}

fn write_manifest(path: &Path, entries: &BTreeMap<PathBuf, Entry>) -> Result<(), Error> {
    let mut text =
        String::from("# toolkit-theme: installed file, backup (- for none), fingerprint\n");
    for (file, entry) in entries {
        let backup = entry
            .backup
            .as_deref()
            .map_or(String::from("-"), |b| b.display().to_string());
        text.push_str(&format!(
            "{}\t{backup}\t{:016x}\n",
            file.display(),
            entry.fingerprint
        ));
    }
    write_atomic(path, text.as_bytes(), false)
}

/// Writes through a temporary file renamed over the target: a reader never sees half a file,
/// and qt5ct and qt6ct, which watch their folder, see the change.
pub(crate) fn write_atomic(path: &Path, content: &[u8], executable: bool) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_error(dir))?;
    }
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".toolkit-theme-new");
    let temporary = path.with_file_name(name);
    fs::write(&temporary, content).map_err(io_error(&temporary))?;
    let mode = if executable { 0o755 } else { 0o644 };
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
        .map_err(io_error(&temporary))?;
    fs::rename(&temporary, path).map_err(io_error(path))
}

/// Renames `path` to `<path>.bak-<stamp>`, with `-2`, `-3`… if that name is taken.
fn back_up(path: &Path, stamp: &str) -> Result<PathBuf, Error> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut backup = path.with_file_name(format!("{name}.bak-{stamp}"));
    let mut n = 2;
    while backup.symlink_metadata().is_ok() {
        backup = path.with_file_name(format!("{name}.bak-{stamp}-{n}"));
        n += 1;
    }
    fs::rename(path, &backup).map_err(io_error(path))?;
    Ok(backup)
}

/// Date and time of the backup names, `YYYYMMDD-HHMMSS` in local time.
pub fn stamp() -> String {
    match appearance_config::local(appearance_config::epoch_now()) {
        Ok(tm) => format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        ),
        Err(_) => appearance_config::epoch_now().to_string(),
    }
}

/// Installs `files`. A file that is there and not ours is backed up first, and so is a file
/// of ours that was changed since. Installing again keeps the first backups.
pub fn install(layout: &Layout, files: &[File], stamp: &str) -> Result<Vec<Action>, Error> {
    let manifest_path = layout.manifest();
    let mut manifest = read_manifest(&manifest_path)?;
    let mut actions = Vec::new();
    for file in files {
        let path = &file.path;
        if path.to_string_lossy().contains(['\t', '\n']) {
            return Err(Error::Path(path.clone()));
        }
        let current = match fs::read(path) {
            Ok(current) => Some(current),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_error(path)(e)),
        };
        let known = manifest.get(path).cloned();
        let mut backup = known.as_ref().and_then(|k| k.backup.clone());
        match (&current, &known) {
            (Some(current), Some(k)) if fingerprint(current) == k.fingerprint => {
                // The same but for the palette line: the session's palette of the moment stays.
                if fingerprint(current) == fingerprint(&file.content) {
                    actions.push(Action::Unchanged(path.clone()));
                    continue;
                }
            }
            (Some(_), Some(_)) => {
                let kept = back_up(path, stamp)?;
                actions.push(Action::BackedUp {
                    path: path.clone(),
                    backup: kept,
                });
            }
            (Some(_), None) => {
                let kept = back_up(path, stamp)?;
                actions.push(Action::BackedUp {
                    path: path.clone(),
                    backup: kept.clone(),
                });
                backup = Some(kept);
            }
            (None, _) => {}
        }
        write_atomic(path, &file.content, file.executable)?;
        actions.push(Action::Wrote(path.clone()));
        manifest.insert(
            path.clone(),
            Entry {
                backup,
                fingerprint: fingerprint(&file.content),
            },
        );
        // Written after each file: an error halfway leaves a manifest that `uninstall` reads.
        write_manifest(&manifest_path, &manifest)?;
    }
    Ok(actions)
}

/// Takes away every installed file and puts the backups back. A file the user changed since
/// it was installed is kept as a dated backup instead of being deleted.
pub fn uninstall(layout: &Layout, stamp: &str) -> Result<Vec<Action>, Error> {
    let manifest_path = layout.manifest();
    let manifest = read_manifest(&manifest_path)?;
    let mut actions = Vec::new();
    for (path, entry) in manifest.iter().rev() {
        match fs::read(path) {
            Ok(current) if fingerprint(&current) != entry.fingerprint => {
                let kept = back_up(path, stamp)?;
                actions.push(Action::BackedUp {
                    path: path.clone(),
                    backup: kept,
                });
            }
            Ok(_) => {
                fs::remove_file(path).map_err(io_error(path))?;
                actions.push(Action::Removed(path.clone()));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(path)(e)),
        }
        if let Some(backup) = &entry.backup {
            if backup.symlink_metadata().is_ok() {
                fs::rename(backup, path).map_err(io_error(backup))?;
                actions.push(Action::Restored {
                    path: path.clone(),
                    backup: backup.clone(),
                });
            } else {
                actions.push(Action::BackupMissing(backup.clone()));
            }
        }
    }
    fs::remove_file(&manifest_path).or_else(|e| match e.kind() {
        io::ErrorKind::NotFound => Ok(()),
        _ => Err(io_error(&manifest_path)(e)),
    })?;
    // Our folders go too once empty; `remove_dir` leaves one that still holds something.
    let mut ours: Vec<PathBuf> = Theme::ALL
        .iter()
        .map(|&t| layout.data.join("themes").join(crate::gtk_theme_name(t)))
        .collect();
    ours.push(layout.data.join("nom-os"));
    for path in manifest.keys().chain([&manifest_path]) {
        let mut dir = path.parent();
        while let Some(d) = dir.filter(|d| ours.iter().any(|o| d.starts_with(o))) {
            if fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
    Ok(actions)
}

/// Whether `path` was installed and is listed in the manifest.
pub fn is_installed(layout: &Layout, path: &Path) -> bool {
    read_manifest(&layout.manifest()).is_ok_and(|m| m.contains_key(path))
}
