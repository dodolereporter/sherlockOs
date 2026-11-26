//! What the tests of the mascot share: the model file and the Vulkan driver.
//!
//! Both are optional on a developer's machine: without them the tests say why and pass. With
//! `SHERLOCK_3D_REQUIRE=1` (the CI, which downloads the model and installs lavapipe), a
//! missing model or driver fails the test instead.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use sherlock_3d::{Model, locate};
use sherlock_ui::gpu::Choice;

/// Whether a missing model or driver must fail.
pub fn required() -> bool {
    std::env::var_os("SHERLOCK_3D_REQUIRE").is_some_and(|v| v == "1")
}

/// Writes a line for the person running the tests. (The test without libtest's harness is
/// not compiled as a test, where `println!` is allowed.)
pub fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Skips (or fails, see `required`) with `why`.
pub fn skip(why: &str) {
    assert!(!required(), "SHERLOCK_3D_REQUIRE=1: {why}");
    say(&format!("skipped: {why}"));
}

/// The model, or `None` after saying it is missing.
pub fn model() -> Option<Model> {
    let Some(path) = locate::find() else {
        skip("no sherlock.glb (run `just mascot`, or set SHERLOCKOS_MASCOT)");
        return None;
    };
    match Model::open(&path) {
        Ok(model) => Some(model),
        Err(error) => panic!("{}: {error}", path.display()),
    }
}

/// Makes the Vulkan loader see one driver: Mesa's for the AMD chip that drives the screen
/// (the shell's choice, ADR 0018), or else Mesa's software driver, lavapipe. Never another
/// GPU's. Returns which one, or `None` when neither exists.
///
/// # Safety
///
/// Changes the process environment: call it while the process has a single thread.
pub unsafe fn pick_driver() -> Option<&'static str> {
    let choice = Choice::detect();
    if let Choice::Vulkan { .. } = choice {
        // SAFETY: the caller guarantees a single thread.
        unsafe { choice.apply() };
        return Some("Mesa RADV (AMD)");
    }
    let lavapipe = manifests("lvp_icd");
    if lavapipe.is_empty() {
        return None;
    }
    let joined = std::env::join_paths(&lavapipe).ok()?;
    // SAFETY: the caller guarantees a single thread.
    unsafe {
        std::env::set_var("VK_DRIVER_FILES", joined);
        for name in [
            "VK_ADD_DRIVER_FILES",
            "__NV_PRIME_RENDER_OFFLOAD",
            "DRI_PRIME",
        ] {
            std::env::remove_var(name);
        }
    }
    Some("Mesa lavapipe (software)")
}

/// The 64-bit Vulkan manifests whose name starts with `prefix`, in the loader's folders.
fn manifests(prefix: &str) -> Vec<PathBuf> {
    let data = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut found = Vec::new();
    for folder in data.split(':').chain(["/etc"]) {
        let Ok(entries) = std::fs::read_dir(Path::new(folder).join("vulkan/icd.d")) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with(prefix)
                        && n.ends_with(".json")
                        && !n.contains(".i686.")
                        && !n.contains(".i386.")
                })
            })
            .collect();
        files.sort();
        found.extend(files);
    }
    found
}

/// Where the captures go: under `target/`, never in the repository (the mascot's rights).
pub fn captures() -> std::io::Result<PathBuf> {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sherlock-3d");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
