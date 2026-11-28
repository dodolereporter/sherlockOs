//! Choice of the graphics card the real session drives, read from `/sys` only.
//!
//! Like the shell's choice of renderer (`sherlock_ui::gpu`, ADR 0018), it reads attributes the
//! kernel keeps (vendor, driver, `boot_vga`, connector status, runtime power state), which never
//! wakes a card. The chosen card renders and drives its screens; another card (the NVIDIA card
//! whose HDMI port a hybrid laptop wires to it) is opened only while a screen is plugged into
//! it, and closed as soon as there is none, so that it sleeps again (lot D-16, ADR 0037).

use std::path::{Path, PathBuf};

use sherlock_ui::gpu::{self, Chip};

/// PCI vendor of NVIDIA.
const NVIDIA: u16 = 0x10de;

/// A card of `/sys/class/drm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// Name in `/sys/class/drm` and in `/dev/dri`: `card0`, `card1`…
    pub name: String,
    pub chip: Chip,
    /// At least one of its connectors has a screen plugged in.
    pub screen: bool,
}

/// The cards under `drm` (`/sys/class/drm`), by name.
pub fn cards(drm: &Path) -> Vec<Card> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let mut cards: Vec<Card> = names
        .iter()
        .filter(|name| {
            name.strip_prefix("card")
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|name| {
            let chip = gpu::chip(&drm.join(name).join("device"))?;
            let screen = has_screen(drm, name);
            Some(Card {
                name: name.clone(),
                chip,
                screen,
            })
        })
        .collect();
    cards.sort_by(|a, b| a.name.cmp(&b.name));
    cards
}

/// The card to drive: the one the firmware showed its boot screen on (`boot_vga`) with a
/// screen plugged in, or else the `boot_vga` one, or else any card with a screen. Never an
/// NVIDIA card.
pub fn choose(cards: &[Card]) -> Option<&Card> {
    let allowed = || cards.iter().filter(|c| c.chip.vendor != NVIDIA);
    allowed()
        .find(|c| c.chip.boot_vga && c.screen)
        .or_else(|| allowed().find(|c| c.chip.boot_vga))
        .or_else(|| allowed().find(|c| c.screen))
}

/// Whether a screen is plugged into the card `name` (`card1`) under `drm`: the status of its
/// connectors (`card1-HDMI-A-1/status`). The kernel gives the status it keeps, updated by the
/// card's hotplug interrupt: reading it never probes the connector, nor wakes the card.
pub fn has_screen(drm: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return false;
    };
    // Connectors are named `card0-eDP-1`, `card0-HDMI-A-1`…
    let prefix = format!("{name}-");
    entries.flatten().any(|entry| {
        entry.file_name().to_string_lossy().starts_with(&prefix)
            && std::fs::read_to_string(entry.path().join("status"))
                .is_ok_and(|s| s.trim() == "connected")
    })
}

/// The runtime power state of the card `name` under `drm` (`active`, `suspended`…), as the
/// kernel keeps it: for the log, to see the NVIDIA card sleep again (lot D-16).
pub fn runtime_status(drm: &Path, name: &str) -> Option<String> {
    let path = drm.join(name).join("device/power/runtime_status");
    Some(std::fs::read_to_string(path).ok()?.trim().to_owned())
}

/// The name of the card at `node` (`/dev/dri/card1`): `card1`.
pub fn name_of(node: &Path) -> Option<String> {
    let name = node.file_name()?.to_str()?;
    name.starts_with("card").then(|| name.to_owned())
}

/// Whether the cards besides the one driven may be opened when a screen is plugged into them:
/// not with `SHERLOCKOS_SECONDARY_CARDS=0`, nor when the file `no-secondary-cards` is in the
/// settings folder (`~/.config/sherlockos`), the way back if one misbehaves (lot D-16).
pub fn secondary_allowed(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    let off = var("SHERLOCKOS_SECONDARY_CARDS").is_some_and(|v| v == "0");
    let file = desktop_config::config_dir(&var).map(|dir| dir.join("no-secondary-cards"));
    !off && !file.is_some_and(|f| f.exists())
}

