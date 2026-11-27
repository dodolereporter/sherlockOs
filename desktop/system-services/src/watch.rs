//! Changes made elsewhere (lot D-35d): a key, another application. The quick settings of the bar
//! follow them while their panel is open. Nothing is polled: the sound through `pactl subscribe`,
//! watched and started again when it dies; the screen brightness through the kernel's notice on
//! `actual_brightness` (`poll(2)`, which sysfs wakes up), and inotify for a plain file.
//!
//! Each watcher runs on a thread of its own and tells `changed` of each change; it stops once
//! `wanted` says nobody listens any more, which it asks at least every `CHECK`.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backlight;

/// Longest wait before a watcher asks whether it is still wanted.
pub const CHECK: Duration = Duration::from_secs(1);
/// First delay before `pactl subscribe` starts again after it died; doubled each time it dies
/// at once, up to `RESTART_MAX`.
pub const RESTART: Duration = Duration::from_millis(500);
pub const RESTART_MAX: Duration = Duration::from_secs(30);
/// Lines of `pactl subscribe` closer than this are one change: a slider dragged elsewhere.
const BURST: Duration = Duration::from_millis(40);

/// The lines of `pactl subscribe` that tell the default output may have changed: an output, or
/// the server (the default output chosen), not the streams of the applications.
pub fn sound_changed(line: &str) -> bool {
    line.contains(" on sink #") || line.contains(" on server")
}

/// Sleeps `delay`, waking every `CHECK`; `false` once nobody wants the watcher.
fn rest(delay: Duration, wanted: &impl Fn() -> bool) -> bool {
    let end = Instant::now() + delay;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if !wanted() {
            return false;
        }
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(CHECK));
    }
}

/// Starts `program subscribe`, killed by the kernel when the thread that started it ends.
fn subscribe(program: &OsString) -> io::Result<(Child, Receiver<String>)> {
    let mut command = Command::new(program);
    command
        .arg("subscribe")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // SAFETY: the closure runs in the child between fork and exec, and only calls prctl(2),
    // which is async-signal-safe, then builds its error from errno without allocating.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("no standard output"))?;
    let (sender, lines) = mpsc::channel();
    std::thread::Builder::new()
        .name(String::from("pactl-lines"))
        .spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    return;
                }
            }
        })?;
    Ok((child, lines))
}

/// Watches the sound with `program subscribe` (`pactl`, or a stand-in for the tests). The
/// command is started again when it dies, and `changed` is told then too: something may have
/// changed meanwhile.
pub fn sound(
    program: OsString,
    mut changed: impl FnMut() -> bool + Send + 'static,
    wanted: impl Fn() -> bool + Send + 'static,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name(String::from("watch-sound"))
        .spawn(move || {
            let mut delay = RESTART;
            let mut first = true;
            while wanted() {
                let (mut child, lines) = match subscribe(&program) {
                    Ok(started) => started,
                    Err(error) => {
                        tracing::warn!(%error, "cannot follow the sound");
                        if !rest(delay, &wanted) {
                            return;
                        }
                        delay = (delay * 2).min(RESTART_MAX);
                        continue;
                    }
                };
                if !first && !changed() {
                    return;
                }
                first = false;
                let started = Instant::now();
                let goes_on = follow(&lines, &mut changed, &wanted);
                let _ = child.kill();
                let _ = child.wait();
                if !goes_on {
                    return;
                }
                // It ran a while: the next death is a new one.
                if started.elapsed() > RESTART_MAX {
                    delay = RESTART;
                }
                tracing::warn!(?delay, "pactl subscribe stopped, started again");
                if !rest(delay, &wanted) {
                    return;
                }
                delay = (delay * 2).min(RESTART_MAX);
            }
        })
}

/// Tells `changed` of each burst of relevant lines, until the command ends (`true`) or nobody
/// wants the watcher (`false`).
fn follow(
    lines: &Receiver<String>,
    changed: &mut impl FnMut() -> bool,
    wanted: &impl Fn() -> bool,
) -> bool {
    loop {
        match lines.recv_timeout(CHECK) {
            Ok(line) if sound_changed(&line) => {
                while lines.recv_timeout(BURST).is_ok() {}
                if !changed() {
                    return false;
                }
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout) if !wanted() => return false,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return true,
        }
    }
}

/// The file of `light` the kernel notifies: `actual_brightness`, or `brightness` without it.
fn notified(light: &Path) -> PathBuf {
    let actual = light.join("actual_brightness");
    if actual.exists() {
        actual
    } else {
        light.join("brightness")
    }
}

/// An inotify watch on `file`, for a plain file that sysfs does not notify.
fn inotify(file: &Path) -> Option<File> {
    let path = std::ffi::CString::new(file.as_os_str().as_encoded_bytes()).ok()?;
    // SAFETY: inotify_init1 takes no pointer; its result is checked before it is used.
    let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    // SAFETY: `fd` was just opened by us and is owned by nothing else.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let mask = libc::IN_MODIFY | libc::IN_CLOSE_WRITE;
    // SAFETY: `owned` is a live inotify descriptor and `path` a NUL-terminated string that
    // outlives the call.
    let watch = unsafe { libc::inotify_add_watch(owned.as_raw_fd(), path.as_ptr(), mask) };
    (watch >= 0).then(|| File::from(owned))
}

