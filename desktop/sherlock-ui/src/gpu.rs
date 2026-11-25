//! Choice of the renderer of the shell programs, before iced starts (ADR 0018).
//!
//! iced picks its renderer and its graphics adapter from environment variables only. Left
//! alone, wgpu asks every Vulkan driver of the machine for its adapters, and on a laptop with
//! a dedicated NVIDIA card that alone wakes the card up. So a program decides first, from
//! `/sys` (which never wakes a card): Vulkan through the Mesa driver of an AMD chip, and that
//! driver only, or software rendering (tiny-skia). The NVIDIA card is never asked anything.
//!
//! ```no_run
//! let choice = sherlock_ui::gpu::Choice::detect();
//! // SAFETY: first thing in `main`, before any other thread exists.
//! unsafe { choice.apply() };
//! ```
//!
//! If wgpu still finds no adapter (a compositor that cannot take GPU buffers, a broken
//! driver), iced falls back to tiny-skia by itself.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// PCI vendor of AMD.
const AMD: u16 = 0x1002;
/// Kernel driver of the AMD chips that Mesa's Vulkan driver (RADV) runs on.
const AMDGPU: &str = "amdgpu";
/// File name prefix of the manifest of RADV (`radeon_icd.json`, `radeon_icd.x86_64.json`).
const RADV_MANIFEST: &str = "radeon_icd";

/// A graphics chip, as `/sys/class/drm/card*/device` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub vendor: u16,
    /// Name of the kernel driver bound to the chip (`amdgpu`, `nvidia`, `i915`…).
    pub driver: String,
    /// The firmware showed its boot screen on this chip: on a laptop, the integrated one.
    pub boot_vga: bool,
}

/// What the program renders with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// `ICED_BACKEND` is already set: the user chose, nothing is changed.
    Inherited,
    /// Software rendering, with tiny-skia.
    Software(Reason),
    /// Vulkan, with only these driver manifests visible to the Vulkan loader.
    Vulkan { manifests: Vec<PathBuf> },
}

/// Why the program renders in software.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// No AMD chip driven by `amdgpu`: no GPU, or only chips not tried yet (ADR 0018).
    NoAmdChip,
    /// An AMD chip, but no manifest of Mesa's Vulkan driver.
    NoVulkanDriver,
}

impl Choice {
    /// Decides from the chips and the Vulkan driver manifests found, unless `ICED_BACKEND`
    /// was set by the user.
    pub fn choose(iced_backend_set: bool, chips: &[Chip], manifests: &[PathBuf]) -> Self {
        if iced_backend_set {
            return Self::Inherited;
        }
        if !chips.iter().any(|c| c.vendor == AMD && c.driver == AMDGPU) {
            return Self::Software(Reason::NoAmdChip);
        }
        let manifests: Vec<PathBuf> = manifests
            .iter()
            .filter(|path| is_radv_manifest(path))
            .cloned()
            .collect();
        if manifests.is_empty() {
            return Self::Software(Reason::NoVulkanDriver);
        }
        Self::Vulkan { manifests }
    }

    /// Reads the machine: `ICED_BACKEND`, the chips in `/sys/class/drm`, the manifests in the
    /// Vulkan loader's folders.
    pub fn detect() -> Self {
        let set = std::env::var_os("ICED_BACKEND").is_some();
        if set {
            return Self::Inherited;
        }
        Self::choose(false, &chips(Path::new("/sys/class/drm")), &manifests())
    }

    /// The variables to set for this choice.
    pub fn variables(&self) -> Vec<(&'static str, OsString)> {
        match self {
            Self::Inherited => Vec::new(),
            Self::Software(_) => vec![("ICED_BACKEND", "tiny-skia".into())],
            Self::Vulkan { manifests } => vec![
                // wgpu first, tiny-skia if it finds no adapter.
                ("ICED_BACKEND", "wgpu,tiny-skia".into()),
                // Vulkan only: wgpu's OpenGL backend would load every EGL vendor library,
                // NVIDIA's included.
                ("WGPU_BACKEND", "vulkan".into()),
                ("WGPU_POWER_PREF", "low".into()),
                // The loader sees Mesa's AMD driver and nothing else.
                ("VK_DRIVER_FILES", join(manifests)),
            ],
        }
    }

