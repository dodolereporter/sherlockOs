//! Installed applications, file types and default applications, after the freedesktop
//! specifications: Desktop Entry, Shared MIME-info (its `globs2` and `subclasses` files) and
//! MIME Applications Associations (`mimeapps.list`).
//!
//! Evidence uses it for "Open with" and default applications; the "Default applications"
//! page of Control Room (D-26) will reuse it. It reads and writes plain files only: no
//! D-Bus, no `xdg-mime` process. See `docs/mime-apps.md`.

pub mod dirs;
pub mod entry;
pub mod list;
pub mod mime;

pub use dirs::Dirs;
pub use entry::{DesktopEntry, read_applications};
pub use list::{MimeApps, set_default};
pub use mime::MimeDb;

/// Everything needed to answer "which applications open this file": read once, off any
/// interface thread, then queried in memory.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub dirs: Dirs,
    pub mime: MimeDb,
    pub applications: Vec<DesktopEntry>,
    pub associations: MimeApps,
}

/// Adds an application unless it is already there, by id or by name and command.
fn push<'a>(out: &mut Vec<&'a DesktopEntry>, app: &'a DesktopEntry) {
    let duplicate = out
        .iter()
        .any(|known| known.id == app.id || (known.name == app.name && known.exec == app.exec));
    if !duplicate {
        out.push(app);
    }
}

impl Catalog {
    /// Reads the applications, the MIME database and the associations under `dirs`.
    /// `desktops` are the current desktop names (`XDG_CURRENT_DESKTOP`, lower case), for the
    /// desktop-specific `<desktop>-mimeapps.list` files; `locales` the locale names to try for
    /// the application names, most specific first.
    pub fn load(dirs: Dirs, desktops: &[String], locales: &[String]) -> Self {
        let applications = read_applications(&dirs, locales);
        let mime = MimeDb::load(&dirs);
        let associations = MimeApps::load(&dirs, desktops);
        Self {
            dirs,
            mime,
            applications,
            associations,
        }
    }

    /// The installed application with this desktop id.
    pub fn application(&self, id: &str) -> Option<&DesktopEntry> {
        self.applications.iter().find(|app| app.id == id)
    }

    /// The default application for a MIME type: the first one the associations name that is
    /// installed, else the first application that declares the type itself, else one that
    /// declares a parent type.
    pub fn default_for(&self, mime: &str) -> Option<&DesktopEntry> {
        self.handlers(mime).into_iter().next()
    }

    /// The applications that can open a MIME type, the default first, without duplicates:
    /// the associations' choices, then the applications declaring the type, then those
    /// declaring one of its parents (`text/plain` for source code, for example). Removed
    /// associations and hidden entries are left out.
    pub fn handlers(&self, mime: &str) -> Vec<&DesktopEntry> {
        let mut out: Vec<&DesktopEntry> = Vec::new();
        let types = self.mime.ancestors(mime);
        for kind in &types {
            for id in self.associations.defaults(kind) {
                if let Some(app) = self.application(&id)
                    && !self.associations.is_removed(kind, &id)
                {
                    push(&mut out, app);
                }
            }
        }
        for kind in &types {
            for id in self.associations.added(kind) {
                if let Some(app) = self.application(&id) {
                    push(&mut out, app);
                }
            }
            for app in &self.applications {
                if app.mime_types.iter().any(|m| m == kind)
                    && !self.associations.is_removed(kind, &app.id)
                {
                    push(&mut out, app);
                }
            }
        }
        out
    }

    /// The applications a user can pick from in a "choose an application" list: every one
    /// that is not hidden from menus, by name.
    pub fn choosable(&self) -> Vec<&DesktopEntry> {
        let mut apps: Vec<&DesktopEntry> = self
            .applications
            .iter()
            .filter(|app| !app.no_display)
            .collect();
        apps.sort_by_key(|app| app.name.to_lowercase());
        apps
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A temporary folder holding fake XDG directories, removed at the end of the test.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(name: &str) -> Self {
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mime-apps-{name}-{}-{n}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temporary folder");
            Self(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }

        pub fn write(&self, relative: &str, content: &str) {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("folder");
            }
            std::fs::write(path, content).expect("write");
        }

        /// Fake XDG directories under this folder.
        pub fn dirs(&self) -> crate::Dirs {
            crate::Dirs {
                config_home: self.0.join("config"),
                config_dirs: vec![self.0.join("etc/xdg")],
                data_home: self.0.join("data"),
                data_dirs: vec![self.0.join("usr/share")],
            }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    fn fixture() -> TempDir {
        let tmp = TempDir::new("catalog");
        tmp.write(
            "usr/share/mime/globs2",
            "# comment\n50:text/rust:*.rs\n50:text/plain:*.txt\n50:image/png:*.png\n",
        );
        tmp.write("usr/share/mime/subclasses", "text/rust text/plain\n");
        tmp.write(
            "usr/share/applications/org.kde.kate.desktop",
            "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate -b %U\nIcon=kate\nMimeType=text/plain;\n",
        );
        tmp.write(
            "usr/share/applications/code.desktop",
            "[Desktop Entry]\nType=Application\nName=Visual Studio Code\nExec=code %F\nMimeType=text/plain;inode/directory;\n",
        );
        tmp.write(
            "usr/share/applications/rust-viewer.desktop",
            "[Desktop Entry]\nType=Application\nName=Rust Viewer\nExec=rv %f\nMimeType=text/rust;\nNoDisplay=true\n",
        );
        tmp.write(
            "usr/share/applications/gimp.desktop",
            "[Desktop Entry]\nType=Application\nName=GIMP\nExec=gimp %U\nMimeType=image/png;\n",
        );
        tmp
    }

    #[test]
    fn handlers_follow_type_then_parents_with_the_default_first() {
        let tmp = fixture();
        tmp.write(
            "config/mimeapps.list",
            "[Default Applications]\ntext/plain=code.desktop;\n",
        );
        let catalog = Catalog::load(tmp.dirs(), &[], &[]);
        let ids: Vec<&str> = catalog
            .handlers("text/rust")
            .iter()
            .map(|app| app.id.as_str())
            .collect();
        // The default of the parent type comes first, then the exact type, then the rest.
        assert_eq!(
            ids,
            [
                "code.desktop",
                "rust-viewer.desktop",
                "org.kde.kate.desktop"
            ]
        );
        assert_eq!(
            catalog.default_for("image/png").map(|a| a.id.as_str()),
            Some("gimp.desktop")
        );
        let choosable: Vec<&str> = catalog
            .choosable()
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(choosable, ["GIMP", "Kate", "Visual Studio Code"]);
    }

    #[test]
    fn a_chosen_default_is_used_afterwards() {
        let tmp = fixture();
        set_default(&tmp.dirs(), "text/rust", "org.kde.kate.desktop").expect("written");
        let catalog = Catalog::load(tmp.dirs(), &[], &[]);
        assert_eq!(
            catalog.default_for("text/rust").map(|a| a.id.as_str()),
            Some("org.kde.kate.desktop")
        );
    }
}