/// Device node of the card chosen on this machine, with the reason when there is none.
pub fn primary() -> Result<PathBuf, String> {
    let cards = cards(Path::new("/sys/class/drm"));
    tracing::info!(?cards, "graphics cards");
    choose(&cards)
        .map(|card| Path::new("/dev/dri").join(&card.name))
        .ok_or_else(|| format!("no card to drive among {cards:?} (an NVIDIA card never renders)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AMD: u16 = 0x1002;
    const INTEL: u16 = 0x8086;

    /// A fake `/sys/class/drm`: each card with its connectors and their status.
    struct FakeSys(PathBuf);

    /// Name, vendor, driver, `boot_vga`, connectors with their status.
    type FakeCard<'a> = (&'a str, u16, &'a str, bool, &'a [(&'a str, &'a str)]);

    impl FakeSys {
        fn new(test: &str, cards: &[FakeCard]) -> Self {
            let root =
                std::env::temp_dir().join(format!("compositor-card-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            for (card, vendor, driver, boot_vga, connectors) in cards {
                let device = root.join(card).join("device");
                std::fs::create_dir_all(&device).unwrap();
                std::fs::write(device.join("vendor"), format!("{vendor:#06x}\n")).unwrap();
                std::fs::write(
                    device.join("boot_vga"),
                    if *boot_vga { "1\n" } else { "0\n" },
                )
                .unwrap();
                let bound = root.join("drivers").join(driver);
                std::fs::create_dir_all(&bound).unwrap();
                std::os::unix::fs::symlink(&bound, device.join("driver")).unwrap();
                for (connector, status) in *connectors {
                    let dir = root.join(format!("{card}-{connector}"));
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join("status"), format!("{status}\n")).unwrap();
                }
            }
            Self(root)
        }

        fn chosen(&self) -> Option<String> {
            choose(&cards(&self.0)).map(|c| c.name.clone())
        }
    }

    impl Drop for FakeSys {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn an_amd_chip_alone_is_chosen() {
        let sys = FakeSys::new(
            "amd",
            &[("card0", AMD, "amdgpu", true, &[("eDP-1", "connected")])],
        );
        assert_eq!(sys.chosen().as_deref(), Some("card0"));
        let cards = cards(&sys.0);
        assert!(cards[0].screen);
    }

    #[test]
    fn the_nvidia_card_of_the_reference_laptop_is_never_chosen() {
        // The laptop: NVIDIA first in the PCI order, with the HDMI port; the AMD chip drives
        // the internal screen.
        let sys = FakeSys::new(
            "amd-nvidia",
            &[
                (
                    "card0",
                    NVIDIA,
                    "nvidia",
                    false,
                    &[("HDMI-A-1", "connected")],
                ),
                (
                    "card1",
                    AMD,
                    "amdgpu",
                    true,
                    &[("eDP-1", "connected"), ("DP-1", "disconnected")],
                ),
            ],
        );
        assert_eq!(sys.chosen().as_deref(), Some("card1"));
        // Its HDMI screen is seen from `/sys`, without opening it (lot D-16).
        assert!(has_screen(&sys.0, "card0"));
        assert!(!has_screen(&sys.0, "card9"));
        assert_eq!(
            name_of(Path::new("/dev/dri/card0")).as_deref(),
            Some("card0")
        );
        assert_eq!(name_of(Path::new("/dev/dri/renderD128")), None);
        let power = sys.0.join("card0/device/power");
        std::fs::create_dir_all(&power).unwrap();
        std::fs::write(power.join("runtime_status"), "suspended\n").unwrap();
        assert_eq!(
            runtime_status(&sys.0, "card0").as_deref(),
            Some("suspended")
        );
        let off = |key: &str| (key == "SHERLOCKOS_SECONDARY_CARDS").then(|| "0".into());
        assert!(!secondary_allowed(off));
        let config = sys.0.join("config");
        let home = |key: &str| (key == "XDG_CONFIG_HOME").then(|| config.clone().into());
        assert!(secondary_allowed(home));
        std::fs::create_dir_all(config.join("sherlockos")).unwrap();
        std::fs::write(config.join("sherlockos/no-secondary-cards"), "").unwrap();
        assert!(!secondary_allowed(home), "the file is the way back");
        // Even if the NVIDIA card were the only one with a screen, or `boot_vga`.
        let only_nvidia = FakeSys::new(
            "nvidia-only",
            &[(
                "card0",
                NVIDIA,
                "nvidia",
                true,
                &[("HDMI-A-1", "connected")],
            )],
        );
        assert_eq!(only_nvidia.chosen(), None);
    }

    #[test]
    fn an_intel_chip_with_a_dedicated_amd_card_drives_the_screen() {
        let sys = FakeSys::new(
            "intel-amd",
            &[
                ("card0", AMD, "amdgpu", false, &[("DP-1", "disconnected")]),
                ("card1", INTEL, "i915", true, &[("eDP-1", "connected")]),
            ],
        );
        assert_eq!(sys.chosen().as_deref(), Some("card1"));
    }

    #[test]
    fn without_a_screen_the_boot_card_is_kept() {
        let sys = FakeSys::new(
            "no-screen",
            &[
                ("card0", AMD, "amdgpu", false, &[("DP-1", "disconnected")]),
                ("card1", INTEL, "i915", true, &[("eDP-1", "disconnected")]),
            ],
        );
        assert_eq!(sys.chosen().as_deref(), Some("card1"));
        // No `boot_vga` at all (some virtual machines): the card with a screen.
        let vm = FakeSys::new(
            "vm",
            &[(
                "card0",
                0x1234,
                "bochs-drm",
                false,
                &[("Virtual-1", "connected")],
            )],
        );
        assert_eq!(vm.chosen().as_deref(), Some("card0"));
    }
}