    /// Sets the variables of this choice for the whole process, and remembers the previous
    /// values for the commands it will start (`for_child`).
    ///
    /// # Safety
    ///
    /// Changes the process environment (`std::env::set_var`): it must be called before any
    /// other thread is started, first thing in `main`, and only once.
    pub unsafe fn apply(&self) {
        let variables = self.variables();
        let previous = variables
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        for (name, value) in variables {
            // SAFETY: the caller guarantees that no other thread exists yet.
            unsafe { std::env::set_var(name, value) };
        }
        let _ = PREVIOUS.set(previous);
    }
}

/// Values of the variables before `apply`, given back to child processes.
static PREVIOUS: OnceLock<Vec<(&'static str, Option<OsString>)>> = OnceLock::new();

/// Gives `command` the environment the program was started with: a program launched from
/// the shell (an application, a game) must not inherit the shell's choice of renderer.
pub fn for_child(command: &mut Command) -> &mut Command {
    restore(command, PREVIOUS.get().map_or(&[], Vec::as_slice))
}

fn restore<'c>(
    command: &'c mut Command,
    previous: &[(&'static str, Option<OsString>)],
) -> &'c mut Command {
    for (name, value) in previous {
        match value {
            Some(value) => command.env(name, value),
            None => command.env_remove(name),
        };
    }
    command
}

fn is_radv_manifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(RADV_MANIFEST) && name.ends_with(".json"))
}

fn join(paths: &[PathBuf]) -> OsString {
    let mut joined = OsString::new();
    for (i, path) in paths.iter().enumerate() {
        if i > 0 {
            joined.push(":");
        }
        joined.push(path);
    }
    joined
}

/// The chips under `drm` (`/sys/class/drm`): `card0`, `card1`… without their connectors.
/// Only attributes kept by the kernel are read, which never wakes a sleeping card (unlike
/// `lspci`, which reads the configuration space).
pub fn chips(drm: &Path) -> Vec<Chip> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut chips: Vec<Chip> = entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.strip_prefix("card")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|entry| chip(&entry.path().join("device")))
        .collect();
    chips.sort_by_key(|c| !c.boot_vga);
    chips
}

fn chip(device: &Path) -> Option<Chip> {
    let vendor = std::fs::read_to_string(device.join("vendor")).ok()?;
    let vendor = u16::from_str_radix(vendor.trim().trim_start_matches("0x"), 16).ok()?;
    let driver = std::fs::read_link(device.join("driver")).ok()?;
    let driver = driver.file_name()?.to_string_lossy().into_owned();
    let boot_vga = std::fs::read_to_string(device.join("boot_vga")).is_ok_and(|v| v.trim() == "1");
    Some(Chip {
        vendor,
        driver,
        boot_vga,
    })
}

