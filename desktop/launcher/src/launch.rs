//! Starting a program without a shell, as the dock does (`desktop/dock/src/launch.rs`).

use std::io;
use std::os::unix::process::CommandExt;
use std::process::Stdio;

use crate::source::Command;

/// Error while starting a program.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// The entry's `Path` is not a folder: said apart, the program itself is not at fault.
    #[error("cannot start {program}: its working directory {dir} is not a folder")]
    MissingDir { program: String, dir: String },
    #[error("cannot start {program}: {source}")]
    Spawn { program: String, source: io::Error },
}

/// Starts `command` and returns at once with its process id. The program outlives the
/// launcher, which closes right after: its own process group, no standard streams.
pub fn spawn(command: &Command) -> Result<u32, LaunchError> {
    let mut process = std::process::Command::new(&command.program);
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
