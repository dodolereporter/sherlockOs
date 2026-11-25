//! What the compositor and Xwayland may load, so that the NVIDIA card stays asleep.
//!
//! - **EGL.** libglvnd loads every EGL vendor library it finds, NVIDIA's first on Debian
//!   (`10_nvidia.json`). The real session restricts it to Mesa's (`__EGL_VENDOR_LIBRARY_FILENAMES`),
//!   for itself and for what it launches, unless the user set the variable.
//! - **GBM.** libgbm loads the backend named after the driver of the card it is given
//!   (`nvidia-drm_gbm.so` for the NVIDIA card) before Mesa's own. Xwayland is given a folder of
//!   backends without NVIDIA's (`GBM_BACKENDS_PATH`).
//!
//! Measured with `tests/xwayland.rs` on the reference laptop: with both, Xwayland 24.1.6 no
//! longer loads NVIDIA's GBM backend, but it still opens the NVIDIA card's node at startup, with
//! glamor or without. So the real session starts XWayland only on request (`--xwayland`).

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// libglvnd's variable: the EGL vendor libraries it may load.
pub const EGL_VENDORS: &str = "__EGL_VENDOR_LIBRARY_FILENAMES";

/// libgbm's variable: the folders of its backends.
pub const GBM_BACKENDS: &str = "GBM_BACKENDS_PATH";

/// Folders of libglvnd's EGL vendor files.
const GLVND_DIRS: [&str; 2] = ["/etc/glvnd/egl_vendor.d", "/usr/share/glvnd/egl_vendor.d"];

/// Folders of libgbm's backends, by distribution (Debian, Fedora, Arch).
const GBM_DIRS: [&str; 3] = [
    "/usr/lib/x86_64-linux-gnu/gbm",
    "/usr/lib64/gbm",
    "/usr/lib/gbm",
];

/// Lets libglvnd load Mesa's EGL only, unless the user set `__EGL_VENDOR_LIBRARY_FILENAMES`.
/// The commands the compositor launches inherit it.
///
/// # Safety
///
/// Changes the process environment: the process must have a single thread.
pub unsafe fn restrict_egl_to_mesa() {
    let dirs = GLVND_DIRS.map(Path::new);
    if std::env::var_os(EGL_VENDORS).is_none()
        && let Some(mesa) = mesa_vendor(&dirs)
    {
        // SAFETY: the caller guarantees that the process has a single thread.
        unsafe { std::env::set_var(EGL_VENDORS, mesa) };
    }
}

/// Mesa's EGL vendor file (`50_mesa.json`) in the first of `dirs` that has one.
fn mesa_vendor(dirs: &[&Path]) -> Option<PathBuf> {
    dirs.iter().find_map(|dir| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .collect();
        files.sort();
        files
            .into_iter()
            .find(|path| name(path).is_some_and(|n| n.contains("mesa") && n.ends_with(".json")))
    })
}

fn name(path: &Path) -> Option<&str> {
    path.file_name().and_then(|n| n.to_str())
}

/// Fills `into` with links to the GBM backends of `dirs`, NVIDIA's left out (and removed if
/// an older run linked it).
fn link_backends(dirs: &[PathBuf], into: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(into)?;
    for entry in std::fs::read_dir(into)?.flatten() {
        if name(&entry.path()).is_some_and(|n| n.starts_with("nvidia")) {
            std::fs::remove_file(entry.path())?;
        }
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file) = name(&path) else {
                continue;
            };
            let link = into.join(file);
            let linked = link.symlink_metadata().is_ok();
            if file.starts_with("nvidia") || !file.ends_with("_gbm.so") || linked {
                continue;
            }
            std::os::unix::fs::symlink(&path, link)?;
        }
    }
    Ok(())
}

