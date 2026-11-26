//! The start script of the real session (lot D-35b), run with fake programs: a fake compositor
//! that records what it is given and launches the shell part as the real one does, fake shell
//! programs, a fake keyring and polkit agent, and a fake `dbus-update-activation-environment`.
//! Everything runs under `env -i`, with a home and XDG folders of the test's own, and a `PATH`
//! that holds only the fakes and links to the few system tools the script needs: no real
//! service, bus, keyring or agent of the machine is ever reached.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/session/sherlockos-session");

    /// The system tools the script uses, linked into the test's `PATH`.
    const TOOLS: [&str; 12] = [
        "sh", "dirname", "readlink", "mkdir", "mv", "awk", "date", "wc", "cat", "sleep", "env",
        "kill",
    ];

    struct Session {
        dir: PathBuf,
    }

    fn write(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
        let status = Command::new("chmod").arg("+x").arg(path).status().unwrap();
        assert!(status.success());
    }

    impl Session {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("session-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            for sub in ["bin", "path", "home", "run", "out"] {
                std::fs::create_dir_all(dir.join(sub)).unwrap();
            }
            let o = dir.join("out");
            let o = o.display();
            // The compositor records its arguments and environment, then runs the shell part
            // with a display, long enough for the shell programs to start.
            write(
                &dir.join("bin/compositor"),
                &format!(
                    "#!/bin/sh\necho \"$@\" > '{o}/args'\nenv > '{o}/env'\n\
                     [ \"$1\" = --xwayland ] && shift\n\
                     WAYLAND_DISPLAY=wayland-test \"$@\" &\nsleep 2\n"
                ),
            );
            for program in ["bar", "dock", "notifications"] {
                write(
                    &dir.join("bin").join(program),
                    &format!("#!/bin/sh\necho > '{o}/{program}'\nsleep 3\n"),
                );
            }
            let path = dir.join("path");
            for tool in TOOLS {
                let found = Command::new("sh")
                    .args(["-c", &format!("command -v {tool}")])
                    .output()
                    .unwrap();
                let found = String::from_utf8(found.stdout).unwrap();
                if found.trim().starts_with('/') {
                    std::os::unix::fs::symlink(found.trim(), path.join(tool)).unwrap();
                }
            }
            write(
                &path.join("dbus-update-activation-environment"),
                &format!("#!/bin/sh\necho \"$@\" > '{o}/activation'\n"),
            );
            Self { dir }
        }

        /// A fake program in the test's `PATH`.
        fn fake(&self, name: &str, body: &str) {
            write(&self.dir.join("path").join(name), body);
        }

        /// Runs the script with `vars`, and returns its log.
        fn run(&self, vars: &[(&str, String)]) -> String {
            let d = &self.dir;
            let mut command = Command::new("env");
            command
                .arg("-i")
                .arg(format!("PATH={}", d.join("path").display()))
                .arg(format!("HOME={}", d.join("home").display()))
                .arg(format!("XDG_RUNTIME_DIR={}", d.join("run").display()))
                .arg(format!("SHERLOCKOS_BIN={}", d.join("bin").display()))
                .arg(format!("SHERLOCKOS_DATA={}", d.join("data").display()))
                .arg("SHERLOCKOS_SESSION_BUS=shared");
            for (key, value) in vars {
                command.arg(format!("{key}={value}"));
            }
            let status = command.arg("sh").arg(SCRIPT).status().unwrap();
            assert!(status.success(), "{status}");
            let log = d.join("home/.local/state/sherlockos/session.log");
            std::fs::read_to_string(log).unwrap_or_default()
        }

        fn out(&self, name: &str) -> String {
            std::fs::read_to_string(self.dir.join("out").join(name)).unwrap_or_default()
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn the_session_starts_xwayland_its_keyring_and_its_polkit_agent() {
        let s = Session::new("full");
        let o = s.dir.join("out");
        s.fake(
            "gnome-keyring-daemon",
            &format!(
                "#!/bin/sh\necho \"$@\" > '{}/keyring'\n\
                 echo GNOME_KEYRING_CONTROL=/run/test/keyring\n",
                o.display()
            ),
        );
        let agent = s.dir.join("agent");
        write(
            &agent,
            &format!("#!/bin/sh\necho > '{}/agent'\nsleep 3\n", o.display()),
        );
        let agents = format!(
            "/nowhere/polkit-kde-authentication-agent-1 {}",
            agent.display()
        );
        let log = s.run(&[("SHERLOCKOS_POLKIT_AGENTS", agents)]);
        assert_eq!(s.out("args").split_whitespace().next(), Some("--xwayland"));
        assert_eq!(s.out("keyring").trim(), "--start --components=secrets");
        let env = s.out("env");
        assert!(
            env.contains("GNOME_KEYRING_CONTROL=/run/test/keyring"),
            "{env}"
        );
        assert!(log.contains("keyring: gnome-keyring-daemon"), "{log}");
        let started = format!("polkit agent: {}", agent.display());
        assert!(log.contains(&started), "{log}");
        assert!(o.join("agent").exists(), "{log}");
        assert!(o.join("bar").exists(), "{log}");
        // The portals learn the name of the session and where its setting is.
        let activation = s.out("activation");
        assert!(activation.starts_with("--systemd "), "{activation}");
        for var in ["WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP", "XDG_DATA_DIRS"] {
            assert!(activation.contains(var), "{activation}");
        }
        assert!(env.contains("XDG_CURRENT_DESKTOP=SherlockOS"), "{env}");
        let data = format!("XDG_DATA_DIRS={}:", s.dir.join("data").display());
        assert!(env.contains(&data), "{env}");
    }

    #[test]
    fn without_agent_nor_keyring_the_session_says_so_and_goes_on() {
        let s = Session::new("bare");
        let log = s.run(&[
            (
                "SHERLOCKOS_POLKIT_AGENTS",
                String::from("/nowhere/agent absent-agent"),
            ),
            ("SHERLOCKOS_XWAYLAND", String::from("0")),
            // No PAM service for the lock screen (lot D-25a).
            ("SHERLOCKOS_PAM_DIR", String::from("/nowhere/pam.d")),
        ]);
        assert!(log.contains("warning: no polkit agent installed"), "{log}");
        assert!(
            log.contains("warning: /nowhere/pam.d/sherlockos-lock is missing"),
            "{log}"
        );
        assert!(log.contains("keyring: none"), "{log}");
        assert!(!s.out("args").contains("--xwayland"), "{}", s.out("args"));
        assert!(s.dir.join("out/bar").exists(), "{log}");
    }
}
