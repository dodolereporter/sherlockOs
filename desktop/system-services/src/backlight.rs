//! Screen brightness: read from `/sys/class/backlight`, written by logind's
//! `Session.SetBrightness` on the system bus, which lets the user of the active session change
//! it without root.

use std::path::{Path, PathBuf};

use zbus::Connection;

use crate::{Problem, Result};

pub const SERVICE: &str = "org.freedesktop.login1";
/// The caller's own session.
pub const SESSION: &str = "/org/freedesktop/login1/session/auto";
const INTERFACE: &str = "org.freedesktop.login1.Session";
/// Where the kernel lists the backlights.
pub const SYSFS: &str = "/sys/class/backlight";
/// Lowest brightness the slider and the keys give, in percent: a black screen is not a
/// brightness.
pub const MIN_PERCENT: u8 = 1;
/// Step of the brightness keys, in percent.
pub const STEP: u8 = 5;

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

/// The brightness one key press away from `percent`: the next multiple of `STEP` up (at most
/// 100) or down (at least `MIN_PERCENT`).
pub fn step(percent: u8, up: bool) -> u8 {
    if up {
        ((percent / STEP + 1) * STEP).min(100)
    } else {
        (percent.saturating_sub(1) / STEP * STEP).max(MIN_PERCENT)
    }
}

/// One step up or down of the backlight found under `root`, through logind. Returns the new
/// brightness, in percent.
pub async fn step_from(bus: &Connection, root: &Path, up: bool) -> Result<u8> {
    let backlight = find(root)?;
    let percent = step(backlight.percent(), up);
    set(bus, &backlight, percent).await?;
    Ok(percent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_step_by_five_and_never_reach_black() {
        assert_eq!(step(50, true), 55);
        assert_eq!(step(52, true), 55);
        assert_eq!(step(52, false), 50);
        assert_eq!(step(50, false), 45);
        assert_eq!(step(100, true), 100);
        assert_eq!(step(5, false), MIN_PERCENT);
        assert_eq!(step(MIN_PERCENT, false), MIN_PERCENT);
        assert_eq!(step(MIN_PERCENT, true), 5);
    }
}
