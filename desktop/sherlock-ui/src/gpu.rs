//! Choice of the renderer of the shell programs, before iced starts (ADR 0018).
//!
//! iced picks its renderer and its graphics adapter from environment variables only. Left
//! alone, wgpu asks every Vulkan driver of the machine for its adapters, and on a laptop with
//! a dedicated NVIDIA card that alone wakes the card up. So a program decides first, from
//! `/sys` (which never wakes a card): Vulkan through the driver of the chip that drives the
//! screen (AMD, Intel, NVIDIA's own driver), and that driver only; otherwise Vulkan on the
//! processor (Mesa's lavapipe), which keeps the glass translucent and the animations; and only
//! without lavapipe software rendering with tiny-skia, whose buffers are opaque (lot UI-09). A
//! dedicated card that does not drive the screen is never asked anything.
//!
//! Every program that draws with iced does this first thing in `main` (a test of sherlock-ui
//! checks it for every program of the workspace):
//!
//! ```no_run
//! let choice = sherlock_ui::gpu::Choice::detect();
//! // SAFETY: first statement of `main`, no other thread exists yet.
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
#[cfg(test)]
const AMD: u16 = 0x1002;
/// Kernel driver of the AMD chips that Mesa's Vulkan driver (RADV) runs on.
#[cfg(test)]
const AMDGPU: &str = "amdgpu";

/// The kernel drivers of the chips the shell renders on, by family, and the file name prefixes
/// of the manifests of their Vulkan drivers (`radeon_icd.x86_64.json`…): RADV for AMD, ANV and
/// HASVK (older generations) for Intel, NVIDIA's own driver (lot UI-09). nouveau's (NVK) is
/// left out: too young for every card it would be asked to run.
const FAMILIES: [(&[&str], &[&str]); 3] = [
    (&["amdgpu"], &["radeon_icd"]),
    (&["i915", "xe"], &["intel_icd", "intel_hasvk_icd"]),
    (&["nvidia"], &["nvidia_icd"]),
];
/// Mesa's Vulkan driver on the processor: rendering without a usable chip (lot UI-09).
const LAVAPIPE: &[&str] = &["lvp_icd"];
/// Manifests of 32-bit libraries (Arch names them `radeon_icd.i686.json`): never for this
/// process.
const OTHER_ARCHES: [&str; 2] = [".i686.", ".i386."];

/// Variables that would bring another GPU back in, unset by `apply` and given back to child
/// processes: a driver added to the loader's list, the PRIME offload that enables NVIDIA's
/// implicit Vulkan layer (which loads `libGLX_nvidia.so.0`), Mesa's choice of another GPU.
const NEUTRALISED: [&str; 3] = [
    "VK_ADD_DRIVER_FILES",
    "__NV_PRIME_RENDER_OFFLOAD",
    "DRI_PRIME",
];

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
    /// `ICED_BACKEND` set by the user without wgpu, or with wgpu on a machine where the GPU
    /// cannot be narrowed down: nothing is changed.
    Inherited,
    /// Software rendering, with tiny-skia.
    Software(Reason),
    /// Vulkan, with only these driver manifests visible to the Vulkan loader. `user_backend`:
    /// `ICED_BACKEND` was set by the user (with wgpu) and is kept as it is. `software`: why it
    /// runs on the processor (lavapipe), when it does.
    Vulkan {
        manifests: Vec<PathBuf>,
        user_backend: bool,
        software: Option<Reason>,
    },
}

/// Why the program renders in software.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// No chip that drives the screen: no GPU (CI, a virtual machine), or several that claim
    /// it.
    NoScreenChip,
    /// The chip that drives the screen has a driver the shell does not use (nouveau, a
    /// virtual GPU).
    UnsupportedChip,
    /// Several chips of the family that drives the screen (an AMD APU and a Radeon card):
    /// their Vulkan driver would open both, and wake the dedicated one.
    SeveralChips,
    /// The chip that drives the screen has no manifest of its Vulkan driver.
    NoVulkanDriver,
    /// The rescue lock screen, in software whatever the machine (lot D-25c).
    Rescue,
}

