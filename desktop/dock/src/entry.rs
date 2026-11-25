//! Desktop entries (`.desktop` files): the minimum the dock reads from them (name, icon,
//! command, actions, window class). See `docs/dock.md` about the `mime-apps` crate.

use std::path::{Path, PathBuf};

pub use dock_config::keyfile::KeyFile;

const ENTRY: &str = "Desktop Entry";

/// An application, from its desktop entry.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopEntry {
    /// Desktop file id: the file name without `.desktop`, such as `org.kde.konsole`.
    pub id: String,
    pub name: String,
    /// `Icon`: a theme icon name or an absolute path.
    pub icon: Option<String>,
    pub exec: Option<String>,
    /// `Path`: working directory of the command.
    pub path: Option<String>,
    pub categories: Vec<String>,
    /// `StartupWMClass`: the class or `app_id` its windows carry.
    pub wm_class: Option<String>,
    /// Hidden from menus (`NoDisplay`) or deleted (`Hidden`).
    pub hidden: bool,
    /// The « new window » action, when the application declares one.
    pub new_window: Option<Action>,
}

/// An action of a desktop entry (`[Desktop Action …]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: String,
    pub exec: String,
}

impl DesktopEntry {
    /// Reads a desktop entry. `None` if it is not an application.
    pub fn parse(id: &str, text: &str) -> Option<Self> {
        let file = KeyFile::parse(text);
        if file.get(ENTRY, "Type").as_deref() != Some("Application") {
            return None;
        }
        let new_window = file
            .list(ENTRY, "Actions")
            .into_iter()
            .filter(|action| is_new_window(action))
            .find_map(|action| {
                let exec = file.get(&format!("Desktop Action {action}"), "Exec")?;
                Some(Action { id: action, exec })
            });
        Some(Self {
            id: id.to_owned(),
            name: file
                .localized(ENTRY, "Name")
                .unwrap_or_else(|| id.to_owned()),
            icon: file.get(ENTRY, "Icon").filter(|i| !i.is_empty()),
            exec: file.get(ENTRY, "Exec").filter(|e| !e.is_empty()),
            path: file.get(ENTRY, "Path").filter(|p| !p.is_empty()),
            categories: file.list(ENTRY, "Categories"),
            wm_class: file.get(ENTRY, "StartupWMClass").filter(|c| !c.is_empty()),
            hidden: file.flag(ENTRY, "NoDisplay") || file.flag(ENTRY, "Hidden"),
            new_window,
        })
    }

    /// Whether a window with this `app_id` (Wayland) or class (X11) belongs to the application:
    /// its desktop file id, the last part of a reverse-DNS id (`org.kde.konsole` for `konsole`),
    /// or its `StartupWMClass`, ignoring case.
    pub fn owns(&self, app_id: &str) -> bool {
        let same = |name: &str| name.eq_ignore_ascii_case(app_id);
        same(&self.id)
            || self.id.rsplit('.').next().is_some_and(same)
            || self.wm_class.as_deref().is_some_and(same)
    }
}

/// Action ids that open a new window, written in the usual ways (`new-window`, `NewWindow`,
/// `new-empty-window`).
fn is_new_window(action: &str) -> bool {
    let compact: String = action
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(compact.as_str(), "newwindow" | "newemptywindow")
}

/// Splits an `Exec` value into a program and its arguments, as the Desktop Entry specification
/// says: double quotes group, a backslash escapes in them, field codes are dropped (`%f`, `%U`…)
/// or replaced (`%c` by the name, `%i` by `--icon` and the icon, `%%` by `%`). No shell.
pub fn command_line(exec: &str, name: &str, icon: Option<&str>) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut in_word = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            ' ' | '\t' if !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    let mut args = Vec::new();
    for word in words {
        match word.as_str() {
            "%i" => {
                if let Some(icon) = icon {
                    args.push(String::from("--icon"));
                    args.push(icon.to_owned());
                }
            }
            "%c" => args.push(name.to_owned()),
            _ => {
                let expanded = expand_codes(&word, name);
                // A word made only of dropped field codes disappears.
                if !(expanded.is_empty() && word.starts_with('%')) {
                    args.push(expanded);
                }
            }
        }
    }
    args
}