/// Watches the brightness of the backlight `find` reads under `root`. Returns `None` when the
/// machine has none. `changed` is told when the value read differs from the last one.
pub fn brightness(
    root: PathBuf,
    mut changed: impl FnMut() -> bool + Send + 'static,
    wanted: impl Fn() -> bool + Send + 'static,
) -> io::Result<Option<JoinHandle<()>>> {
    let Ok((_, light)) = backlight::locate(&root) else {
        return Ok(None);
    };
    let path = notified(&light);
    let file = File::open(&path)?;
    let mut events = inotify(&path);
    let read = |file: &File| {
        let mut bytes = [0u8; 32];
        let n = file.read_at(&mut bytes, 0).unwrap_or(0);
        bytes[..n].to_vec()
    };
    // Read once, so that sysfs knows what was seen; here rather than on the thread, which may
    // start late: a change made meanwhile would be taken for the first value (lot Q-18b).
    let mut last = read(&file);
    let thread = std::thread::Builder::new()
        .name(String::from("watch-brightness"))
        .spawn(move || {
            let timeout = i32::try_from(CHECK.as_millis()).unwrap_or(1000);
            loop {
                let mut fds = [
                    libc::pollfd {
                        fd: file.as_raw_fd(),
                        events: libc::POLLPRI,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: events.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                // SAFETY: `fds` is a live array of two pollfd for the whole call, and its
                // length is the one given.
                let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout) };
                if !wanted() {
                    return;
                }
                if ready <= 0 {
                    continue;
                }
                if let Some(events) = &mut events {
                    let mut drained = [0u8; 4096];
                    while events.read(&mut drained).is_ok_and(|n| n > 0) {}
                }
                let now = read(&file);
                if now != last {
                    last = now;
                    if !changed() {
                        return;
                    }
                }
            }
        })?;
    Ok(Some(thread))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Waits until `count` reaches `n`, for 5 s at most.
    fn reaches(count: &AtomicUsize, n: usize) -> bool {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if count.load(Ordering::SeqCst) >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn only_the_output_and_the_server_count() {
        assert!(sound_changed("Event 'change' on sink #52"));
        assert!(sound_changed("Event 'change' on server #-1"));
        assert!(!sound_changed("Event 'change' on sink-input #120"));
        assert!(!sound_changed("Event 'new' on client #7"));
    }

    #[test]
    fn a_fake_pactl_subscribe_is_followed_and_started_again() {
        let dir = folder("pactl");
        // Prints a change of the output and one of a stream, then dies; the second time it
        // stays.
        let script = dir.join("pactl");
        let body = format!(
            "#!/bin/sh\nn=$(cat '{d}/runs' 2>/dev/null || echo 0)\necho $((n + 1)) > '{d}/runs'\n\
             echo \"Event 'change' on sink-input #3\"\necho \"Event 'change' on sink #1\"\n\
             if [ \"$n\" -gt 0 ]; then exec sleep 30; fi\n",
            d = dir.display()
        );
        std::fs::write(&script, body).unwrap();
        let chmod = Command::new("chmod").arg("+x").arg(&script).status();
        assert!(chmod.unwrap().success());
        let count = Arc::new(AtomicUsize::new(0));
        let wanted = Arc::new(AtomicBool::new(true));
        let (c, w) = (count.clone(), wanted.clone());
        let thread = sound(
            script.into_os_string(),
            move || c.fetch_add(1, Ordering::SeqCst) < 100,
            move || w.load(Ordering::SeqCst),
        )
        .unwrap();
        // Its change, the restart, the change of the second run.
        assert!(reaches(&count, 3), "{}", count.load(Ordering::SeqCst));
        let runs = std::fs::read_to_string(dir.join("runs")).unwrap();
        assert_eq!(runs.trim(), "2");
        // Not wanted any more: it stops within `CHECK`, and so does its command.
        wanted.store(false, Ordering::SeqCst);
        thread.join().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fake_backlight_is_followed_without_polling() {
        let dir = folder("sysfs");
        let light = dir.join("amdgpu_bl1");
        std::fs::create_dir_all(&light).unwrap();
        std::fs::write(light.join("brightness"), "051").unwrap();
        std::fs::write(light.join("actual_brightness"), "051").unwrap();
        std::fs::write(light.join("max_brightness"), "255").unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let wanted = Arc::new(AtomicBool::new(true));
        let (c, w) = (count.clone(), wanted.clone());
        let thread = brightness(
            dir.clone(),
            move || c.fetch_add(1, Ordering::SeqCst) < 100,
            move || w.load(Ordering::SeqCst),
        )
        .unwrap()
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        // As the kernel changes it: in one write. `std::fs::write` truncates first, and the
        // watcher could read the empty file in between, a change of its own (lot Q-18b).
        let set = |value: &str| {
            use std::os::unix::fs::FileExt as _;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(light.join("actual_brightness"))
                .unwrap();
            file.write_all_at(value.as_bytes(), 0).unwrap();
        };
        set("128");
        assert!(reaches(&count, 1));
        // The same value written again is no change.
        set("128");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(count.load(Ordering::SeqCst), 1);
        wanted.store(false, Ordering::SeqCst);
        thread.join().unwrap();
        // No backlight, nothing to watch.
        let none = brightness(dir.join("none"), || true, || true).unwrap();
        assert!(none.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