impl Choice {
    /// Decides from `ICED_BACKEND` as the user set it, the chips and the Vulkan driver
    /// manifests found. The GPU is used when one chip drives the screen (`boot_vga`, or the
    /// only chip), its driver is one the shell renders on (`FAMILIES`), no other chip of that
    /// family would be opened with it, and its Vulkan driver is installed. Otherwise lavapipe,
    /// when installed, and tiny-skia last.
    pub fn choose(iced_backend: Option<&str>, chips: &[Chip], manifests: &[PathBuf]) -> Self {
        // Read like iced reads it: empty candidates are dropped, and an empty list is no
        // choice at all (iced then tries wgpu first, on any adapter).
        let iced_backend = iced_backend.filter(|b| b.split(',').any(|c| !c.is_empty()));
        let wants_wgpu = iced_backend.is_none_or(|b| b.split(',').any(|c| c == "wgpu"));
        if !wants_wgpu {
            return Self::Inherited;
        }
        let user_backend = iced_backend.is_some();
        let vulkan = |prefixes: &[&str], software| {
            let found: Vec<PathBuf> = manifests
                .iter()
                .filter(|path| is_manifest(path, prefixes))
                .cloned()
                .collect();
            (!found.is_empty()).then_some(Self::Vulkan {
                manifests: found,
                user_backend,
                software,
            })
        };
        let reason = match screen_chip(chips) {
            None => Reason::NoScreenChip,
            Some(chip) => match family(&chip.driver) {
                None => Reason::UnsupportedChip,
                Some((drivers, prefixes)) => {
                    let same = chips
                        .iter()
                        .filter(|c| drivers.contains(&c.driver.as_str()));
                    if same.count() > 1 {
                        Reason::SeveralChips
                    } else if let Some(choice) = vulkan(prefixes, None) {
                        return choice;
                    } else {
                        Reason::NoVulkanDriver
                    }
                }
            },
        };
        // On the processor: Vulkan still, for the translucent glass and the animations.
        if let Some(choice) = vulkan(LAVAPIPE, Some(reason)) {
            return choice;
        }
        if user_backend {
            Self::Inherited
        } else {
            Self::Software(reason)
        }
    }

    /// Reads the machine: `ICED_BACKEND`, the chips in `/sys/class/drm`, the manifests in the
    /// Vulkan loader's folders.
    pub fn detect() -> Self {
        let backend = std::env::var("ICED_BACKEND").ok();
        Self::choose(
            backend.as_deref(),
            &chips(Path::new("/sys/class/drm")),
            &manifests(),
        )
    }

    /// The variables of this choice: a value to set, or `None` to unset it.
    pub fn variables(&self) -> Vec<(&'static str, Option<OsString>)> {
        let set = |name, value: OsString| (name, Some(value));
        match self {
            Self::Inherited => Vec::new(),
            Self::Software(_) => vec![set("ICED_BACKEND", "tiny-skia".into())],
            Self::Vulkan {
                manifests,
                user_backend,
                ..
            } => {
                let mut variables = Vec::new();
                if !user_backend {
                    // wgpu first, tiny-skia if it finds no adapter.
                    variables.push(set("ICED_BACKEND", "wgpu,tiny-skia".into()));
                }
                variables.extend([
                    // Vulkan only: wgpu's OpenGL backend would go through EGL vendor libraries,
                    // NVIDIA's first on Debian. (Only Vulkan is compiled anyway.)
                    set("WGPU_BACKEND", "vulkan".into()),
                    set("WGPU_POWER_PREF", "low".into()),
                    // The loader sees Mesa's AMD driver and nothing else.
                    set("VK_DRIVER_FILES", join(manifests)),
                    // iced takes the first format the driver lists without sRGB. On Wayland,
                    // Mesa lists the 10-bit ones first, with 2 bits of alpha: the bar's glass
                    // at 0.78 would come out at 0.67. This Mesa option (driconf, read from the
                    // environment) lists 8-bit BGRA first.
                    set("vk_wsi_force_bgra8_unorm_first", "true".into()),
                ]);
                variables.extend(NEUTRALISED.map(|name| (name, None)));
                variables
            }
        }
    }

