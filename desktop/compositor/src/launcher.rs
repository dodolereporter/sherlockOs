//! The keyboard shortcut of the launcher (lot C-01): Super + Space, or Alt + Space in nested
//! mode, starts `launcher` with the compositor's Wayland socket (and its X11 display). The
//! launcher is started on demand: started again while open, it closes the open one, so the
//! same shortcut opens and closes it (`docs/launcher.md`). The program is
//! `$SHERLOCKOS_BIN/launcher` when that file exists, `launcher` in the session's `PATH`
//! otherwise. F4 starts the applications view, `sherlockos-apps`, the same way (lot D-41), and
//! Super + V the clipboard history panel, `sherlockos-clipboard` (lot D-42).

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use crate::State;

/// Name of the program.
pub const PROGRAM: &str = "launcher";

/// The applications view (lot D-41), found the same way, started by F4.
pub const APPLICATIONS: &str = "sherlockos-apps";

/// The clipboard history panel (lot D-42), found the same way, started by Super + V.
pub const CLIPBOARD: &str = "sherlockos-clipboard";

/// The folder of the session's programs, as `sherlockos-session` exports it.
pub const BIN_VAR: &str = "SHERLOCKOS_BIN";

/// The launcher to start: `PROGRAM` in the folder `var` gives for `BIN_VAR` if it is there,
/// else `PROGRAM` alone, looked for in the `PATH`.
pub fn program(var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    program_named(PROGRAM, var)
}

/// `name` in the folder `var` gives for `BIN_VAR` if it is there, else `name` in the `PATH`.
pub fn program_named(name: &str, var: impl Fn(&str) -> Option<OsString>) -> PathBuf {
    var(BIN_VAR)
        .map(|dir| Path::new(&dir).join(name))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| name.into())
}

impl State {
    /// Replaces the program the shortcut starts (the tests, or another launcher).
    pub fn set_launcher_program(&mut self, program: PathBuf) {
        self.launcher_program = Some(program);
    }

    /// Starts the launcher, or closes it if it is open: a second launcher closes the first.
    pub(crate) fn start_launcher(&mut self) {
        let program = self
            .launcher_program
            .clone()
            .unwrap_or_else(|| program(|key| std::env::var_os(key)));
        self.start_shell_program(program);
    }

    /// Opens the applications view, or closes it if it is open, the same way (lot D-41).
    pub(crate) fn start_applications(&mut self) {
        self.start_shell_program(program_named(APPLICATIONS, |key| std::env::var_os(key)));
    }

    /// Opens the clipboard history panel, or closes it if it is open, the same way (lot D-42).
    pub(crate) fn start_clipboard_panel(&mut self) {
        self.start_shell_program(program_named(CLIPBOARD, |key| std::env::var_os(key)));
    }

    /// Starts `program` with the compositor's Wayland socket and X11 display.
    fn start_shell_program(&mut self, program: PathBuf) {
        let mut command = Command::new(&program);
        if let Some(socket) = &self.socket_name {
            command.env("WAYLAND_DISPLAY", socket);
        }
        match self.x_display {
            Some(n) => command.env("DISPLAY", format!(":{n}")),
            None => command.env_remove("DISPLAY"),
        };
        match command.spawn() {
            Ok(mut child) => {
                tracing::info!(program = %program.display(), "started");
                // A thread waits for it, so that it does not stay a zombie.
                let spawned = std::thread::Builder::new()
                    .name("shell-program".into())
                    .spawn(move || {
                        if let Err(err) = child.wait() {
                            tracing::warn!("waiting for a shell program: {err}");
                        }
                    });
                if let Err(err) = spawned {
                    tracing::warn!("no thread to wait for a shell program: {err}");
                }
            }
            Err(err) => tracing::warn!(program = %program.display(), "cannot start: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_is_found_beside_the_programs_then_in_the_path() {
        let dir = std::env::temp_dir().join(format!("launcher-bin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let var = |dir: &Path| {
            let dir = dir.as_os_str().to_owned();
            move |key: &str| (key == BIN_VAR).then(|| dir.clone())
        };
        // Not there: the PATH.
        assert_eq!(program(var(&dir)), PathBuf::from(PROGRAM));
        assert_eq!(program(|_| None), PathBuf::from(PROGRAM));
        std::fs::write(dir.join(PROGRAM), b"").unwrap();
        assert_eq!(program(var(&dir)), dir.join(PROGRAM));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
