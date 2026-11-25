//! The monitoring commands die with the bar. The test runs the real sources in a helper
//! process (this test binary, started again), with fake `upower`, `nmcli` and `pactl` first in
//! the `PATH`, kills the helper, and checks that none of the fake monitors survives.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use bar::source::battery::UPower;
use bar::source::network::NetworkManager;
use bar::source::sound::Pactl;
use bar::source::{Source, watch};
use iced::futures::StreamExt;
use iced::futures::executor::block_on;

/// Set in the helper's environment: the file where each fake monitor writes its pid.
const PIDS_VAR: &str = "BAR_TEST_PIDS";

/// Each fake prints a plausible answer, or, in monitor mode, writes its pid and sleeps. `exec`
/// keeps the pid of the shell for `sleep`.
const FAKES: [(&str, &str); 3] = [
    (
        "upower",
        r#"if [ "$1" = "--monitor" ]; then echo $$ >> "$BAR_TEST_PIDS"; exec sleep 600; fi
printf '  battery\n    present: yes\n    state: discharging\n    percentage: 50%%\n'"#,
    ),
    (
        "nmcli",
        r#"if [ "$1" = "monitor" ]; then echo $$ >> "$BAR_TEST_PIDS"; exec sleep 600; fi
echo 'wifi:connected:MonWifi'"#,
    ),
    (
        "pactl",
        r#"case "$1" in
subscribe) echo $$ >> "$BAR_TEST_PIDS"; exec sleep 600;;
get-sink-volume) echo 'Volume: front-left: 32768 /  50% / -18.06 dB';;
get-sink-mute) echo 'Mute: no';;
esac"#,
    ),
];

/// In the helper process only: runs the three sources until killed.
#[test]
fn helper_runs_the_sources() {
    if std::env::var_os(PIDS_VAR).is_none() {
        return;
    }
    fn spawn<S: Source>(source: S, name: &'static str) {
        std::thread::spawn(move || block_on(watch(source, name).collect::<Vec<_>>()));
    }
    spawn(UPower::default(), "battery");
    spawn(NetworkManager::default(), "network");
    spawn(Pactl::default(), "sound");
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn write_fakes(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir)?;
    for (name, body) in FAKES {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n"))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

fn read_pids(file: &Path) -> Vec<i32> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect()
}

/// Alive: the process exists and is not a zombie waiting to be reaped.
fn alive(pid: i32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z')),
        Err(_) => false,
    }
}

fn children_die_after(signal: i32, name: &str) -> Result<(), Box<dyn Error>> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("children-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let fakes = dir.join("bin");
    write_fakes(&fakes)?;
    let pids = dir.join("pids");
    let path = format!(
        "{}:{}",
        fakes.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut helper = Command::new(std::env::current_exe()?)
        .args(["--exact", "helper_runs_the_sources", "--nocapture"])
        .env("PATH", path)
        .env(PIDS_VAR, &pids)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(20);
    while read_pids(&pids).len() < FAKES.len() {
        assert!(Instant::now() < deadline, "the fake monitors did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    let monitors = read_pids(&pids);
    assert!(monitors.iter().all(|pid| alive(*pid)));

    let helper_pid = i32::try_from(helper.id())?;
    // SAFETY: kill(2) on the helper we started and have not waited for yet.
    let sent = unsafe { libc::kill(helper_pid, signal) };
    assert_eq!(sent, 0);
    let _ = helper.wait();

    let deadline = Instant::now() + Duration::from_secs(5);
    while monitors.iter().any(|pid| alive(*pid)) {
        assert!(
            Instant::now() < deadline,
            "monitors survived {name}: {monitors:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

#[test]
fn monitors_die_when_the_bar_is_terminated() {
    children_die_after(libc::SIGTERM, "sigterm").unwrap();
}

#[test]
fn monitors_die_when_the_bar_is_killed() {
    children_die_after(libc::SIGKILL, "sigkill").unwrap();
}