    /// Software for the rescue lock screen when `rescue`, this choice otherwise (lot D-25c).
    pub fn or_rescue(self, rescue: bool) -> Self {
        match rescue {
            true => Self::Software(Reason::Rescue),
            false => self,
        }
    }

    /// Sets the variables of this choice for the whole process, and remembers the previous
    /// values for the commands it will start (`for_child`).
    ///
    /// # Safety
    ///
    /// Changes the process environment (`std::env::set_var`, `remove_var`). It must be the
    /// first statement of `main`, called once, while the process has a single thread: no
    /// thread may have been started before, and `main` must not carry `#[tokio::main]` or any
    /// other attribute that starts a runtime or threads before its body runs.
    pub unsafe fn apply(&self) {
        let variables = self.variables();
        let previous = variables
            .iter()
            .map(|(name, _)| (*name, std::env::var_os(name)))
            .collect();
        for (name, value) in variables {
            match value {
                // SAFETY: the caller guarantees that the process has a single thread.
                Some(value) => unsafe { std::env::set_var(name, value) },
                // SAFETY: as above.
                None => unsafe { std::env::remove_var(name) },
            }
        }
        let _ = PREVIOUS.set(previous);
    }
}

/// Values of the variables before `apply`, given back to child processes.
static PREVIOUS: OnceLock<Vec<(&'static str, Option<OsString>)>> = OnceLock::new();

/// Gives `command` the environment the program was started with: a program launched from
/// the shell (an application, a game) must not inherit the shell's choice of renderer, and
/// gets back the variables `apply` unset (`DRI_PRIME`, `__NV_PRIME_RENDER_OFFLOAD`…).
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

/// The chip that drives the screen: the one the firmware showed its boot screen on, or the
/// only one. None when several claim it, or none.
fn screen_chip(chips: &[Chip]) -> Option<&Chip> {
    let shown: Vec<&Chip> = chips.iter().filter(|c| c.boot_vga).collect();
    match (shown.as_slice(), chips) {
        ([chip], _) => Some(chip),
        ([], [chip]) => Some(chip),
        _ => None,
    }
}

/// The family of the kernel driver `driver`: its drivers, and the prefixes of the manifests of
/// its Vulkan driver.
fn family(driver: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    FAMILIES
        .iter()
        .find(|(drivers, _)| drivers.contains(&driver))
        .copied()
}

/// A manifest of one of the drivers named by `prefixes`, for this process's architecture.
fn is_manifest(path: &Path, prefixes: &[&str]) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let stem = name.strip_suffix(".json").unwrap_or("");
            prefixes
                .iter()
                .any(|p| stem == *p || stem.starts_with(&format!("{p}.")))
                && !OTHER_ARCHES.iter().any(|arch| name.contains(arch))
        })
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