/// The Vulkan driver manifests in the loader's folders (`$XDG_DATA_DIRS`, `/etc`).
fn manifests() -> Vec<PathBuf> {
    let data = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| String::from("/usr/local/share:/usr/share"));
    let folders = data
        .split(':')
        .map(PathBuf::from)
        .chain([PathBuf::from("/etc")])
        .map(|dir| dir.join("vulkan/icd.d"));
    let mut found = Vec::new();
    for folder in folders {
        if let Ok(entries) = std::fs::read_dir(&folder) {
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            files.sort();
            found.extend(files);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(vendor: u16, driver: &str, boot_vga: bool) -> Chip {
        Chip {
            vendor,
            driver: String::from(driver),
            boot_vga,
        }
    }

    /// The manifests of the reference laptop (Debian), plus Arch's naming.
    fn manifests() -> Vec<PathBuf> {
        [
            "/usr/share/vulkan/icd.d/intel_icd.json",
            "/usr/share/vulkan/icd.d/lvp_icd.json",
            "/usr/share/vulkan/icd.d/nouveau_icd.json",
            "/usr/share/vulkan/icd.d/nvidia_icd.json",
            "/usr/share/vulkan/icd.d/radeon_icd.json",
            "/usr/share/vulkan/icd.d/radeon_icd.x86_64.json",
        ]
        .map(PathBuf::from)
        .to_vec()
    }

    #[test]
    fn the_reference_laptop_renders_on_the_amd_chip_only() {
        let chips = [chip(0x10de, "nvidia", false), chip(AMD, AMDGPU, true)];
        let choice = Choice::choose(false, &chips, &manifests());
        let Choice::Vulkan { manifests } = &choice else {
            panic!("expected Vulkan, got {choice:?}");
        };
        // Neither NVIDIA's driver nor nouveau's (NVK) nor the software one: only RADV.
        assert_eq!(
            manifests,
            &[
                PathBuf::from("/usr/share/vulkan/icd.d/radeon_icd.json"),
                PathBuf::from("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json"),
            ]
        );
        let variables = choice.variables();
        let get = |name| variables.iter().find(|(n, _)| *n == name).map(|(_, v)| v);
        assert_eq!(get("WGPU_BACKEND"), Some(&OsString::from("vulkan")));
        assert_eq!(get("ICED_BACKEND"), Some(&OsString::from("wgpu,tiny-skia")));
        assert_eq!(
            get("VK_DRIVER_FILES"),
            Some(&OsString::from(
                "/usr/share/vulkan/icd.d/radeon_icd.json:/usr/share/vulkan/icd.d/radeon_icd.x86_64.json"
            ))
        );
    }

    #[test]
    fn without_an_amd_chip_rendering_stays_in_software() {
        // No GPU at all (CI), NVIDIA alone, nouveau, an Intel chip not tried yet.
        for chips in [
            vec![],
            vec![chip(0x10de, "nvidia", true)],
            vec![chip(0x10de, "nouveau", true)],
            vec![chip(0x8086, "i915", true)],
            // An AMD card without its kernel driver.
            vec![chip(AMD, "vfio-pci", false)],
        ] {
            let choice = Choice::choose(false, &chips, &manifests());
            assert_eq!(choice, Choice::Software(Reason::NoAmdChip), "{chips:?}");
            assert_eq!(
                choice.variables(),
                vec![("ICED_BACKEND", OsString::from("tiny-skia"))]
            );
        }
    }

    #[test]
    fn without_mesa_vulkan_rendering_stays_in_software() {
        let chips = [chip(AMD, AMDGPU, true)];
        let others: Vec<PathBuf> = manifests()
            .into_iter()
            .filter(|p| !is_radv_manifest(p))
            .collect();
        assert_eq!(
            Choice::choose(false, &chips, &others),
            Choice::Software(Reason::NoVulkanDriver)
        );
    }

    #[test]
    fn a_renderer_set_by_the_user_wins() {
        let chips = [chip(AMD, AMDGPU, true)];
        let choice = Choice::choose(true, &chips, &manifests());
        assert_eq!(choice, Choice::Inherited);
        assert!(choice.variables().is_empty());
    }

    #[test]
    fn children_get_the_environment_back() {
        let mut command = Command::new("true");
        let previous = [
            ("ICED_BACKEND", None),
            ("VK_DRIVER_FILES", Some(OsString::from("/elsewhere.json"))),
        ];
        restore(&mut command, &previous);
        let envs: Vec<_> = command.get_envs().collect();
        assert!(envs.contains(&(std::ffi::OsStr::new("ICED_BACKEND"), None)));
        assert!(envs.contains(&(
            std::ffi::OsStr::new("VK_DRIVER_FILES"),
            Some(std::ffi::OsStr::new("/elsewhere.json"))
        )));
    }

    #[test]
    fn chips_are_read_from_sysfs_without_connectors() -> std::io::Result<()> {
        let root = std::env::temp_dir().join(format!("sherlock-ui-gpu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (card, vendor, driver, boot) in [
            ("card0", "0x1002", "amdgpu", "1"),
            ("card1", "0x10de", "nvidia", "0"),
        ] {
            let device = root.join(card).join("device");
            std::fs::create_dir_all(&device)?;
            std::fs::write(device.join("vendor"), format!("{vendor}\n"))?;
            std::fs::write(device.join("boot_vga"), format!("{boot}\n"))?;
            let bound = root.join("drivers").join(driver);
            std::fs::create_dir_all(&bound)?;
            std::os::unix::fs::symlink(&bound, device.join("driver"))?;
        }
        // A connector has no `device/vendor` of its own here, and must be skipped by name.
        std::fs::create_dir_all(root.join("card0-eDP-1"))?;
        let found = chips(&root);
        std::fs::remove_dir_all(&root)?;
        assert_eq!(
            found,
            vec![chip(AMD, AMDGPU, true), chip(0x10de, "nvidia", false)]
        );
        Ok(())
    }
}
