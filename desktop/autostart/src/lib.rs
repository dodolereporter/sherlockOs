//! The applications started with the session (lot D-44), after the Desktop Application
//! Autostart Specification: the `.desktop` files of `$XDG_CONFIG_HOME/autostart` (the user's)
//! and of each `$XDG_CONFIG_DIRS/autostart` (`/etc/xdg/autostart`). A file of the user hides the
//! system file of the same name, `Hidden=true` included: that is how a user turns one off.
//!
//! An entry is left out when it is `Hidden`, when `OnlyShowIn` does not name our desktop or
//! `NotShowIn` does (`SherlockOS`, from `XDG_CURRENT_DESKTOP`), when its `TryExec` is not
//! installed, when `X-GNOME-Autostart-enabled=false`, when its `X-KDE-autostart-condition` is
//! false, when it runs in a terminal, and when the session already does what it does
//! (`EXCLUDED`). `X-GNOME-Autostart-Delay` delays it. The command is the `Exec` line, its field
//! codes expanded without files, run without a shell (`mime_apps`). See `docs/autostart.md`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

use mime_apps::{Dirs, entry};

/// What the session already does, by file name or by program: started by the session itself
/// (the polkit agent, the keyring, kwalletd), or the bar's work (network, Bluetooth). KDE's and
/// GNOME's own entries mostly say `OnlyShowIn=KDE` or `GNOME` already; these do not, or not
/// always.
pub const EXCLUDED: &[&str] = &[
    // The polkit agent: the session starts the first one installed.
    "polkit-kde-authentication-agent-1",
    "org.kde.polkit-kde-authentication-agent-1",
    "polkit-gnome-authentication-agent-1",
    "polkit-mate-authentication-agent-1",
    "lxpolkit",
    // The keyring: gnome-keyring-daemon's secrets, or kwalletd6, started by the session.
    "gnome-keyring-secrets",
    "gnome-keyring-daemon",
    "kwalletd5",
    "kwalletd6",
    "org.kde.kwalletd5",
    "org.kde.kwalletd6",
    "pam_kwallet_init",
    // Network and Bluetooth: the bar has them.
    "nm-applet",
    "nm-tray",
    "blueman",
    "blueman-applet",
    // KDE's tray proxy for X11 icons and its daemons of the Plasma session.
    "xembedsniproxy",
    "org.kde.plasmashell",
    "org.kde.kded6",
];

/// Longest delay honored: a typo in `X-GNOME-Autostart-Delay` must not hold an application
/// back for hours.
pub const MAX_DELAY: Duration = Duration::from_secs(300);

/// Where the entries are and who we are.
#[derive(Debug, Clone)]
pub struct Context {
    pub dirs: Dirs,
    /// `XDG_CURRENT_DESKTOP`, split on `:`.
    pub desktops: Vec<String>,
}

impl Context {
    /// From the environment of the process, with the defaults of the specification.
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
        let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        Self {
            dirs: Dirs::from_env(&home),
            desktops: desktops
                .split(':')
                .filter(|d| !d.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }
}

/// An application to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The file name, such as `discord.desktop`.
    pub id: String,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub working_dir: Option<PathBuf>,
    pub delay: Duration,
}

/// Why an entry is left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Skip {
    Hidden,
    /// `OnlyShowIn` or `NotShowIn`.
    OtherDesktop,
    /// `X-GNOME-Autostart-enabled=false`.
    Disabled,
    /// `X-KDE-autostart-condition` is false.
    Condition,
    /// The `TryExec` program is not installed.
    Missing(String),
    /// The session already does it (`EXCLUDED`).
    Excluded,
    Terminal,
    /// Not an application with a command.
    Invalid,
}

/// The `.desktop` files of the autostart folders, the user's first: a name seen once hides the
/// same name further down.
fn files(dirs: &Dirs) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for dir in dirs.config() {
        let Ok(items) = std::fs::read_dir(dir.join("autostart")) else {
            continue;
        };
        let mut here: Vec<(String, PathBuf)> = items
            .filter_map(Result::ok)
            .map(|item| item.path())
            .filter(|path| path.extension().is_some_and(|e| e == "desktop"))
            .filter_map(|path| Some((path.file_name()?.to_str()?.to_owned(), path)))
            .collect();
        here.sort();
        for (id, path) in here {
            if !found.iter().any(|(seen, _)| *seen == id) {
                found.push((id, path));
            }
        }
    }
    found
}

