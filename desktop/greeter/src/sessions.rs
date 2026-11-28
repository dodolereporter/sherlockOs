//! The sessions the screen can start, from the desktop entries of `wayland-sessions` in the data
//! folders (`/usr/local/share` then `/usr/share`): `sherlockos.desktop`, the desktop of
//! SherlockOS (« Bureau »), and the others (Plasma) as a way back, in a discreet list. The first
//! entry of an id wins, as in the XDG specification. Entries that are hidden, without a command,
//! or whose `TryExec` is not there are left out.

use std::path::{Path, PathBuf};

use crate::login::Launch;
use crate::users::read_bounded;

/// The id of the session of SherlockOS (`desktop/compositor/session/sherlockos.desktop`).
pub const SHERLOCKOS: &str = "sherlockos";

/// The most entries read: a folder with more is not a folder of sessions.
pub const MAX_ENTRIES: usize = 64;

/// A session of `wayland-sessions`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The file name, without `.desktop`.
    pub id: String,
    /// Its `Name`.
    pub name: String,
    /// Its `Exec`, split into words.
    pub command: Vec<String>,
    /// Its `DesktopNames`.
    pub desktops: Vec<String>,
}

impl Session {
    /// What greetd starts: the command, and the variables a display manager gives a Wayland
    /// session.
    pub fn launch(&self) -> Launch {
        let desktop = self.desktops.first().cloned().unwrap_or(self.id.clone());
        let mut env = vec![
            "XDG_SESSION_TYPE=wayland".to_owned(),
            format!("XDG_SESSION_DESKTOP={desktop}"),
        ];
        if !self.desktops.is_empty() {
            env.push(format!("XDG_CURRENT_DESKTOP={}", self.desktops.join(":")));
        }
        Launch {
            command: self.command.clone(),
            env,
        }
    }
}

/// The folders of the sessions, in order.
pub fn folders() -> Vec<PathBuf> {
    let dirs = std::env::var_os("XDG_DATA_DIRS").filter(|d| !d.is_empty());
    let dirs: Vec<PathBuf> = match dirs {
        Some(dirs) => std::env::split_paths(&dirs)
            .filter(|p| p.is_absolute())
            .collect(),
        None => vec!["/usr/local/share".into(), "/usr/share".into()],
    };
    dirs.into_iter()
        .map(|d| d.join("wayland-sessions"))
        .collect()
}

/// The sessions of `folders`: SherlockOS first, then the others by name.
pub fn list(folders: &[PathBuf]) -> Vec<Session> {
    let mut sessions: Vec<Session> = Vec::new();
    for folder in folders {
        let Ok(entries) = std::fs::read_dir(folder) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "desktop"))
            .take(MAX_ENTRIES)
            .collect();
        paths.sort();
        for path in paths {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if sessions.iter().any(|s| s.id == id) {
                continue;
            }
            if let Some(session) = read_bounded(&path).and_then(|text| parse(id, &text)) {
                sessions.push(session);
            }
        }
    }
    sessions.sort_by(|a, b| {
        (b.id == SHERLOCKOS)
            .cmp(&(a.id == SHERLOCKOS))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    sessions
}

/// Reads the `[Desktop Entry]` group of a session entry.
pub fn parse(id: &str, text: &str) -> Option<Session> {
    let mut group = false;
    let (mut name, mut exec, mut try_exec, mut desktops) = (None, None, None, Vec::new());
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            group = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| group) else {
            continue;
        };
        match key.trim() {
            "Name" => name = Some(value.trim().to_owned()),
            "Exec" => exec = Some(value.trim().to_owned()),
            "TryExec" => try_exec = Some(value.trim().to_owned()),
            "DesktopNames" => {
                desktops = value
                    .split(';')
                    .filter(|d| !d.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "Hidden" | "NoDisplay" if value.trim() == "true" => return None,
            _ => {}
        }
    }
    if try_exec.is_some_and(|program| !installed(&program)) {
        return None;
    }
    let command = split_exec(&exec?)?;
    Some(Session {
        id: id.to_owned(),
        name: name.unwrap_or_else(|| id.to_owned()),
        command,
        desktops,
    })
}

/// `program` is an executable file, by its path or in the `PATH`.
fn installed(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    let executable = |path: &Path| {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| executable(&dir.join(program))))
}

