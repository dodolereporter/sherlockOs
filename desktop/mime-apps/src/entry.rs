//! Desktop entries (Desktop Entry Specification 1.5): the `.desktop` files that describe
//! installed applications, and their `Exec` command line.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::Dirs;

/// An installed application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Desktop file id: its path under `applications/`, with `/` replaced by `-`.
    pub id: String,
    pub path: PathBuf,
    /// Name in the user's language when the entry has one.
    pub name: String,
    /// `GenericName`, such as « Navigateur web », and `Keywords`, both localized: what a
    /// search matches besides the name.
    pub generic_name: Option<String>,
    pub keywords: Vec<String>,
    pub exec: String,
    /// `Path`: working directory of the command.
    pub working_dir: Option<PathBuf>,
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    /// Hidden from menus, but still able to open files.
    pub no_display: bool,
    pub terminal: bool,
    /// Program that must exist for the entry to be shown (`TryExec`).
    pub try_exec: Option<String>,
}

/// A program name or path that can be run: an executable file, found in `PATH` when the name
/// has no slash.
pub fn is_executable(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let runnable = |path: &Path| {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return runnable(Path::new(program));
    }
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| runnable(&dir.join(program))))
}

/// Reads the value of `key` in the `[Desktop Entry]` group, trying the locales first
/// (`Name[fr_FR]`, then `Name[fr]`, then `Name`).
fn value<'a>(group: &'a [(String, String)], key: &str, locales: &[String]) -> Option<&'a str> {
    let localized = locales.iter().map(|l| format!("{key}[{l}]"));
    localized
        .chain(std::iter::once(key.to_owned()))
        .find_map(|k| {
            group
                .iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.as_str())
        })
}

/// Unescapes a string value (`\s`, `\n`, `\t`, `\r`, `\\`). Any other backslash stays,
/// for the `Exec` quoting rules that come after.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Splits a list value (`a;b;c;`) on its separators; `\;` is a semicolon inside an item.
/// Items are unescaped, empty ones dropped.
fn split_list(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut item = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => item.push(';'),
                Some(next) => {
                    item.push('\\');
                    item.push(next);
                }
                None => item.push('\\'),
            },
            ';' => items.push(std::mem::take(&mut item)),
            c => item.push(c),
        }
    }
    items.push(item);
    items
        .iter()
        .map(|i| unescape(i))
        .filter(|i| !i.is_empty())
        .collect()
}

/// Parses one `.desktop` file. Only applications that can run are kept: `Type=Application`,
/// an `Exec`, not `Hidden`.
pub fn parse(id: &str, path: &Path, text: &str, locales: &[String]) -> Option<DesktopEntry> {
    let mut group = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
        } else if inside
            && !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            group.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    let get = |key: &str| value(&group, key, locales);
    let flag = |key: &str| get(key) == Some("true");
    if get("Type") != Some("Application") || flag("Hidden") {
        return None;
    }
    let exec = unescape(get("Exec")?);
    let list = |key: &str| get(key).map(split_list).unwrap_or_default();
    Some(DesktopEntry {
        id: id.to_owned(),
        path: path.to_path_buf(),
        name: unescape(get("Name")?),
        generic_name: get("GenericName").map(unescape).filter(|g| !g.is_empty()),
        keywords: list("Keywords"),
        exec,
        working_dir: get("Path")
            .map(unescape)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
        icon: get("Icon").map(unescape).filter(|i| !i.is_empty()),
        mime_types: list("MimeType"),
        no_display: flag("NoDisplay"),
        terminal: flag("Terminal"),
        try_exec: get("TryExec").map(unescape).filter(|t| !t.is_empty()),
    })
}

/// Collects the `.desktop` files under `dir`. A folder already seen (by its real path) is not
/// entered again: a loop of symbolic links cannot list the same application twice.
fn walk(root: &Path, dir: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<(String, PathBuf)>) {
    let Ok(real) = std::fs::canonicalize(dir) else {
        return;
    };
    if !seen.insert(real) {
        return;
    }
    let Ok(items) = std::fs::read_dir(dir) else {
        return;
    };
    for item in items.filter_map(Result::ok) {
        let path = item.path();
        if path.is_dir() {
            walk(root, &path, seen, out);
        } else if path.extension().is_some_and(|e| e == "desktop")
            && let Ok(relative) = path.strip_prefix(root)
        {
            let id = relative.to_string_lossy().replace('/', "-");
            out.push((id, path));
        }
    }
}