/// The chip of one card, from its `device` folder (`/sys/class/drm/card0/device`). Also read by
/// the compositor to choose the card it drives.
pub fn chip(device: &Path) -> Option<Chip> {
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

    /// The manifests of the reference laptop (Debian), plus Arch's naming (with lib32).
    fn manifests() -> Vec<PathBuf> {
        [
            "/usr/share/vulkan/icd.d/intel_hasvk_icd.json",
            "/usr/share/vulkan/icd.d/intel_icd.json",
            "/usr/share/vulkan/icd.d/lvp_icd.json",
            "/usr/share/vulkan/icd.d/nouveau_icd.json",
            "/usr/share/vulkan/icd.d/nvidia_icd.json",
            "/usr/share/vulkan/icd.d/radeon_icd.json",
            "/usr/share/vulkan/icd.d/radeon_icd.i686.json",
            "/usr/share/vulkan/icd.d/radeon_icd.x86_64.json",
        ]
        .map(PathBuf::from)
        .to_vec()
    }

    fn get(variables: &[(&'static str, Option<OsString>)], name: &str) -> Option<Option<OsString>> {
        variables
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.clone())
    }

    fn set(value: &str) -> Option<Option<OsString>> {
        Some(Some(OsString::from(value)))
    }

    #[test]
    fn the_reference_laptop_renders_on_the_amd_chip_only() {
        let chips = [chip(0x10de, "nvidia", false), chip(AMD, AMDGPU, true)];
        let choice = Choice::choose(None, &chips, &manifests());
        let Choice::Vulkan { manifests, .. } = &choice else {
            panic!("expected Vulkan, got {choice:?}");
        };
        // Neither NVIDIA's driver nor nouveau's (NVK) nor the software one nor the 32-bit
        // one: only RADV for this process, on the chip, not the processor.
        assert!(matches!(choice, Choice::Vulkan { software: None, .. }));
        assert_eq!(
            manifests,
            &[
                PathBuf::from("/usr/share/vulkan/icd.d/radeon_icd.json"),
                PathBuf::from("/usr/share/vulkan/icd.d/radeon_icd.x86_64.json"),
            ]
        );
        let variables = choice.variables();
        assert_eq!(get(&variables, "WGPU_BACKEND"), set("vulkan"));
        assert_eq!(get(&variables, "ICED_BACKEND"), set("wgpu,tiny-skia"));
        assert_eq!(
            get(&variables, "vk_wsi_force_bgra8_unorm_first"),
            set("true")
        );
        assert_eq!(
            get(&variables, "VK_DRIVER_FILES"),
            set(concat!(
                "/usr/share/vulkan/icd.d/radeon_icd.json:",
                "/usr/share/vulkan/icd.d/radeon_icd.x86_64.json"
            ))
        );
        // What would bring another GPU back in is unset.
        for name in NEUTRALISED {
            assert_eq!(get(&variables, name), Some(None), "{name}");
        }
    }

    #[test]
    fn an_intel_chip_renders_with_anv_and_a_radeon_card_beside_it_sleeps() {
        // An Intel laptop with a Radeon card: the Intel chip drives the screen.
        let intel_and_radeon = [chip(0x8086, "i915", true), chip(AMD, AMDGPU, false)];
        let choice = Choice::choose(None, &intel_and_radeon, &manifests());
        assert_eq!(
            get(&choice.variables(), "VK_DRIVER_FILES"),
            set(concat!(
                "/usr/share/vulkan/icd.d/intel_hasvk_icd.json:",
                "/usr/share/vulkan/icd.d/intel_icd.json"
            ))
        );
        assert!(matches!(choice, Choice::Vulkan { software: None, .. }));
        // The newer kernel driver of Intel, `xe`, the same. NVIDIA's own driver, when its chip
        // drives the screen (a desktop): its Vulkan driver only.
        let xe = Choice::choose(None, &[chip(0x8086, "xe", true)], &manifests());
        assert!(matches!(xe, Choice::Vulkan { software: None, .. }));
        let nvidia = Choice::choose(None, &[chip(0x10de, "nvidia", true)], &manifests());
        assert_eq!(
            get(&nvidia.variables(), "VK_DRIVER_FILES"),
            set("/usr/share/vulkan/icd.d/nvidia_icd.json")
        );
        // A hybrid laptop: Intel on the screen, NVIDIA asleep and never asked.
        let hybrid = [chip(0x8086, "i915", true), chip(0x10de, "nvidia", false)];
        let choice = Choice::choose(None, &hybrid, &manifests());
        let files = get(&choice.variables(), "VK_DRIVER_FILES")
            .flatten()
            .unwrap();
        assert!(!files.to_string_lossy().contains("nvidia"));
    }

    #[test]
    fn without_a_usable_chip_rendering_goes_to_lavapipe_then_tiny_skia() {
        for (chips, reason) in [
            (vec![], Reason::NoScreenChip),
            (vec![chip(0x10de, "nouveau", true)], Reason::UnsupportedChip),
            (
                vec![chip(0x1af4, "virtio-pci", true)],
                Reason::UnsupportedChip,
            ),
            // An AMD APU with a Radeon card: RADV would open both.
            (
                vec![chip(AMD, AMDGPU, true), chip(AMD, AMDGPU, false)],
                Reason::SeveralChips,
            ),
        ] {
            let choice = Choice::choose(None, &chips, &manifests());
            assert_eq!(
                choice,
                Choice::Vulkan {
                    manifests: vec![PathBuf::from("/usr/share/vulkan/icd.d/lvp_icd.json")],
                    user_backend: false,
                    software: Some(reason),
                },
                "{chips:?}"
            );
            assert_eq!(
                get(&choice.variables(), "ICED_BACKEND"),
                set("wgpu,tiny-skia")
            );
            // Without lavapipe either: tiny-skia.
            let none: Vec<PathBuf> = Vec::new();
            let choice = Choice::choose(None, &chips, &none);
            assert_eq!(choice, Choice::Software(reason));
            assert_eq!(
                choice.variables(),
                vec![("ICED_BACKEND", Some(OsString::from("tiny-skia")))]
            );
        }
    }

    #[test]
    fn without_the_chips_vulkan_driver_rendering_goes_to_lavapipe() {
        let chips = [chip(AMD, AMDGPU, true)];
        let others: Vec<PathBuf> = manifests()
            .into_iter()
            .filter(|p| !is_manifest(p, &["radeon_icd"]))
            .collect();
        assert!(matches!(
            Choice::choose(None, &chips, &others),
            Choice::Vulkan {
                software: Some(Reason::NoVulkanDriver),
                ..
            }
        ));
    }

    #[test]
    fn a_renderer_set_by_the_user_wins() {
        let chips = [chip(AMD, AMDGPU, true)];
        let choice = Choice::choose(Some("tiny-skia"), &chips, &manifests());
        assert_eq!(choice, Choice::Inherited);
        assert!(choice.variables().is_empty());
    }

    #[test]
    fn an_empty_iced_backend_is_no_choice() {
        // iced drops empty candidates and falls back to wgpu first: so does the choice, which
        // then restricts it to the AMD chip.
        let chips = [chip(0x10de, "nvidia", false), chip(AMD, AMDGPU, true)];
        for empty in ["", ",", ",,"] {
            let choice = Choice::choose(Some(empty), &chips, &manifests());
            assert!(
                matches!(
                    choice,
                    Choice::Vulkan {
                        user_backend: false,
                        ..
                    }
                ),
                "{empty:?}: {choice:?}"
            );
            assert_eq!(
                get(&choice.variables(), "ICED_BACKEND"),
                set("wgpu,tiny-skia")
            );
        }
    }

    #[test]
    fn wgpu_asked_by_the_user_stays_on_the_chip_of_the_screen() {
        let chips = [chip(0x10de, "nvidia", false), chip(AMD, AMDGPU, true)];
        let choice = Choice::choose(Some("wgpu"), &chips, &manifests());
        let variables = choice.variables();
        // The user's ICED_BACKEND is kept, the Vulkan restriction still applies.
        assert_eq!(get(&variables, "ICED_BACKEND"), None);
        assert!(get(&variables, "VK_DRIVER_FILES").is_some());
        // Where nothing can be narrowed down, the user's choice stands as it is.
        let none: Vec<PathBuf> = Vec::new();
        assert_eq!(
            Choice::choose(Some("wgpu"), &[chip(0x10de, "nouveau", true)], &none),
            Choice::Inherited
        );
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
