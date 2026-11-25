//! Default and associated applications: `mimeapps.list` (MIME Applications Associations 1.0.1).
//!
//! Reading merges every `mimeapps.list` in the specification's order. Writing touches only
//! the user's `$XDG_CONFIG_HOME/mimeapps.list`, line by line: other sections, other
//! entries, comments and blank lines stay as they were, and the new file replaces the old
//! one atomically (written beside it, then renamed).

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::Dirs;

const DEFAULTS: &str = "Default Applications";
const ADDED: &str = "Added Associations";
const REMOVED: &str = "Removed Associations";

/// The associations, merged from every `mimeapps.list`, most important first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MimeApps {
    defaults: HashMap<String, Vec<String>>,
    added: HashMap<String, Vec<String>>,
    removed: HashMap<String, Vec<String>>,
}

/// The `mimeapps.list` files, most important first: for each configuration directory then
/// each data directory's `applications/`, the desktop-specific file before the generic one.
pub fn files(dirs: &Dirs, desktops: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let roots = dirs
        .config()
        .cloned()
        .chain(dirs.data().map(|d| d.join("applications")));
    for root in roots {
        for desktop in desktops {
            out.push(root.join(format!("{desktop}-mimeapps.list")));
        }
        out.push(root.join("mimeapps.list"));
    }
    out
}

/// Reads `[group]` and `key=a;b;` lines.
fn parse(text: &str) -> Vec<(String, String, Vec<String>)> {
    let mut out = Vec::new();
    let mut group = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name.to_owned();
        } else if !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            let ids = value
                .split(';')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
                .collect();
            out.push((group.clone(), key.trim().to_owned(), ids));
        }
    }
    out
}

impl MimeApps {
    /// Reads and merges the associations.
    pub fn load(dirs: &Dirs, desktops: &[String]) -> Self {
        let mut apps = Self::default();
        for file in files(dirs, desktops) {
            if let Ok(text) = std::fs::read_to_string(&file) {
                apps.merge(&text);
            }
        }
        apps
    }

    /// Adds a less important file: its entries come after those already read.
    pub fn merge(&mut self, text: &str) {
        for (group, mime, ids) in parse(text) {
            let target = match group.as_str() {
                DEFAULTS => &mut self.defaults,
                ADDED => &mut self.added,
                REMOVED => &mut self.removed,
                _ => continue,
            };
            let list = target.entry(mime).or_default();
            for id in ids {
                if !list.contains(&id) {
                    list.push(id);
                }
            }
        }
    }

    /// The default applications chosen for a type, in order.
    pub fn defaults(&self, mime: &str) -> Vec<String> {
        self.defaults.get(mime).cloned().unwrap_or_default()
    }

    /// The applications added by hand for a type.
    pub fn added(&self, mime: &str) -> Vec<String> {
        self.added.get(mime).cloned().unwrap_or_default()
    }

    /// The user removed this association.
    pub fn is_removed(&self, mime: &str, id: &str) -> bool {
        self.removed
            .get(mime)
            .is_some_and(|ids| ids.iter().any(|i| i == id))
    }
}

/// Makes `app` the default application for `mime` in `$XDG_CONFIG_HOME/mimeapps.list`, and
/// adds the association. The application goes first in both lists; the others stay after it.
pub fn set_default(dirs: &Dirs, mime: &str, app: &str) -> io::Result<()> {
    let path = dirs.config_home.join("mimeapps.list");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let text = set_in(&text, DEFAULTS, mime, app);
    let text = set_in(&text, ADDED, mime, app);
    write_atomically(&path, &text)
}

/// Puts `app` first in `group`'s entry for `mime`, creating the entry or the group.
fn set_in(text: &str, group: &str, mime: &str, app: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let header = format!("[{group}]");
    let start = lines.iter().position(|l| l.trim() == header);
    let Some(start) = start else {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(format!("{mime}={app};"));
        return lines.join("\n") + "\n";
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map_or(lines.len(), |i| start + 1 + i);
    let existing = (start + 1..end).find(|&i| {
        lines[i]
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == mime)
    });
    match existing {
        Some(i) => {
            let (_, value) = lines[i].split_once('=').unwrap_or_default();
            let others = value
                .split(';')
                .map(str::trim)
                .filter(|v| !v.is_empty() && *v != app);
            let ids: Vec<&str> = std::iter::once(app).chain(others).collect();
            lines[i] = format!("{mime}={};", ids.join(";"));
        }
        None => {
            // After the last entry of the group, before trailing blank lines.
            let mut at = end;
            while at > start + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, format!("{mime}={app};"));
        }
    }
    lines.join("\n") + "\n"
}

/// Writes a file beside the target, flushes it to disk, then renames it over the target: a
/// reader sees the old file or the new one, never half of one.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    let dir = path.parent().ok_or(io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join(format!(".mimeapps.list.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Ok(meta) = std::fs::metadata(path) {
            file.set_permissions(meta.permissions())?;
        }
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    const EXISTING: &str = "# My associations\n[Added Associations]\nimage/png=gimp.desktop;\n\n\
                            [Default Applications]\ntext/plain=org.kde.kate.desktop;code.desktop;\n\
                            x-scheme-handler/https=firefox.desktop;\n\n[Removed Associations]\n\
                            text/html=code.desktop;\n";

    #[test]
    fn setting_a_default_keeps_everything_else() {
        let tmp = TempDir::new("set");
        tmp.write("config/mimeapps.list", EXISTING);
        set_default(&tmp.dirs(), "text/plain", "code.desktop").expect("written");
        set_default(&tmp.dirs(), "text/rust", "org.kde.kate.desktop").expect("written");
        let text = std::fs::read_to_string(tmp.path().join("config/mimeapps.list")).expect("read");
        assert_eq!(
            text,
            "# My associations\n[Added Associations]\nimage/png=gimp.desktop;\n\
             text/plain=code.desktop;\ntext/rust=org.kde.kate.desktop;\n\n\
             [Default Applications]\ntext/plain=code.desktop;org.kde.kate.desktop;\n\
             x-scheme-handler/https=firefox.desktop;\ntext/rust=org.kde.kate.desktop;\n\n\
             [Removed Associations]\ntext/html=code.desktop;\n"
        );
        // No temporary file left beside it.
        let names: Vec<_> = std::fs::read_dir(tmp.path().join("config"))
            .expect("dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(names, ["mimeapps.list"]);
    }

    #[test]
    fn a_missing_file_is_created() {
        let tmp = TempDir::new("new");
        set_default(&tmp.dirs(), "image/png", "gimp.desktop").expect("written");
        let apps = MimeApps::load(&tmp.dirs(), &[]);
        assert_eq!(apps.defaults("image/png"), ["gimp.desktop"]);
        assert_eq!(apps.added("image/png"), ["gimp.desktop"]);
    }

    #[test]
    fn files_merge_most_important_first() {
        let tmp = TempDir::new("merge");
        tmp.write(
            "config/kde-mimeapps.list",
            "[Default Applications]\ntext/plain=kwrite.desktop;\n",
        );
        tmp.write("config/mimeapps.list", EXISTING);
        tmp.write(
            "usr/share/applications/mimeapps.list",
            "[Default Applications]\ntext/plain=gedit.desktop;\n",
        );
        let apps = MimeApps::load(&tmp.dirs(), &["kde".into()]);
        assert_eq!(
            apps.defaults("text/plain"),
            [
                "kwrite.desktop",
                "org.kde.kate.desktop",
                "code.desktop",
                "gedit.desktop"
            ]
        );
        assert!(apps.is_removed("text/html", "code.desktop"));
    }
}