/// Every application installed under the data directories. A desktop id found in a more
/// important directory hides the same id further down, even when that first file is
/// `Hidden` (that is how a user removes a system application).
pub fn read_applications(dirs: &Dirs, locales: &[String]) -> Vec<DesktopEntry> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for data in dirs.data() {
        let root = data.join("applications");
        let mut files = Vec::new();
        walk(&root, &root, &mut HashSet::new(), &mut files);
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(text) = read_entry(&path)
                && let Some(app) = parse(&id, &path, &text, locales)
                && app.try_exec.as_deref().is_none_or(is_executable)
            {
                apps.push(app);
            }
        }
    }
    apps
}

/// The largest desktop entry read: real ones hold a few kilobytes, even with every
/// translation.
pub const MAX_ENTRY_SIZE: u64 = 256 * 1024;

/// The text of the desktop entry at `path`: only a regular file once its links are followed,
/// of `MAX_ENTRY_SIZE` at most, in UTF-8. A link to `/dev/zero`, a pipe or a huge file in an
/// `applications` folder would otherwise take all the memory of whoever lists them.
fn read_entry(path: &Path) -> Option<String> {
    use std::io::Read;
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_ENTRY_SIZE + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_ENTRY_SIZE {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Splits an `Exec` value into arguments: spaces separate them, double quotes group them,
/// and inside quotes `\"`, `` \` ``, `\$` and `\\` stand for the character.
fn split(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ' ' | '\t' if !quoted => {
                if started || !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started || !current.is_empty() {
        args.push(current);
    }
    args
}

impl DesktopEntry {
    /// The program and arguments that open `files`, field codes expanded: `%f`/`%u` take
    /// one file, `%F`/`%U` all of them, `%i` the icon, `%c` the name, `%k` the entry's path,
    /// `%%` a percent sign; deprecated codes are dropped. Nothing goes through a shell.
    pub fn command(&self, files: &[PathBuf]) -> Option<(OsString, Vec<OsString>)> {
        let mut out: Vec<OsString> = Vec::new();
        for arg in split(&self.exec) {
            match arg.as_str() {
                "%f" | "%u" => out.extend(files.first().map(|f| f.clone().into_os_string())),
                "%F" | "%U" => out.extend(files.iter().map(|f| f.clone().into_os_string())),
                "%i" => {
                    if let Some(icon) = &self.icon {
                        out.push("--icon".into());
                        out.push(icon.into());
                    }
                }
                "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => {}
                _ => {
                    let mut expanded = String::new();
                    let mut chars = arg.chars();
                    while let Some(c) = chars.next() {
                        if c != '%' {
                            expanded.push(c);
                            continue;
                        }
                        match chars.next() {
                            Some('%') => expanded.push('%'),
                            Some('c') => expanded.push_str(&self.name),
                            Some('k') => expanded.push_str(&self.path.to_string_lossy()),
                            Some('f' | 'u') => {
                                if let Some(file) = files.first() {
                                    expanded.push_str(&file.to_string_lossy());
                                }
                            }
                            _ => {}
                        }
                    }
                    out.push(expanded.into());
                }
            }
        }
        if out.is_empty() {
            return None;
        }
        let program = out.remove(0);
        Some((program, out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn only_regular_files_of_a_sane_size_are_read() {
        let dir = TempDir::new("sizes");
        let entry =
            |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name}\n");
        dir.write("data/applications/plain.desktop", &entry("Plain"));
        // Just past the limit: left out, even though it starts as a valid entry.
        let padding = "#".repeat(MAX_ENTRY_SIZE as usize);
        dir.write(
            "data/applications/huge.desktop",
            &format!("{}{padding}\n", entry("Huge")),
        );
        let apps = dir.path().join("data/applications");
        // A link to a device that never ends, a link to a valid entry, a folder named as one.
        std::os::unix::fs::symlink("/dev/zero", apps.join("zero.desktop")).expect("link");
        std::os::unix::fs::symlink(apps.join("plain.desktop"), apps.join("linked.desktop"))
            .expect("link");
        std::fs::create_dir_all(apps.join("folder.desktop")).expect("folder");
        let found = read_applications(&dir.dirs(), &[]);
        let mut ids: Vec<&str> = found.iter().map(|a| a.id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, ["linked.desktop", "plain.desktop"]);
    }

    #[test]
    fn entries_read_localized_names_and_types() {
        let text = "[Desktop Entry]\nType=Application\nName=Text Editor\nName[fr]=Éditeur de texte\n\
                    GenericName=Editor\nGenericName[fr]=Éditeur\nKeywords=text;code;\n\
                    Keywords[fr]=texte;code;a\\;b;c\\\\;\nPath=/srv/my\\snotes\nExec=kate -b %U\nIcon=kate\nMimeType=text/plain;text/x-c;\n\
                    [Desktop Action new]\nName=New\n";
        let app = parse(
            "kate.desktop",
            Path::new("/x/kate.desktop"),
            text,
            &["fr_FR".into(), "fr".into()],
        )
        .expect("entry");
        assert_eq!(app.name, "Éditeur de texte");
        assert_eq!(app.mime_types, ["text/plain", "text/x-c"]);
        assert_eq!(app.generic_name.as_deref(), Some("Éditeur"));
        // `\;` stays in its item; `\\` then `;` ends one.
        assert_eq!(app.keywords, ["texte", "code", "a;b", "c\\"]);
        assert_eq!(app.working_dir, Some(PathBuf::from("/srv/my notes")));
        assert!(
            parse(
                "x",
                Path::new("x"),
                "[Desktop Entry]\nType=Link\nName=x\nExec=x\n",
                &[]
            )
            .is_none()
        );
        assert!(
            parse(
                "x",
                Path::new("x"),
                "[Desktop Entry]\nType=Application\nName=x\nExec=x\nHidden=true\n",
                &[]
            )
            .is_none()
        );
    }

    #[test]
    fn exec_lines_expand_without_a_shell() {
        let app = DesktopEntry {
            id: "a.desktop".into(),
            path: PathBuf::from("/apps/a.desktop"),
            name: "App".into(),
            generic_name: None,
            keywords: Vec::new(),
            exec: r#""/opt/My App/bin/app" --title "%c's \"window\"" %U %i --flag=%%"#.into(),
            working_dir: None,
            icon: Some("app".into()),
            mime_types: Vec::new(),
            no_display: false,
            terminal: false,
            try_exec: None,
        };
        let files = [PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c.txt")];
        let (program, args) = app.command(&files).expect("command");
        assert_eq!(program, "/opt/My App/bin/app");
        let args: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--title",
                "App's \"window\"",
                "/tmp/a b.txt",
                "/tmp/c.txt",
                "--icon",
                "app",
                "--flag=%"
            ]
        );
    }

    #[test]
    fn the_users_entries_hide_the_systems() {
        let tmp = TempDir::new("entries");
        tmp.write(
            "usr/share/applications/kde/org.kde.kate.desktop",
            "[Desktop Entry]\nType=Application\nName=Kate\nExec=kate %U\n",
        );
        tmp.write(
            "usr/share/applications/firefox.desktop",
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\n",
        );
        tmp.write(
            "data/applications/firefox.desktop",
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\nHidden=true\n",
        );
        let apps = read_applications(&tmp.dirs(), &[]);
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["kde-org.kde.kate.desktop"]);
    }

    #[test]
    fn missing_programs_and_link_loops_are_left_out() {
        let tmp = TempDir::new("tryexec");
        tmp.write(
            "usr/share/applications/gone.desktop",
            "[Desktop Entry]\nType=Application\nName=Gone\nExec=gone %F\nTryExec=/nonexistent/gone\n",
        );
        tmp.write(
            "usr/share/applications/here.desktop",
            "[Desktop Entry]\nType=Application\nName=Here\nExec=sh %F\nTryExec=sh\n",
        );
        let loop_dir = tmp.path().join("usr/share/applications/loop");
        std::fs::create_dir_all(&loop_dir).expect("folder");
        std::os::unix::fs::symlink("..", loop_dir.join("back")).expect("link");
        let apps = read_applications(&tmp.dirs(), &[]);
        let ids: Vec<&str> = apps.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["here.desktop"]);
    }
}
