//! `apply`, then `for_child`, end to end. `apply` changes the environment of the process,
//! which needs a single thread: this test runs without libtest's harness (`harness = false`),
//! in its own process, on the main thread only.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;

use sherlock_ui::gpu::{self, Choice};

fn main() {
    // A test runner that lists the tests first (`cargo nextest`, the CI: lot Q-18b) gets this
    // binary's one test, as libtest would name it, and nothing runs then.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        if !args.iter().any(|a| a == "--ignored") {
            use std::io::Write as _;
            let _ = writeln!(std::io::stdout().lock(), "gpu_apply: test");
        }
        return;
    }
    // SAFETY: no harness, so no other thread exists.
    unsafe {
        std::env::set_var("DRI_PRIME", "1");
        std::env::set_var("__NV_PRIME_RENDER_OFFLOAD", "1");
        std::env::set_var("VK_ADD_DRIVER_FILES", "/added.json");
        std::env::set_var("VK_DRIVER_FILES", "/before.json");
        std::env::remove_var("ICED_BACKEND");
    }
    let radv = "/usr/share/vulkan/icd.d/radeon_icd.json";
    let choice = Choice::Vulkan {
        manifests: vec![PathBuf::from(radv)],
        user_backend: false,
        software: None,
    };
    // SAFETY: as above.
    unsafe { choice.apply() };

    // The process renders with Vulkan on RADV only, with no other GPU brought back in.
    assert_eq!(
        std::env::var("ICED_BACKEND").as_deref(),
        Ok("wgpu,tiny-skia")
    );
    assert_eq!(std::env::var("VK_DRIVER_FILES").as_deref(), Ok(radv));
    for name in [
        "DRI_PRIME",
        "__NV_PRIME_RENDER_OFFLOAD",
        "VK_ADD_DRIVER_FILES",
    ] {
        assert_eq!(std::env::var_os(name), None, "{name}");
    }

    // A program it starts gets the environment the process was started with.
    let mut command = Command::new("true");
    gpu::for_child(&mut command);
    let envs: Vec<_> = command.get_envs().collect();
    assert!(envs.contains(&(OsStr::new("DRI_PRIME"), Some(OsStr::new("1")))));
    assert!(envs.contains(&(
        OsStr::new("__NV_PRIME_RENDER_OFFLOAD"),
        Some(OsStr::new("1"))
    )));
    assert!(envs.contains(&(
        OsStr::new("VK_ADD_DRIVER_FILES"),
        Some(OsStr::new("/added.json"))
    )));
    assert!(envs.contains(&(
        OsStr::new("VK_DRIVER_FILES"),
        Some(OsStr::new("/before.json"))
    )));
    assert!(envs.contains(&(OsStr::new("ICED_BACKEND"), None)));
    assert!(envs.contains(&(OsStr::new("WGPU_BACKEND"), None)));
}
