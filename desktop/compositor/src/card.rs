//! Choice of the graphics card the real session drives, read from `/sys` only.
//!
//! Like the shell's choice of renderer (`sherlock_ui::gpu`, ADR 0018), it reads attributes the
//! kernel keeps (vendor, driver, `boot_vga`, connector status), which never wakes a card. The
//! chosen card is the only one the compositor opens: an NVIDIA card is never opened, so that
//! it stays asleep outside games.

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
            // Connectors are named `card0-eDP-1`, `card0-HDMI-A-1`…
            let prefix = format!("{name}-");
            let screen = names.iter().any(|c| {
                c.starts_with(&prefix)
                    && std::fs::read_to_string(drm.join(c).join("status"))
                        .is_ok_and(|s| s.trim() == "connected")
            });
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

/// Device node of the card chosen on this machine, with the reason when there is none.
pub fn primary() -> Result<PathBuf, String> {
    let cards = cards(Path::new("/sys/class/drm"));
    tracing::info!(?cards, "graphics cards");
    choose(&cards)
        .map(|card| Path::new("/dev/dri").join(&card.name))
        .ok_or_else(|| format!("no card to drive among {cards:?} (NVIDIA cards are never opened)"))
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