/// The keys of the `[Desktop Entry]` group, unlocalized.
fn keys(text: &str) -> Vec<(String, String)> {
    let mut keys = Vec::new();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
        } else if inside
            && !line.starts_with('#')
            && let Some((key, value)) = line.split_once('=')
        {
            keys.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }
    keys
}

fn list(value: &str) -> impl Iterator<Item = &str> {
    value.split(';').map(str::trim).filter(|v| !v.is_empty())
}

/// `X-KDE-autostart-condition=file:group:key:default`: the boolean `key` of `[group]` in the
/// configuration file `file`, the user's first, `default` when it is not set.
fn kde_condition(dirs: &Dirs, condition: &str) -> bool {
    let mut parts = condition.splitn(4, ':');
    let (Some(file), Some(group), Some(key)) = (parts.next(), parts.next(), parts.next()) else {
        return true;
    };
    let default = parts.next().is_none_or(|d| d.trim() == "true");
    let wanted = format!("[{group}]");
    for dir in dirs.config() {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        let mut inside = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                inside = line == wanted;
            } else if inside
                && let Some((k, v)) = line.split_once('=')
                && k.trim() == key
            {
                return match v.trim() {
                    "true" => true,
                    "false" => false,
                    _ => default,
                };
            }
        }
    }
    default
}