fn expand_codes(word: &str, name: &str) -> String {
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('c') => out.push_str(name),
            // Files, URLs and deprecated codes: the dock opens nothing.
            Some(_) | None => {}
        }
    }
    out
}

/// Directories that hold data files, in the order of precedence of the XDG Base Directory
/// specification: `$XDG_DATA_HOME`, then each of `$XDG_DATA_DIRS`.
pub fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".local/share")));
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
    data_home
        .into_iter()
        .chain(dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from))
        .collect()
}

/// The installed applications found in `<dir>/applications` of each data directory. When the
/// same id is in several directories, the first one wins.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    pub entries: Vec<DesktopEntry>,
}

impl Catalog {
    /// Reads the applications of `data_dirs`. Unreadable files are skipped.
    pub fn load(data_dirs: &[PathBuf]) -> Self {
        let mut entries: Vec<DesktopEntry> = Vec::new();
        for dir in data_dirs {
            let Ok(files) = std::fs::read_dir(dir.join("applications")) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = files.filter_map(|f| f.ok().map(|f| f.path())).collect();
            paths.sort();
            for path in paths {
                let Some(id) = desktop_id(&path) else {
                    continue;
                };
                if entries.iter().any(|e| e.id == id) {
                    continue;
                }
                if let Some(entry) = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| DesktopEntry::parse(&id, &text))
                {
                    entries.push(entry);
                }
            }
        }
        Self { entries }
    }

    /// The application with this desktop file id (`.desktop` suffix accepted).
    pub fn get(&self, id: &str) -> Option<&DesktopEntry> {
        let id = id.strip_suffix(".desktop").unwrap_or(id);
        self.entries.iter().find(|e| e.id == id)
    }
}

fn desktop_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".desktop").map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
    }

    #[test]
    fn a_desktop_entry_gives_name_icon_command_and_new_window() {
        let catalog = Catalog::load(&[fixtures().join("data")]);
        let term = catalog
            .get("org.example.Terminal")
            .expect("terminal fixture");
        assert_eq!(term.name, "Terminal d'exemple");
        assert_eq!(term.icon.as_deref(), Some("example-terminal"));
        assert_eq!(term.categories, ["System", "TerminalEmulator"]);
        let action = term.new_window.as_ref().expect("new window action");
        assert_eq!(action.id, "NewWindow");
        assert_eq!(action.exec, "example-terminal --new-window");
        assert!(!term.hidden);
        assert!(term.owns("terminal") && term.owns("org.example.terminal"));
        assert!(!term.owns("browser"));

        let browser = catalog
            .get("example-browser.desktop")
            .expect("browser fixture");
        assert_eq!(
            browser.new_window, None,
            "only a private window is declared"
        );
        assert!(browser.owns("ExampleBrowser"), "StartupWMClass");
        // Not an application, and hidden ones are still read but marked.
        assert!(catalog.get("example-link").is_none());
        assert!(catalog.get("example-hidden").is_some_and(|e| e.hidden));
    }

    #[test]
    fn the_command_is_split_without_a_shell() {
        let line = command_line(
            r#"example --title "Mon %c" %U --x="a \"b\"" %%"#,
            "App",
            None,
        );
        assert_eq!(line, ["example", "--title", "Mon App", "--x=a \"b\"", "%"]);
        let line = command_line("app %i %f", "App", Some("app-icon"));
        assert_eq!(line, ["app", "--icon", "app-icon"]);
        // Flatpak's forwarding markers are passed on: flatpak reads them.
        let line = command_line(
            "flatpak run --file-forwarding org.ex.App @@u %u @@",
            "",
            None,
        );
        assert_eq!(line.last().map(String::as_str), Some("@@"));
        assert_eq!(line.len(), 6);
    }
}
