//! Starting applications from their desktop entry, without a shell.
//!
//! Unlike the bar's monitoring commands, an application must outlive the dock: it gets its own
//! process group, no death signal, and a thread reaps it when it exits.

use std::ffi::OsString;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::entry::{self, DesktopEntry};

/// Error while starting an application.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("{0} has no command")]
    NoCommand(String),
    #[error("cannot start {program}: {source}")]
    Spawn { program: String, source: io::Error },
}

/// What to start: the application itself, a new window of it, or one of its actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    App,
    /// Its « new window » action when it declares one, otherwise its `Exec` again without
    /// argument. An application with a single instance decides what that does.
    NewWindow,
    /// The action of this index in `DesktopEntry::actions`.
    Action(usize),
}

/// The command line `what` runs for `app`.
pub fn command_line(app: &DesktopEntry, what: What) -> Option<Vec<String>> {
    let exec = match what {
        What::App => app.exec.as_deref()?,
        What::NewWindow => match &app.new_window {
            Some(action) => action.exec.as_str(),
            None => app.exec.as_deref()?,
        },
        What::Action(index) => app.actions.get(index)?.exec.as_str(),
    };
    let line = entry::command_line(exec, &app.name, app.icon.as_deref(), &app.file);
    (!line.is_empty()).then_some(line)
}

/// Starts `what` for `app` and returns at once.
pub fn launch(app: &DesktopEntry, what: What) -> Result<u32, LaunchError> {
    let line = command_line(app, what).ok_or_else(|| LaunchError::NoCommand(app.id.clone()))?;
    let (program, args) = line
        .split_first()
        .ok_or_else(|| LaunchError::NoCommand(app.id.clone()))?;
    spawn(Path::new(program), args, app.path.as_deref().map(Path::new))
}

/// Program of the applications view (D-41, `docs/apps.md`).
pub const APPLICATIONS: &str = "sherlockos-apps";

/// The applications view to start: in the folder `var` gives for `SHERLOCKOS_BIN` when it is
/// there, as the compositor finds the launcher, else in the `PATH`.
pub fn applications_program(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    var("SHERLOCKOS_BIN")
        .map(|dir| Path::new(&dir).join(APPLICATIONS))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| APPLICATIONS.into())
}

/// Opens the applications view, or closes it when it is open: a second one closes the first.
pub fn toggle_applications() -> Result<u32, LaunchError> {
    let program = applications_program(|key| std::env::var_os(key));
    spawn(&program, &[], None)
}

/// The file manager: « Afficher dans Evidence » (lot D-21i).
pub const EVIDENCE: &str = "evidence";

/// Opens Evidence on `folder`, found beside the session's programs or in the `PATH`.
pub fn show_in_evidence(folder: &Path) -> Result<u32, LaunchError> {
    let program = std::env::var_os("SHERLOCKOS_BIN")
        .map(|dir| Path::new(&dir).join(EVIDENCE))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| EVIDENCE.into());
    spawn(&program, &[folder.display().to_string()], None)
}

/// Program of the launcher, Super + Space (`docs/launcher.md`).
pub const LAUNCHER: &str = "launcher";

/// The launcher to start, found as the compositor finds it: beside the session's programs
/// (`SHERLOCKOS_BIN`), else in the `PATH`.
pub fn launcher_program(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    var("SHERLOCKOS_BIN")
        .map(|dir| Path::new(&dir).join(LAUNCHER))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| LAUNCHER.into())
}

/// Opens the launcher, where Sherlock is asked, as Super + Space does (lot D-21m): the click
/// on Sherlock's icon until Sherlock has a window of its own (A-19). Started again while open,
/// the launcher closes, as with the shortcut.
pub fn open_launcher() -> Result<u32, LaunchError> {
    let program = launcher_program(|key| std::env::var_os(key));
    spawn(&program, &[], None)
}

