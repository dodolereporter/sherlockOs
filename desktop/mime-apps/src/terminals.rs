//! The default terminal: `$XDG_CONFIG_HOME/xdg-terminals.list`, after the proposed
//! freedesktop specification that `xdg-terminal-exec` follows. Each line names a desktop
//! entry, the preferred first; `#` starts a comment. Control Room's « Applications par
//! défaut » page writes it (lot D-26a).

use std::io;
use std::path::PathBuf;

use crate::{Catalog, DesktopEntry, Dirs};

/// Name of the file, in the configuration folder.
pub const FILE: &str = "xdg-terminals.list";

/// Category of the desktop entry of a terminal.
pub const CATEGORY: &str = "TerminalEmulator";

fn path(dirs: &Dirs) -> PathBuf {
    dirs.config_home.join(FILE)
}

/// The desktop ids of the list, in order: comments, blank lines and actions
/// (`id.desktop:action`) left aside.
fn ids(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split(':').next().unwrap_or(line))
        .collect()
}

/// The installed terminals, by name.
pub fn installed(catalog: &Catalog) -> Vec<&DesktopEntry> {
    catalog
        .choosable()
        .into_iter()
        .filter(|app| app.categories.iter().any(|c| c == CATEGORY))
        .collect()
}

/// The default terminal: the first of the list that is installed, else the first installed.
pub fn default(catalog: &Catalog) -> Option<&DesktopEntry> {
    let text = std::fs::read_to_string(path(&catalog.dirs)).unwrap_or_default();
    let terminals = installed(catalog);
    ids(&text)
        .into_iter()
        .find_map(|id| terminals.iter().copied().find(|app| app.id == id))
        .or_else(|| terminals.first().copied())
}

/// Makes `id` the default terminal: first in the list, the others kept after it.
pub fn set_default(dirs: &Dirs, id: &str) -> io::Result<()> {
    let path = path(dirs);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut lines = vec![id.to_owned()];
    lines.extend(
        text.lines()
            .filter(|line| ids(line).first() != Some(&id))
            .map(str::to_owned),
    );
    crate::list::write_atomically(&path, &(lines.join("\n") + "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn the_chosen_terminal_comes_first_and_the_others_stay() {
        let tmp = TempDir::new("terminals");
        for (id, name) in [("foot", "Foot"), ("org.kde.konsole", "Konsole")] {
            tmp.write(
                &format!("usr/share/applications/{id}.desktop"),
                &format!("[Desktop Entry]\nType=Application\nName={name}\nExec={id}\nCategories=System;TerminalEmulator;\n"),
            );
        }
        tmp.write(
            "usr/share/applications/kate.desktop",
            "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate\nCategories=Utility;\n",
        );
        let catalog = Catalog::load_with_terminal(tmp.dirs(), &[], &[], None);
        let names: Vec<&str> = installed(&catalog)
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["Foot", "Konsole"]);
        assert_eq!(
            default(&catalog).map(|a| a.id.as_str()),
            Some("foot.desktop")
        );
        tmp.write(
            "config/xdg-terminals.list",
            "# mine\nother.desktop\nfoot.desktop\n",
        );
        set_default(&tmp.dirs(), "org.kde.konsole.desktop").unwrap();
        let catalog = Catalog::load_with_terminal(tmp.dirs(), &[], &[], None);
        let chosen = default(&catalog).map(|a| a.id.as_str());
        assert_eq!(chosen, Some("org.kde.konsole.desktop"));
        let text = std::fs::read_to_string(tmp.path().join("config").join(FILE)).unwrap();
        assert_eq!(
            text,
            "org.kde.konsole.desktop\n# mine\nother.desktop\nfoot.desktop\n"
        );
        set_default(&tmp.dirs(), "foot.desktop").unwrap();
        let text = std::fs::read_to_string(tmp.path().join("config").join(FILE)).unwrap();
        assert_eq!(
            text,
            "foot.desktop\norg.kde.konsole.desktop\n# mine\nother.desktop\n"
        );
    }
}
