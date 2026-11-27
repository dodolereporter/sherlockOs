//! The name of an application, from its desktop entry (lot D-36): what the bar writes in bold
//! before its menus, « Kate » rather than the title of its window. The window's `app_id` (the
//! xdg-shell one, or the X11 class) names the entry: `org.kde.kate` is `org.kde.kate.desktop`,
//! and an X11 class is matched without its case, or by the program its entry runs.

use std::path::PathBuf;

use mime_apps::{DesktopEntry, Dirs};

/// The name of the application whose windows carry `app_id`, in the user's language. `None`
/// when no entry matches: the bar keeps the window's title. Reads the entries of the data
/// directories: run it off the interface thread.
pub fn of(app_id: &str) -> Option<String> {
    let home = std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from);
    let entries = mime_apps::read_applications(&Dirs::from_env(&home), &launcher::locales());
    find(&entries, app_id)
}

/// The name of the entry of `entries` that `app_id` names.
pub fn find(entries: &[DesktopEntry], app_id: &str) -> Option<String> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return None;
    }
    let lower = app_id.to_lowercase();
    let stem = |entry: &DesktopEntry| entry.id.trim_end_matches(".desktop").to_lowercase();
    let program = |entry: &DesktopEntry| {
        let first = entry.exec.split_whitespace().next().unwrap_or_default();
        first.rsplit('/').next().unwrap_or_default().to_lowercase()
    };
    let by_id = entries.iter().find(|e| stem(e) == lower);
    // `kate` for `org.kde.kate`, or a class such as `Konsole`.
    let by_last = || {
        entries
            .iter()
            .find(|e| stem(e).rsplit('.').next() == Some(lower.as_str()))
    };
    let by_program = || entries.iter().find(|e| program(e) == lower);
    let entry = by_id.or_else(by_last).or_else(by_program)?;
    Some(entry.name.clone()).filter(|n| !n.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, name: &str, exec: &str) -> DesktopEntry {
        DesktopEntry {
            id: id.to_owned(),
            path: PathBuf::from(id),
            name: name.to_owned(),
            generic_name: None,
            keywords: Vec::new(),
            exec: exec.to_owned(),
            working_dir: None,
            icon: None,
            mime_types: Vec::new(),
            no_display: false,
            terminal: false,
            try_exec: None,
        }
    }

    #[test]
    fn the_app_id_names_the_entry() {
        let entries = [
            entry("org.kde.kate.desktop", "Kate", "kate -b %U"),
            entry("org.kde.konsole.desktop", "Konsole", "/usr/bin/konsole"),
            entry(
                "firefox-esr.desktop",
                "Firefox ESR",
                "/usr/lib/firefox-esr/firefox-esr %u",
            ),
        ];
        assert_eq!(find(&entries, "org.kde.kate").as_deref(), Some("Kate"));
        // The X11 class, without its case.
        assert_eq!(find(&entries, "konsole").as_deref(), Some("Konsole"));
        assert_eq!(
            find(&entries, "Firefox-esr").as_deref(),
            Some("Firefox ESR")
        );
        assert_eq!(find(&entries, "inconnue"), None);
        assert_eq!(find(&entries, ""), None);
    }
}