/// The folder of GBM backends without NVIDIA's, made in the user's runtime folder (private to
/// them): `$XDG_RUNTIME_DIR/nom-os/gbm-backends`. `None` without a runtime folder.
pub fn backends_without_nvidia(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let into = PathBuf::from(var("XDG_RUNTIME_DIR")?).join("nom-os/gbm-backends");
    let dirs: Vec<PathBuf> = match var(GBM_BACKENDS) {
        Some(path) => std::env::split_paths(&path).collect(),
        None => GBM_DIRS.map(PathBuf::from).to_vec(),
    };
    match link_backends(&dirs, &into) {
        Ok(()) => Some(into),
        Err(err) => {
            tracing::warn!(folder = %into.display(), "GBM backends for Xwayland: {err}");
            None
        }
    }
}

/// Environment of Xwayland, which smithay starts with an empty one (`PATH` and
/// `XDG_RUNTIME_DIR` aside): the EGL vendors of the compositor, and the GBM backends without
/// NVIDIA's.
pub fn xwayland_env(
    var: impl Fn(&str) -> Option<OsString>,
    gbm: Option<PathBuf>,
) -> Vec<(&'static str, OsString)> {
    let egl = var(EGL_VENDORS).map(|v| (EGL_VENDORS, v));
    let gbm = gbm.map(|dir| (GBM_BACKENDS, dir.into_os_string()));
    egl.into_iter().chain(gbm).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("compositor-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn mesa_is_the_only_egl_vendor() {
        let root = folder("egl");
        let (etc, share) = (root.join("etc"), root.join("share"));
        std::fs::create_dir_all(&etc).unwrap();
        std::fs::create_dir_all(&share).unwrap();
        for file in ["10_nvidia.json", "50_mesa.json"] {
            std::fs::write(share.join(file), "{}").unwrap();
        }
        let found = mesa_vendor(&[&etc, &share, &root.join("missing")]);
        assert_eq!(found, Some(share.join("50_mesa.json")));
        // Without Mesa's, nothing is restricted.
        std::fs::remove_file(share.join("50_mesa.json")).unwrap();
        assert_eq!(mesa_vendor(&[&etc, &share]), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn xwayland_gets_mesa_and_the_gbm_backends_without_nvidia() {
        let root = folder("gbm");
        let system = root.join("lib/gbm");
        std::fs::create_dir_all(&system).unwrap();
        for file in ["dri_gbm.so", "nvidia-drm_gbm.so"] {
            std::fs::write(system.join(file), "").unwrap();
        }
        let runtime = root.join("run");
        let var = |key: &str| match key {
            "XDG_RUNTIME_DIR" => Some(runtime.clone().into_os_string()),
            GBM_BACKENDS => Some(system.clone().into_os_string()),
            EGL_VENDORS => Some(OsString::from("/usr/share/glvnd/egl_vendor.d/50_mesa.json")),
            _ => None,
        };
        // An older run had linked NVIDIA's: it goes away.
        let backends = runtime.join("nom-os/gbm-backends");
        std::fs::create_dir_all(&backends).unwrap();
        std::os::unix::fs::symlink(
            system.join("nvidia-drm_gbm.so"),
            backends.join("nvidia-drm_gbm.so"),
        )
        .unwrap();

        let gbm = backends_without_nvidia(var);
        assert_eq!(gbm.as_deref(), Some(backends.as_path()));
        let mut linked: Vec<_> = std::fs::read_dir(&backends)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        linked.sort();
        assert_eq!(linked, ["dri_gbm.so"]);
        // Done twice, it changes nothing.
        assert_eq!(
            backends_without_nvidia(var).as_deref(),
            Some(backends.as_path())
        );

        let env = xwayland_env(var, gbm);
        assert_eq!(
            env,
            [
                (
                    EGL_VENDORS,
                    OsString::from("/usr/share/glvnd/egl_vendor.d/50_mesa.json")
                ),
                (GBM_BACKENDS, backends.clone().into_os_string()),
            ]
        );
        // Without a runtime folder nor a restriction (nested mode under a bare environment),
        // Xwayland gets nothing more than before.
        assert_eq!(backends_without_nvidia(|_| None), None);
        assert!(xwayland_env(|_| None, None).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
