//! Screen brightness: read from `/sys/class/backlight`, written by logind's
//! `Session.SetBrightness` on the system bus, which lets the user of the active session change
//! it without root.

use std::path::{Path, PathBuf};

use zbus::Connection;

use super::{Problem, Result};

pub const SERVICE: &str = "org.freedesktop.login1";
/// The caller's own session.
pub const SESSION: &str = "/org/freedesktop/login1/session/auto";
const INTERFACE: &str = "org.freedesktop.login1.Session";
/// Where the kernel lists the backlights.
pub const SYSFS: &str = "/sys/class/backlight";
/// Lowest brightness the slider gives, in percent: a black screen is not a brightness.
pub const MIN_PERCENT: u8 = 1;

/// A backlight and its brightness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Backlight {
    /// Its name under `/sys/class/backlight` (`amdgpu_bl1`, `intel_backlight`…).
    pub name: String,
    pub brightness: u32,
    pub max: u32,
}

impl Backlight {
    pub fn percent(&self) -> u8 {
        if self.max == 0 {
            return 0;
        }
        let max = u64::from(self.max);
        let percent = (u64::from(self.brightness) * 100 + max / 2) / max;
        u8::try_from(percent.min(100)).unwrap_or(100)
    }

    /// The raw value of `percent`, never below `MIN_PERCENT` nor 1.
    pub fn raw(&self, percent: u8) -> u32 {
        let percent = u64::from(percent.clamp(MIN_PERCENT, 100));
        let raw = (u64::from(self.max) * percent + 50) / 100;
        u32::try_from(raw)
            .unwrap_or(self.max)
            .clamp(1, self.max.max(1))
    }
}

/// The backlight of the screen under `root`: the firmware one first, then the platform one,
/// then a raw one, as the kernel documentation advises; by name within a kind.
pub fn find(root: &Path) -> Result<Backlight> {
    let entries =
        std::fs::read_dir(root).map_err(|e| Problem::Absent(format!("{}: {e}", root.display())))?;
    let mut found: Vec<(u8, String, PathBuf)> = entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
            let rank = match kind.trim() {
                "firmware" => 0,
                "platform" => 1,
                _ => 2,
            };
            (rank, entry.file_name().to_string_lossy().into_owned(), path)
        })
        .collect();
    found.sort();
    let Some((_, name, path)) = found.into_iter().next() else {
        return Err(Problem::Absent(String::from("no backlight")));
    };
    let number = |file: &str| -> Result<u32> {
        let text = std::fs::read_to_string(path.join(file))
            .map_err(|e| Problem::Failed(format!("{name}/{file}: {e}")))?;
        text.trim()
            .parse()
            .map_err(|e| Problem::Failed(format!("{name}/{file}: {e}")))
    };
    Ok(Backlight {
        brightness: number("brightness")?,
        max: number("max_brightness")?,
        name,
    })
}

/// Sets the brightness of `backlight` to `percent`, through logind.
pub async fn set(bus: &Connection, backlight: &Backlight, percent: u8) -> Result<()> {
    let raw = backlight.raw(percent);
    bus.call_method(
        Some(SERVICE),
        SESSION,
        Some(INTERFACE),
        "SetBrightness",
        &("backlight", backlight.name.as_str(), raw),
    )
    .await?;
    Ok(())
}