/// Starts `program` with `args`, in `dir`, and returns at once.
fn spawn(program: &Path, args: &[String], dir: Option<&Path>) -> Result<u32, LaunchError> {
    let mut command = Command::new(program);
    // The application gets the environment the dock started with, not its renderer choice:
    // a game launched from the dock must still see the NVIDIA card (ADR 0018).
    sherlock_ui::gpu::for_child(&mut command);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own group: a Ctrl+C or a signal sent to the dock's group spares it.
        .process_group(0);
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|source| LaunchError::Spawn {
        program: program.display().to_string(),
        source,
    })?;
    let pid = child.id();
    let reaper = std::thread::Builder::new()
        .name(String::from("reap"))
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(error) = reaper {
        tracing::warn!(%error, pid, "cannot watch the application: it will stay a zombie");
    }
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(exec: &str) -> DesktopEntry {
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Sample\nExec={exec}\nActions=new-window;\n\
             [Desktop Action new-window]\nExec={exec} --new\n"
        );
        DesktopEntry::parse("sample", std::path::Path::new("sample.desktop"), &text)
            .expect("application")
    }

    #[test]
    fn the_new_window_runs_its_own_command() {
        let app = app("sample %U");
        assert_eq!(
            command_line(&app, What::App),
            Some(vec!["sample".to_owned()])
        );
        // `sample %U --new`: the field code goes, the action's own argument stays.
        assert_eq!(
            command_line(&app, What::NewWindow),
            Some(vec!["sample".to_owned(), "--new".to_owned()])
        );
        assert_eq!(
            command_line(&app, What::Action(0)),
            command_line(&app, What::NewWindow)
        );
        assert_eq!(command_line(&app, What::Action(1)), None);
    }

    #[test]
    fn without_a_new_window_action_the_application_is_started_again() {
        let text = "[Desktop Entry]\nType=Application\nName=Single\nExec=single %F\n";
        let app =
            DesktopEntry::parse("single", Path::new("single.desktop"), text).expect("application");
        assert_eq!(
            command_line(&app, What::NewWindow),
            Some(vec!["single".to_owned()]),
            "its Exec, without argument"
        );
    }

    #[test]
    fn a_launched_application_runs_without_a_shell() {
        let dir = std::env::temp_dir().join(format!("dock-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("directory");
        let marker = dir.join("lance; echo injecte");
        // A shell would run the second command; without one, `touch` gets one odd file name.
        let touch = app(&format!("touch \"{}\"", marker.display()));
        launch(&touch, What::App).expect("started");
        for _ in 0..100 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(marker.exists());
        assert_eq!(std::fs::read_dir(&dir).expect("listed").count(), 1);
        let missing = app("/nonexistent/program");
        assert!(matches!(
            launch(&missing, What::App),
            Err(LaunchError::Spawn { .. })
        ));
    }

    #[test]
    fn the_applications_view_is_found_beside_the_programs_then_in_the_path() {
        let dir = std::env::temp_dir().join(format!("dock-apps-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        let var = |key: &str| (key == "SHERLOCKOS_BIN").then(|| dir.clone().into_os_string());
        assert_eq!(applications_program(var), PathBuf::from(APPLICATIONS));
        std::fs::write(dir.join(APPLICATIONS), b"").expect("program");
        assert_eq!(applications_program(var), dir.join(APPLICATIONS));
        assert_eq!(applications_program(|_| None), PathBuf::from(APPLICATIONS));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_launcher_is_found_beside_the_programs_then_in_the_path() {
        let dir = std::env::temp_dir().join(format!("dock-launcher-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("directory");
        let var = |key: &str| (key == "SHERLOCKOS_BIN").then(|| dir.clone().into_os_string());
        assert_eq!(launcher_program(var), PathBuf::from(LAUNCHER));
        std::fs::write(dir.join(LAUNCHER), b"").expect("program");
        assert_eq!(launcher_program(var), dir.join(LAUNCHER));
        assert_eq!(launcher_program(|_| None), PathBuf::from(LAUNCHER));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