/// The words of an `Exec` value, as the desktop entry specification quotes them; its field
/// codes (`%f`, `%U`…) dropped and `%%` kept as `%`. `None` if a quote is not closed, or if
/// nothing is left.
pub fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let (mut word, mut quoted, mut any) = (String::new(), false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' if quoted => word.push(chars.next()?),
            c if c.is_whitespace() && !quoted => {
                if any {
                    words.push(std::mem::take(&mut word));
                    any = false;
                }
            }
            '%' => {
                if chars.next()? == '%' {
                    word.push('%');
                    any = true;
                }
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if any {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = "[Desktop Entry]\nType=Application\nName=SherlockOS\n\
        Exec=/usr/local/libexec/sherlockos/sherlockos-session\nDesktopNames=SherlockOS\n";
    const PLASMA: &str = "[Desktop Entry]\nExec=/usr/lib/plasma-dbus-run-session-if-needed \
        /usr/bin/startplasma-wayland\nName=Plasma (Wayland)\nDesktopNames=KDE\n\
        [Desktop Action Other]\nExec=nothing\n";

    #[test]
    fn an_entry_gives_its_command_and_desktop() {
        let ours = parse("sherlockos", OURS).unwrap();
        assert_eq!(ours.name, "SherlockOS");
        assert_eq!(
            ours.launch(),
            Launch {
                command: vec!["/usr/local/libexec/sherlockos/sherlockos-session".into()],
                env: vec![
                    "XDG_SESSION_TYPE=wayland".into(),
                    "XDG_SESSION_DESKTOP=SherlockOS".into(),
                    "XDG_CURRENT_DESKTOP=SherlockOS".into(),
                ],
            }
        );
        let plasma = parse("plasma", PLASMA).unwrap();
        assert_eq!(plasma.command.len(), 2);
        assert_eq!(plasma.desktops, ["KDE"]);
    }

    #[test]
    fn hidden_entries_and_missing_programs_are_left_out() {
        assert!(parse("x", "[Desktop Entry]\nExec=a\nHidden=true\n").is_none());
        assert!(parse("x", "[Desktop Entry]\nExec=a\nNoDisplay=true\n").is_none());
        assert!(parse("x", "[Desktop Entry]\nName=No command\n").is_none());
        let missing = "[Desktop Entry]\nExec=a\nTryExec=/nowhere/at-all\n";
        assert!(parse("x", missing).is_none());
        let there = "[Desktop Entry]\nExec=a\nTryExec=/bin/sh\n";
        assert!(parse("x", there).is_some());
    }

    #[test]
    fn exec_is_split_as_the_specification_quotes_it() {
        let words = |exec| split_exec(exec).unwrap();
        assert_eq!(words("a  b"), ["a", "b"]);
        assert_eq!(
            words(r#""/opt/my app/run" --x"#),
            ["/opt/my app/run", "--x"]
        );
        assert_eq!(words(r#""say \"hi\"" %U 100%%"#), ["say \"hi\"", "100%"]);
        assert_eq!(words(r#""""#), [""]);
        assert_eq!(split_exec(r#""open"#), None);
        assert_eq!(split_exec("%f"), None);
    }

    #[test]
    fn sherlockos_comes_first_and_the_first_folder_wins() {
        let dir = std::env::temp_dir().join(format!("greeter-sessions-{}", std::process::id()));
        let (local, share) = (dir.join("local"), dir.join("share"));
        for d in [&local, &share] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(share.join("plasma.desktop"), PLASMA).unwrap();
        std::fs::write(share.join("sherlockos.desktop"), OURS).unwrap();
        let older = OURS.replace("Name=SherlockOS", "Name=Older");
        std::fs::write(local.join("sherlockos.desktop"), older).unwrap();
        std::fs::write(share.join("notes.txt"), "not a session").unwrap();
        let sessions = list(&[local, share]);
        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["sherlockos", "plasma"]);
        assert_eq!(sessions[0].name, "Older");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