/// The program an `Exec` command runs, without its folder.
fn program_name(program: &OsString) -> String {
    Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether the entry at `path`, named `id`, starts, and how.
pub fn decide(context: &Context, id: &str, path: &Path) -> Result<Launch, Skip> {
    let text = entry::read_entry(path).ok_or(Skip::Invalid)?;
    let keys = keys(&text);
    let get = |key: &str| {
        keys.iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    if get("Hidden") == Some("true") {
        return Err(Skip::Hidden);
    }
    let ours = |value: &str| list(value).any(|d| context.desktops.iter().any(|o| o == d));
    if get("OnlyShowIn").is_some_and(|v| !ours(v)) || get("NotShowIn").is_some_and(ours) {
        return Err(Skip::OtherDesktop);
    }
    if get("X-GNOME-Autostart-enabled") == Some("false") {
        return Err(Skip::Disabled);
    }
    if get("X-KDE-autostart-condition").is_some_and(|c| !kde_condition(&context.dirs, c)) {
        return Err(Skip::Condition);
    }
    if get("Terminal") == Some("true") {
        return Err(Skip::Terminal);
    }
    let app = entry::parse(id, path, &text, &[]).ok_or(Skip::Invalid)?;
    if let Some(program) = &app.try_exec
        && !entry::is_executable(program)
    {
        return Err(Skip::Missing(program.clone()));
    }
    let (program, args) = app.command(&[]).ok_or(Skip::Invalid)?;
    let stem = id.strip_suffix(".desktop").unwrap_or(id);
    if EXCLUDED.contains(&stem) || EXCLUDED.contains(&program_name(&program).as_str()) {
        return Err(Skip::Excluded);
    }
    let delay = get("X-GNOME-Autostart-Delay")
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0)
        .map_or(Duration::ZERO, |d| {
            Duration::from_secs_f64(d.min(MAX_DELAY.as_secs_f64()))
        });
    Ok(Launch {
        id: id.to_owned(),
        program,
        args,
        working_dir: app.working_dir,
        delay,
    })
}

/// Every autostart entry, with what to do with it, the soonest first.
pub fn plan(context: &Context) -> Vec<(String, Result<Launch, Skip>)> {
    let mut plan: Vec<(String, Result<Launch, Skip>)> = files(&context.dirs)
        .into_iter()
        .map(|(id, path)| {
            let decision = decide(context, &id, &path);
            (id, decision)
        })
        .collect();
    plan.sort_by_key(|(_, d)| d.as_ref().map_or(Duration::ZERO, |l| l.delay));
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary folder, removed on drop.
    struct Temp(PathBuf);

    impl Temp {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("autostart-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn context(&self) -> Context {
            Context {
                dirs: Dirs {
                    config_home: self.0.join("home"),
                    config_dirs: vec![self.0.join("etc")],
                    data_home: self.0.join("data"),
                    data_dirs: Vec::new(),
                },
                desktops: vec!["SherlockOS".into()],
            }
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn entry(exec: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName=App\nExec={exec}\n{extra}")
    }

    fn decided(plan: &[(String, Result<Launch, Skip>)], id: &str) -> Result<Launch, Skip> {
        plan.iter().find(|(i, _)| i == id).unwrap().1.clone()
    }

    #[test]
    fn the_user_hides_the_system_entry_of_the_same_name() {
        let t = Temp::new("hide");
        t.write("etc/autostart/chat.desktop", &entry("chat --tray", ""));
        t.write("etc/autostart/mail.desktop", &entry("mail", ""));
        t.write(
            "home/autostart/chat.desktop",
            &entry("chat", "Hidden=true\n"),
        );
        t.write(
            "home/autostart/mail.desktop",
            &entry("mail --quiet %U", "Path=/srv\n"),
        );
        let plan = plan(&t.context());
        assert_eq!(plan.len(), 2);
        assert_eq!(decided(&plan, "chat.desktop"), Err(Skip::Hidden));
        let mail = decided(&plan, "mail.desktop").unwrap();
        assert_eq!(mail.program, "mail");
        assert_eq!(mail.args, ["--quiet"], "field codes expanded without files");
        assert_eq!(mail.working_dir, Some(PathBuf::from("/srv")));
    }

    #[test]
    fn only_our_desktop_and_what_is_enabled_starts() {
        let t = Temp::new("desktops");
        let cases = [
            ("kde.desktop", "OnlyShowIn=KDE;\n"),
            ("ours.desktop", "OnlyShowIn=GNOME;SherlockOS;\n"),
            ("notours.desktop", "NotShowIn=SherlockOS;\n"),
            ("gnome-off.desktop", "X-GNOME-Autostart-enabled=false\n"),
            ("missing.desktop", "TryExec=/nowhere/at-all\n"),
            ("terminal.desktop", "Terminal=true\n"),
            (
                "cond-off.desktop",
                "X-KDE-autostart-condition=apprc:General:Start:true\n",
            ),
            (
                "cond-default.desktop",
                "X-KDE-autostart-condition=nonerc:General:Start:false\n",
            ),
        ];
        for (id, extra) in cases {
            t.write(&format!("etc/autostart/{id}"), &entry("app", extra));
        }
        t.write("home/apprc", "[General]\nStart=false\n");
        let plan = plan(&t.context());
        assert_eq!(decided(&plan, "kde.desktop"), Err(Skip::OtherDesktop));
        assert!(decided(&plan, "ours.desktop").is_ok());
        assert_eq!(decided(&plan, "notours.desktop"), Err(Skip::OtherDesktop));
        assert_eq!(decided(&plan, "gnome-off.desktop"), Err(Skip::Disabled));
        assert_eq!(
            decided(&plan, "missing.desktop"),
            Err(Skip::Missing("/nowhere/at-all".into()))
        );
        assert_eq!(decided(&plan, "terminal.desktop"), Err(Skip::Terminal));
        assert_eq!(decided(&plan, "cond-off.desktop"), Err(Skip::Condition));
        assert_eq!(decided(&plan, "cond-default.desktop"), Err(Skip::Condition));
    }

    #[test]
    fn what_the_session_does_is_not_started_twice() {
        let t = Temp::new("excluded");
        t.write("etc/autostart/nm-applet.desktop", &entry("nm-applet", ""));
        t.write(
            "etc/autostart/agent.desktop",
            &entry("/usr/libexec/polkit-kde-authentication-agent-1", ""),
        );
        let plan = plan(&t.context());
        assert_eq!(decided(&plan, "nm-applet.desktop"), Err(Skip::Excluded));
        assert_eq!(decided(&plan, "agent.desktop"), Err(Skip::Excluded));
    }

    #[test]
    fn delays_are_kept_and_bounded() {
        let t = Temp::new("delays");
        t.write(
            "etc/autostart/late.desktop",
            &entry("late", "X-GNOME-Autostart-Delay=2\n"),
        );
        t.write(
            "etc/autostart/huge.desktop",
            &entry("huge", "X-GNOME-Autostart-Delay=99999\n"),
        );
        t.write(
            "etc/autostart/bad.desktop",
            &entry("bad", "X-GNOME-Autostart-Delay=soon\n"),
        );
        t.write(
            "etc/autostart/broken.desktop",
            "[Desktop Entry]\nType=Link\n",
        );
        let plan = plan(&t.context());
        let delay = |id| decided(&plan, id).map(|l| l.delay);
        assert_eq!(delay("late.desktop"), Ok(Duration::from_secs(2)));
        assert_eq!(delay("huge.desktop"), Ok(MAX_DELAY));
        assert_eq!(delay("bad.desktop"), Ok(Duration::ZERO));
        assert_eq!(decided(&plan, "broken.desktop"), Err(Skip::Invalid));
        // The soonest first.
        assert_eq!(plan.last().unwrap().0, "huge.desktop");
    }
}
