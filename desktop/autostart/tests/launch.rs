//! `sherlockos-autostart` run for real, under `env -i`, on temporary folders: the entries start
//! their programs, without a shell, the delayed one later, and the others are left out.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        process::Command,
        time::{Duration, Instant},
    };

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_entries_start_their_programs() {
        let dir = std::env::temp_dir().join(format!("autostart-launch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = dir.join("out");
        std::fs::create_dir_all(&out).unwrap();
        // A program that writes its arguments: `$` and `;` reach it as they are, no shell runs.
        let program = dir.join("bin/record");
        write(
            &program,
            &format!(
                "#!/bin/sh\nprintf '%s|' \"$@\" > \"{}/$1\"\n",
                out.display()
            ),
        );
        let status = Command::new("chmod")
            .arg("+x")
            .arg(&program)
            .status()
            .unwrap();
        assert!(status.success());
        let entry = |args: &str, extra: &str| {
            format!(
                "[Desktop Entry]\nType=Application\nName=Test\nExec={} {args}\n{extra}",
                program.display()
            )
        };
        write(
            &dir.join("etc/autostart/now.desktop"),
            &entry("now \"$HOME;x\" %U", ""),
        );
        write(
            &dir.join("etc/autostart/later.desktop"),
            &entry("later", "X-GNOME-Autostart-Delay=1\n"),
        );
        write(
            &dir.join("etc/autostart/kde.desktop"),
            &entry("kde", "OnlyShowIn=KDE;\n"),
        );
        write(
            &dir.join("home/.config/autostart/off.desktop"),
            &entry("off", "Hidden=true\n"),
        );
        write(&dir.join("etc/autostart/off.desktop"), &entry("off", ""));
        let started = Instant::now();
        let status = Command::new("env")
            .arg("-i")
            .arg(format!("HOME={}", dir.join("home").display()))
            .arg(format!("XDG_CONFIG_DIRS={}", dir.join("etc").display()))
            .arg("XDG_CURRENT_DESKTOP=SherlockOS")
            .arg("PATH=/usr/bin:/bin")
            .arg(env!("CARGO_BIN_EXE_sherlockos-autostart"))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(started.elapsed() >= Duration::from_secs(1), "the delay");
        // The programs run on after it: wait for their files.
        let read = |name: &str| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Ok(text) = std::fs::read_to_string(out.join(name))
                    && !text.is_empty()
                {
                    return text;
                }
                assert!(Instant::now() < deadline, "{name} not started");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        assert_eq!(read("now"), "now|$HOME;x|");
        assert_eq!(read("later"), "later|");
        std::thread::sleep(Duration::from_millis(200));
        assert!(!out.join("kde").exists() && !out.join("off").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
