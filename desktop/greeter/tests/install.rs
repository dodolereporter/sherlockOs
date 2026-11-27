//! `just install-greeter` prints the commands that make the login screen the display manager, and
//! those that go back to SDDM: run here on a fake repository, under `env -i`, it must print them
//! and run nothing as root.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::Command;

    const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/install.sh");

    fn fake_repo(name: &str, built: bool) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("greeter-install-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        if built {
            build(&dir, &dir.join("my repo/target"));
        } else {
            std::fs::create_dir_all(dir.join("my repo/target/release")).unwrap();
        }
        dir
    }

    /// The programs in `target`/release, each with the dep-info file cargo writes beside it,
    /// which names a source of the fake repository, older than the program.
    fn build(dir: &std::path::Path, target: &std::path::Path) {
        let release = target.join("release");
        std::fs::create_dir_all(&release).unwrap();
        let source = dir.join("my repo/desktop/greeter/src/main.rs");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, "fn main() {}\n").unwrap();
        let old = Command::new("touch")
            .args(["-d", "2020-01-01"])
            .arg(&source)
            .status()
            .unwrap();
        assert!(old.success());
        let escaped = |path: &std::path::Path| path.display().to_string().replace(' ', "\\ ");
        for program in ["compositor", "sherlockos-greeter"] {
            let path = release.join(program);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            let status = Command::new("chmod").arg("+x").arg(&path).status().unwrap();
            assert!(status.success());
            let info = format!("{}: {}\n", escaped(&path), escaped(&source));
            std::fs::write(release.join(format!("{program}.d")), info).unwrap();
        }
    }

    fn run(dir: &std::path::Path) -> std::process::Output {
        run_with(dir, None)
    }

    fn run_with(dir: &std::path::Path, target: Option<&std::path::Path>) -> std::process::Output {
        let mut command = Command::new("env");
        command.arg("-i");
        if let Some(target) = target {
            command.arg(format!("CARGO_TARGET_DIR={}", target.display()));
        }
        command
            .arg("PATH=/usr/bin:/bin")
            .arg(format!("HOME={}", dir.join("home").display()))
            .arg("sh")
            .arg(SCRIPT)
            .arg(dir.join("my repo"))
            .output()
            .unwrap()
    }

    #[test]
    fn the_commands_switch_to_greetd_and_back_to_sddm() {
        let dir = fake_repo("ok", true);
        let output = run(&dir);
        assert!(output.status.success());
        let printed = String::from_utf8(output.stdout).unwrap();
        for line in [
            "sudo apt install greetd",
            "command = \"/usr/local/libexec/sherlockos/compositor --greeter -- /usr/local/libexec/sherlockos/sherlockos-greeter\"",
            "user = \"_greetd\"",
            "sudo install -d -o _greetd -g _greetd -m 700 /var/cache/sherlockos-greeter",
            "/usr/local/share/sherlockos/lock/",
            "sudo systemctl disable sddm.service",
            "sudo systemctl enable greetd.service",
            "#   sudo systemctl disable greetd.service",
            "#   sudo systemctl enable sddm.service",
        ] {
            assert!(printed.contains(line), "{line}\n{printed}");
        }
        // The path with a space is quoted.
        assert!(printed.contains("'"), "{printed}");
        // Without the model, the screen shows the still picture: said, not installed.
        assert!(!printed.contains("mascot/sherlock.glb"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_model_is_installed_when_it_was_downloaded() {
        let dir = fake_repo("mascot", true);
        let mascot = dir.join("home/.local/share/sherlockos/mascot");
        std::fs::create_dir_all(&mascot).unwrap();
        std::fs::write(mascot.join("sherlock.glb"), "glTF").unwrap();
        let printed = String::from_utf8(run(&dir).stdout).unwrap();
        assert!(
            printed.contains("/usr/local/share/sherlockos/mascot/sherlock.glb"),
            "{printed}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_build_of_cargo_target_dir_is_installed() {
        let dir = fake_repo("target-dir", false);
        let elsewhere = dir.join("shared target");
        build(&dir, &elsewhere);
        let output = run_with(&dir, Some(&elsewhere));
        assert!(output.status.success(), "{output:?}");
        let printed = String::from_utf8(output.stdout).unwrap();
        assert!(
            printed.contains("shared target/release'/sherlockos-greeter"),
            "{printed}"
        );
        // Without the variable, the build in the repository's target is missing.
        assert!(!run(&dir).status.success());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_older_than_its_sources_is_refused() {
        let dir = fake_repo("stale", true);
        let source = dir.join("my repo/desktop/greeter/src/main.rs");
        std::fs::write(&source, "fn main() { changed() }\n").unwrap();
        let output = run(&dir);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let said = String::from_utf8(output.stderr).unwrap();
        assert!(said.contains("is older than"), "{said}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_built_from_another_checkout_is_refused() {
        let dir = fake_repo("other", true);
        let other = dir.join("other repo");
        // The sources the dep-info names are no longer in the repository given.
        std::fs::create_dir_all(&other).unwrap();
        std::fs::rename(dir.join("my repo/desktop"), other.join("desktop")).unwrap();
        let output = run(&dir);
        assert!(!output.status.success());
        let said = String::from_utf8(output.stderr).unwrap();
        assert!(said.contains("not built from"), "{said}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_the_build_nothing_is_printed() {
        let dir = fake_repo("missing", false);
        let output = run(&dir);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
