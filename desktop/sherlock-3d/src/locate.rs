//! Where the model file lives (ADR 0024).
//!
//! `sherlock.glb` is not in this repository: it is downloaded by `just mascot` from the public
//! repository `dodolereporter/sherlock`, at a pinned revision checked by its SHA-256.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Environment variable that names the model file, ahead of every folder.
pub const VARIABLE: &str = "SHERLOCKOS_MASCOT";

/// Path of the model under a data folder.
pub const RELATIVE: &str = "sherlockos/mascot/sherlock.glb";

/// The places the model is looked for, in order:
///
/// 1. the file named by `SHERLOCKOS_MASCOT`;
/// 2. `$XDG_DATA_HOME/sherlockos/mascot/sherlock.glb`;
/// 3. `~/.local/share/sherlockos/mascot/sherlock.glb`;
/// 4. the same under every folder of `$XDG_DATA_DIRS` (`/usr/local/share:/usr/share` when
///    unset), where the system installs it.
pub fn candidates() -> Vec<PathBuf> {
    candidates_from(&|name| std::env::var_os(name))
}

/// The first candidate that is a file.
pub fn find() -> Option<PathBuf> {
    candidates().into_iter().find(|path| path.is_file())
}

/// `candidates`, reading the environment through `var`.
pub fn candidates_from(var: &dyn Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let set = |name: &str| var(name).filter(|value| !value.is_empty());
    let mut found = Vec::new();
    if let Some(file) = set(VARIABLE) {
        found.push(PathBuf::from(file));
    }
    // The XDG specification ignores relative paths.
    if let Some(data) = set("XDG_DATA_HOME").map(PathBuf::from)
        && data.is_absolute()
    {
        found.push(data.join(RELATIVE));
    }
    if let Some(home) = set("HOME").map(PathBuf::from) {
        let path = home.join(".local/share").join(RELATIVE);
        if !found.contains(&path) {
            found.push(path);
        }
    }
    let dirs = set("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for dir in std::env::split_paths(&dirs) {
        if dir.is_absolute() {
            found.push(Path::new(&dir).join(RELATIVE));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn the_variable_comes_first_then_the_user_then_the_system() {
        let vars = env(&[
            (VARIABLE, "/tmp/sherlock.glb"),
            ("XDG_DATA_HOME", "/data"),
            ("HOME", "/home/sam"),
            ("XDG_DATA_DIRS", "/usr/share"),
        ]);
        assert_eq!(
            candidates_from(&vars),
            [
                "/tmp/sherlock.glb",
                "/data/sherlockos/mascot/sherlock.glb",
                "/home/sam/.local/share/sherlockos/mascot/sherlock.glb",
                "/usr/share/sherlockos/mascot/sherlock.glb",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn defaults_without_variables() {
        let vars = env(&[("HOME", "/home/sam"), ("XDG_DATA_HOME", "relative")]);
        assert_eq!(
            candidates_from(&vars),
            [
                "/home/sam/.local/share/sherlockos/mascot/sherlock.glb",
                "/usr/local/share/sherlockos/mascot/sherlock.glb",
                "/usr/share/sherlockos/mascot/sherlock.glb",
            ]
            .map(PathBuf::from)
        );
    }
}
