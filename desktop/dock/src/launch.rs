//! Starting applications from their desktop entry, without a shell.
//!
//! Unlike the bar's monitoring commands, an application must outlive the dock: it gets its own
//! process group, no death signal, and a thread reaps it when it exits.

use std::io;
use std::os::unix::process::CommandExt;
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

/// What to start: the application itself, or its « new window » action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    App,
    NewWindow,
}

/// The command line `what` runs for `app`.
pub fn command_line(app: &DesktopEntry, what: What) -> Option<Vec<String>> {
    let exec = match what {
        What::App => app.exec.as_deref()?,
        What::NewWindow => app.new_window.as_ref()?.exec.as_str(),
    };
    let line = entry::command_line(exec, &app.name, app.icon.as_deref());
    (!line.is_empty()).then_some(line)
}

/// Starts `what` for `app` and returns at once.
pub fn launch(app: &DesktopEntry, what: What) -> Result<u32, LaunchError> {
    let line = command_line(app, what).ok_or_else(|| LaunchError::NoCommand(app.id.clone()))?;
    let (program, args) = line
        .split_first()
        .ok_or_else(|| LaunchError::NoCommand(app.id.clone()))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own group: a Ctrl+C or a signal sent to the dock's group spares it.
        .process_group(0);
    if let Some(dir) = app.path.as_deref() {
        command.current_dir(dir);
    }
    let mut child = command.spawn().map_err(|source| LaunchError::Spawn {
        program: program.clone(),
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
        DesktopEntry::parse("sample", &text).expect("application")
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
}
