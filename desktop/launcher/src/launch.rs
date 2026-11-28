//! Starting a program without a shell, as the dock does (`desktop/dock/src/launch.rs`).

use std::io;
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::Mutex;

use iced_exwlshell::reexport::{ActivationTokenSink, XdgActivationTokenV1State};

use crate::source::Command;

/// Error while starting a program.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// The entry's `Path` is not a folder: said apart, the program itself is not at fault.
    #[error("cannot start {program}: its working directory {dir} is not a folder")]
    MissingDir { program: String, dir: String },
    #[error("cannot start {program}: {source}")]
    Spawn { program: String, source: io::Error },
    /// The program ran and said no: logind or polkit refused a session command.
    #[error("{program} refused: {detail}")]
    Refused { program: String, detail: String },
}

/// Starts `command` and returns at once with its process id. The program outlives the
/// launcher, which closes right after: its own process group, no standard streams.
pub fn spawn(command: &Command) -> Result<u32, LaunchError> {
    spawn_with(command, None)
}

/// `spawn`, the program given the activation `token` of the click that starts it (lot Q-52).
pub fn spawn_with(command: &Command, token: Option<&str>) -> Result<u32, LaunchError> {
    let mut process = std::process::Command::new(&command.program);
    give_token(&mut process, token);
    // The program gets the environment the launcher started with, not its renderer choice: a
    // game started from here must still see the NVIDIA card (ADR 0018).
    sherlock_ui::gpu::for_child(&mut process);
    process
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let program = || command.program.to_string_lossy().into_owned();
    if let Some(dir) = &command.dir {
        if !dir.is_dir() {
            return Err(LaunchError::MissingDir {
                program: program(),
                dir: dir.display().to_string(),
            });
        }
        process.current_dir(dir);
    }
    let mut child = process.spawn().map_err(|source| LaunchError::Spawn {
        program: program(),
        source,
    })?;
    let pid = child.id();
    // Should the launcher stay open (a failed exit), the program is still reaped.
    let reaper = std::thread::Builder::new()
        .name(String::from("reap"))
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(error) = reaper {
        tracing::warn!(%error, pid, "cannot watch the program");
    }
    Ok(pid)
}

/// A sink for the activation token the compositor gives on behalf of the surface where the user
/// clicked (lot Q-52): `then` runs once with it, or without one when none comes (no
/// `xdg-activation-v1`, the surface gone, the request dropped).
pub fn after_token(then: impl FnOnce(Option<String>) + Send + 'static) -> ActivationTokenSink {
    type Then = Box<dyn FnOnce(Option<String>) + Send>;
    struct Once(Mutex<Option<Then>>);
    impl Once {
        fn run(&self, token: Option<String>) {
            let then = self.0.lock().ok().and_then(|mut then| then.take());
            if let Some(then) = then {
                then(token);
            }
        }
    }
    impl Drop for Once {
        fn drop(&mut self) {
            self.run(None);
        }
    }
    let once = Once(Mutex::new(Some(Box::new(then))));
    ActivationTokenSink::callback(move |state| {
        let token = match state {
            XdgActivationTokenV1State::Ready(token) => Some(token.token().to_owned()),
            XdgActivationTokenV1State::Unsupported | XdgActivationTokenV1State::NoSurface => None,
        };
        once.run(token);
    })
}

/// Gives a started program the activation `token`, Wayland's and X11's names for it: an
/// application already open that gets the launch can bring its window forward with it.
pub fn give_token(process: &mut std::process::Command, token: Option<&str>) {
    if let Some(token) = token {
        process
            .env("XDG_ACTIVATION_TOKEN", token)
            .env("DESKTOP_STARTUP_ID", token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_started_program_gets_the_activation_token() {
        let file = std::env::temp_dir().join(format!("launcher-token-{}", std::process::id()));
        let command = Command {
            program: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "printf '%s %s' \"$XDG_ACTIVATION_TOKEN\" \"$DESKTOP_STARTUP_ID\" > '{}'",
                    file.display()
                )
                .into(),
            ],
            dir: None,
        };
        spawn_with(&command, Some("click-token")).unwrap();
        let mut read = String::new();
        for _ in 0..100 {
            read = std::fs::read_to_string(&file).unwrap_or_default();
            if !read.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&file);
        assert_eq!(read, "click-token click-token");
    }

    #[test]
    fn without_a_token_the_start_still_happens_once() {
        let runs = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen = std::sync::Arc::clone(&runs);
        let sink = after_token(move |token| seen.lock().unwrap().push(token));
        sink.deliver(XdgActivationTokenV1State::Unsupported);
        sink.deliver(XdgActivationTokenV1State::NoSurface);
        drop(sink);
        assert_eq!(*runs.lock().unwrap(), [None]);
        // A request dropped without an answer starts it all the same.
        let seen = std::sync::Arc::clone(&runs);
        drop(after_token(move |token| seen.lock().unwrap().push(token)));
        assert_eq!(runs.lock().unwrap().len(), 2);
    }
}
