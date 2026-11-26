//! Test helpers: a temporary folder removed at the end, the active theme held by one test.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use sherlock_ui::color::{self, Theme};

/// A test that sets the active theme holds it: captures run one at a time.
static THEME: Mutex<()> = Mutex::new(());

/// Makes `which` the active theme until the guard is dropped.
pub fn in_theme(which: Theme) -> MutexGuard<'static, ()> {
    let guard = THEME.lock().unwrap_or_else(PoisonError::into_inner);
    color::set_active(which);
    guard
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Temporary folder owned by one test, removed when it goes out of scope.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("evidence-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temporary folder");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
