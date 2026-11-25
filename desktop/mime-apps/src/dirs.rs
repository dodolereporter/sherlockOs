//! XDG base directories (XDG Base Directory Specification 0.8).

use std::path::{Path, PathBuf};

/// The XDG directories, most important first within each list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dirs {
    /// `$XDG_CONFIG_HOME`, `~/.config` by default: where `mimeapps.list` is written.
    pub config_home: PathBuf,
    /// `$XDG_CONFIG_DIRS`, `/etc/xdg` by default.
    pub config_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_HOME`, `~/.local/share` by default.
    pub data_home: PathBuf,
    /// `$XDG_DATA_DIRS`, `/usr/local/share:/usr/share` by default.
    pub data_dirs: Vec<PathBuf>,
}

/// An absolute path from a variable, or none: the specification ignores relative paths.
fn absolute(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

fn list(value: Option<std::ffi::OsString>, default: &[&str]) -> Vec<PathBuf> {
    let parsed: Vec<PathBuf> = value
        .map(|v| {
            std::env::split_paths(&v)
                .filter(|p| p.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    if parsed.is_empty() {
        default.iter().map(PathBuf::from).collect()
    } else {
        parsed
    }
}

impl Dirs {
    /// From the environment, with the specification's defaults under `home`.
    pub fn from_env(home: &Path) -> Self {
        let var = std::env::var_os;
        Self {
            config_home: absolute(var("XDG_CONFIG_HOME")).unwrap_or_else(|| home.join(".config")),
            config_dirs: list(var("XDG_CONFIG_DIRS"), &["/etc/xdg"]),
            data_home: absolute(var("XDG_DATA_HOME")).unwrap_or_else(|| home.join(".local/share")),
            data_dirs: list(var("XDG_DATA_DIRS"), &["/usr/local/share", "/usr/share"]),
        }
    }

    /// Data directories, the user's first.
    pub fn data(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.data_home).chain(&self.data_dirs)
    }

    /// Configuration directories, the user's first.
    pub fn config(&self) -> impl Iterator<Item = &PathBuf> {
        std::iter::once(&self.config_home).chain(&self.config_dirs)
    }
}
