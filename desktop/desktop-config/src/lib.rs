//! Settings of the desktop that several programs share: the compositor applies them, Control
//! Room (lot D-26a) writes them: the wallpaper (see "Wallpapers" in `docs/compositor.md`),
//! the touchpad (see "Touchpad" there), the pointer (see "Show the clicks" in
//! `docs/cursor.md`), the lock (see "Session lock" in `docs/compositor.md`) and the screens
//! (ADR 0025, see "Screens" in `docs/compositor.md`).

use std::{ffi::OsString, path::PathBuf};

pub mod conf;
pub mod displays;
pub mod lock;
pub mod pointer;
pub mod touchpad;
pub mod wallpaper;

#[cfg(feature = "variants")]
pub mod variants;

/// Folder of the desktop in the XDG directories: `$XDG_CONFIG_HOME/<DIR>`,
/// `$XDG_DATA_DIRS/<DIR>`: the name of the OS in paths (ADR 0022).
pub const DIR: &str = "sherlockos";

/// `$XDG_CONFIG_HOME/<DIR>`, or `~/.config/<DIR>`. `var` reads the environment.
pub fn config_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = absolute(var("XDG_CONFIG_HOME"))
        .or_else(|| absolute(var("HOME")).map(|home| home.join(".config")))?;
    Some(home.join(DIR))
}

/// `<DIR>` in `$XDG_DATA_HOME` (`~/.local/share`), then in each of `$XDG_DATA_DIRS`
/// (`/usr/local/share:/usr/share`), in order of precedence.
pub fn data_dirs(var: impl Fn(&str) -> Option<OsString>) -> Vec<PathBuf> {
    let home = absolute(var("XDG_DATA_HOME"))
        .or_else(|| absolute(var("HOME")).map(|home| home.join(".local/share")));
    let system = var("XDG_DATA_DIRS").filter(|dirs| !dirs.is_empty());
    let system: Vec<PathBuf> = match system {
        Some(dirs) => std::env::split_paths(&dirs)
            .filter(|p| p.is_absolute())
            .collect(),
        None => vec!["/usr/local/share".into(), "/usr/share".into()],
    };
    home.into_iter()
        .chain(system)
        .map(|dir| dir.join(DIR))
        .collect()
}

/// The XDG specification ignores relative paths.
fn absolute(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&str, &str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |key| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v.into())
    }

    #[test]
    fn the_directories_follow_xdg() {
        let home = [("HOME", "/home/u")];
        assert_eq!(
            config_dir(env(&home)),
            Some("/home/u/.config/sherlockos".into())
        );
        assert_eq!(
            data_dirs(env(&home)),
            [
                PathBuf::from("/home/u/.local/share/sherlockos"),
                "/usr/local/share/sherlockos".into(),
                "/usr/share/sherlockos".into(),
            ]
        );
        let set = [
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("XDG_DATA_HOME", "relative"),
            ("XDG_DATA_DIRS", "/a:relative:/b"),
        ];
        assert_eq!(config_dir(env(&set)), Some("/cfg/sherlockos".into()));
        assert_eq!(
            data_dirs(env(&set)),
            [
                PathBuf::from("/home/u/.local/share/sherlockos"),
                "/a/sherlockos".into(),
                "/b/sherlockos".into(),
            ]
        );
        assert_eq!(config_dir(env(&[])), None);
    }
}
