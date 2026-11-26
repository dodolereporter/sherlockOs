//! Switching the session to a theme: the palette of qt5ct and qt6ct, which their applications
//! read again at once, and the GTK theme and `color-scheme` in the session's dconf database.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;

use sherlock_ui::color::Theme;

use crate::install::{self, Error, Layout, Qt};

/// Points the installed settings of qt5ct and qt6ct at the palette of `theme`. A settings file
/// that `install` did not write is left alone. Returns the files changed.
pub fn qt(layout: &Layout, theme: Theme) -> Result<Vec<std::path::PathBuf>, Error> {
    let mut changed = Vec::new();
    for tool in Qt::ALL {
        let path = layout.qtct_settings(tool);
        if !install::is_installed(layout, &path) {
            continue;
        }
        let current = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(Error::Io { path, source }),
        };
        if let Some(text) = crate::qt::with_palette(&current, &layout.qtct_palette(tool, theme)) {
            install::write_atomic(&path, text.as_bytes(), false)?;
            changed.push(path);
        }
    }
    Ok(changed)
}

/// Whether this process reads the session's dconf database: `DCONF_PROFILE` names the profile
/// that `install` wrote. Only then may `gsettings` write, so that a theme set by hand outside
/// the session never lands in the user's database, the one Plasma reads.
pub fn in_session(layout: &Layout, profile: Option<&OsStr>) -> bool {
    profile.is_some_and(|p| Path::new(p) == layout.dconf_profile())
}

/// The keys of `org.gnome.desktop.interface` that follow the theme: the GTK theme, read again
/// at once by GTK 3 and GTK 4, and the preference libadwaita follows.
pub fn gsettings_keys(theme: Theme) -> [(&'static str, &'static str); 2] {
    let scheme = match theme {
        Theme::Dark => "prefer-dark",
        Theme::Light => "default",
    };
    [
        ("gtk-theme", crate::gtk_theme_name(theme)),
        ("color-scheme", scheme),
    ]
}

/// Sets the keys with `gsettings`, in the session's database only (`in_session`).
pub fn gsettings(layout: &Layout, theme: Theme) -> io::Result<()> {
    if !in_session(layout, std::env::var_os("DCONF_PROFILE").as_deref()) {
        return Err(io::Error::other(format!(
            "DCONF_PROFILE is not {}: outside the NOM_OS session, the GTK settings are left alone",
            layout.dconf_profile().display()
        )));
    }
    for (key, value) in gsettings_keys(theme) {
        let status = Command::new("gsettings")
            .args(["set", "org.gnome.desktop.interface", key, value])
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "gsettings set {key} {value}: {status}"
            )));
        }
    }
    Ok(())
}

/// Both, each logging what goes wrong without stopping the other.
pub fn apply(layout: &Layout, theme: Theme) {
    match qt(layout, theme) {
        Ok(changed) => {
            for path in changed {
                tracing::info!(path = %path.display(), ?theme, "Qt palette switched");
            }
        }
        Err(error) => tracing::warn!(%error, "cannot switch the Qt palette"),
    }
    match gsettings(layout, theme) {
        Ok(()) => tracing::info!(?theme, "GTK theme and color scheme set"),
        Err(error) => tracing::warn!(%error, "cannot set the GTK theme"),
    }
}
