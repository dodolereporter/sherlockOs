//! The folder shown, watched with inotify (lot E-09, audit I-4): a file downloaded, made in a
//! terminal or removed by another program shows or goes without F5. The watch runs on a thread
//! of its own; changes are grouped for `QUIET` and reported at most once per `MIN_INTERVAL`, so
//! that a folder that changes all the time (a build, a download) never floods the interface.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use iced::futures::channel::mpsc::{self, UnboundedSender};
use iced::futures::{SinkExt, Stream, StreamExt};

/// Changes closer than this are one change.
pub const QUIET: Duration = Duration::from_millis(100);
/// At most one rereading of the folder in this time.
pub const MIN_INTERVAL: Duration = Duration::from_millis(400);
/// How often the thread checks that it is still wanted.
const POLL: Duration = Duration::from_millis(250);

/// What is watched: entries made, removed, moved in or out, written, changed (mode, times),
/// and the folder itself going away.
const MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CLOSE_WRITE
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// An inotify instance watching one folder; closed when dropped.
struct Watch(libc::c_int);

impl Watch {
    fn new(dir: &Path) -> Option<Self> {
        let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: no pointer is passed; the flags are valid for inotify_init1.
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return None;
        }
        let watch = Self(fd);
        // SAFETY: `fd` is the inotify instance just made, `path` a NUL-terminated string alive
        // for the call, which only reads it.
        let added = unsafe { libc::inotify_add_watch(watch.0, path.as_ptr(), MASK) };
        (added >= 0).then_some(watch)
    }

    /// Waits up to `timeout` for events, and empties them: whether any came.
    fn wait(&self, timeout: Duration) -> bool {
        let mut poll = libc::pollfd {
            fd: self.0,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        // SAFETY: `poll` is one valid pollfd, alive for the call, which writes its `revents`.
        let ready = unsafe { libc::poll(&mut poll, 1, millis) };
        if ready <= 0 {
            return false;
        }
        let mut buffer = [0u8; 4096];
        let mut any = false;
        loop {
            // SAFETY: `buffer` is writable for its whole length, and `fd` is ours.
            let read = unsafe { libc::read(self.0, buffer.as_mut_ptr().cast(), buffer.len()) };
            if read <= 0 {
                break;
            }
            any = true;
        }
        any
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: `fd` is ours, closed once here.
        unsafe { libc::close(self.0) };
    }
}

/// Watches `dir` until `changes` is closed, sending its path at each group of changes.
fn watch(dir: PathBuf, changes: UnboundedSender<PathBuf>) {
    let Some(watch) = Watch::new(&dir) else {
        tracing::debug!(dir = %dir.display(), "folder not watched");
        return;
    };
    let mut last = Instant::now() - MIN_INTERVAL;
    while !changes.is_closed() {
        if !watch.wait(POLL) {
            continue;
        }
        // Gathers what follows within `QUIET`, and keeps to `MIN_INTERVAL`.
        let since = last.elapsed();
        while watch.wait(QUIET) {}
        if since < MIN_INTERVAL {
            std::thread::sleep(MIN_INTERVAL - since);
            while watch.wait(Duration::ZERO) {}
        }
        last = Instant::now();
        if changes.unbounded_send(dir.clone()).is_err() {
            break;
        }
    }
}

/// The folder a subscription watches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Watched(pub PathBuf);

/// The changes of the folder, from a thread of its own, while the stream is kept.
pub fn changes(watched: &Watched) -> impl Stream<Item = PathBuf> + use<> {
    let dir = watched.0.clone();
    iced::stream::channel(4, |mut output: mpsc::Sender<PathBuf>| async move {
        let (sender, mut receiver) = mpsc::unbounded();
        let spawned = std::thread::Builder::new()
            .name("evidence-watch".into())
            .spawn(move || watch(dir, sender));
        if spawned.is_err() {
            return;
        }
        while let Some(dir) = receiver.next().await {
            if output.send(dir).await.is_err() {
                break;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDir;

    #[test]
    fn a_change_made_elsewhere_is_reported_once_grouped() {
        let tmp = TempDir::new("watch");
        let (sender, mut receiver) = mpsc::unbounded();
        let dir = tmp.path().to_path_buf();
        let thread = std::thread::spawn(move || watch(dir, sender));
        std::thread::sleep(Duration::from_millis(100));
        for i in 0..20 {
            std::fs::write(tmp.path().join(format!("f{i}")), "x").expect("write");
        }
        let start = Instant::now();
        let first = loop {
            if let Ok(dir) = receiver.try_recv() {
                break dir;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "no change reported"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(first, tmp.path());
        // Twenty files in a row: one report, or two at most.
        std::thread::sleep(MIN_INTERVAL + QUIET);
        let mut more = 0;
        while receiver.try_recv().is_ok() {
            more += 1;
        }
        assert!(more <= 1, "{more} more reports");
        drop(receiver);
        thread.join().expect("the watch ends once nobody listens");
        assert!(Watch::new(&tmp.path().join("absent")).is_none());
    }
}
