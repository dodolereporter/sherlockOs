//! Keyboard backlight (lot D-35d): UPower's `KbdBacklight` on the system bus, or else logind's
//! `Session.SetBrightness` on the LED the kernel names `…::kbd_backlight`, read from
//! `/sys/class/leds`. Nothing is ever written into sysfs.

use std::path::Path;

use zbus::Connection;

use crate::{Problem, Result, backlight};

pub const UPOWER: &str = "org.freedesktop.UPower";
pub const PATH: &str = "/org/freedesktop/UPower/KbdBacklight";
pub const INTERFACE: &str = "org.freedesktop.UPower.KbdBacklight";
/// Where the kernel lists the LEDs; only read.
pub const LEDS: &str = "/sys/class/leds";
/// End of the name of a keyboard backlight LED (`tpacpi::kbd_backlight`, `asus::kbd_backlight`).
const SUFFIX: &str = "::kbd_backlight";
/// Most steps from off to full: a backlight of 3 levels steps by one, one of 255 by a tenth.
pub const LEVELS: u32 = 10;

/// What the brightness goes through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Through {
    UPower,
    /// logind, on this LED of `/sys/class/leds`.
    Logind(String),
}

/// The keyboard backlight and its brightness, in the machine's own units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardLight {
    pub brightness: u32,
    pub max: u32,
    pub through: Through,
}

impl KeyboardLight {
    pub fn percent(&self) -> u8 {
        let max = u64::from(self.max.max(1));
        let percent = (u64::from(self.brightness) * 100 + max / 2) / max;
        u8::try_from(percent.min(100)).unwrap_or(100)
    }

    /// The raw value nearest to `percent`: what a slider sets.
    pub fn raw(&self, percent: u8) -> u32 {
        let raw = (u64::from(self.max) * u64::from(percent.min(100)) + 50) / 100;
        u32::try_from(raw).unwrap_or(self.max).min(self.max)
    }
}

/// The raw value of the level `k` of `levels`, under `max`.
fn level(k: u32, levels: u32, max: u32) -> u32 {
    let raw = (u64::from(k) * u64::from(max) + u64::from(levels) / 2) / u64::from(levels);
    u32::try_from(raw).unwrap_or(max)
}

/// The brightness one key press away from `brightness`, on a backlight of `max`: the next of at
/// most `LEVELS` levels, up (at most `max`) or down (at least off).
pub fn step(brightness: u32, max: u32, up: bool) -> u32 {
    let levels = max.clamp(1, LEVELS);
    let mut raws = (0..=levels).map(|k| level(k, levels, max));
    if up {
        raws.find(|raw| *raw > brightness)
            .unwrap_or(brightness.max(max))
    } else {
        raws.rev().find(|raw| *raw < brightness).unwrap_or(0)
    }
}

/// The brightness the on and off key gives back: half the levels.
fn half(max: u32) -> u32 {
    let levels = max.clamp(1, LEVELS);
    level(levels.div_ceil(2), levels, max)
}

async fn upower(bus: &Connection) -> Result<KeyboardLight> {
    let get = |method: &'static str| async move {
        let reply = bus
            .call_method(Some(UPOWER), PATH, Some(INTERFACE), method, &())
            .await?;
        Ok::<i32, Problem>(reply.body().deserialize()?)
    };
    let max = get("GetMaxBrightness").await?;
    if max <= 0 {
        return Err(Problem::Absent(String::from("no keyboard backlight")));
    }
    let brightness = get("GetBrightness").await?;
    Ok(KeyboardLight {
        brightness: u32::try_from(brightness).unwrap_or(0),
        max: max.unsigned_abs(),
        through: Through::UPower,
    })
}

/// The keyboard backlight LED under `leds`, the first by name.
pub fn led(leds: &Path) -> Result<KeyboardLight> {
    let absent = || Problem::Absent(String::from("no keyboard backlight"));
    let mut names: Vec<String> = std::fs::read_dir(leds)
        .map_err(|_| absent())?
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(SUFFIX))
        .collect();
    names.sort();
    let name = names.into_iter().next().ok_or_else(absent)?;
    let number = |file: &str| -> Result<u32> {
        let text = std::fs::read_to_string(leds.join(&name).join(file))
            .map_err(|e| Problem::Failed(format!("{name}/{file}: {e}")))?;
        text.trim()
            .parse()
            .map_err(|e| Problem::Failed(format!("{name}/{file}: {e}")))
    };
    let max = number("max_brightness")?;
    if max == 0 {
        return Err(absent());
    }
    Ok(KeyboardLight {
        brightness: number("brightness")?.min(max),
        max,
        through: Through::Logind(name),
    })
}

/// The keyboard backlight: UPower's, or else the LED under `leds`. `Problem::Absent` when the
/// machine has none.
pub async fn read(bus: &Connection, leds: &Path) -> Result<KeyboardLight> {
    match upower(bus).await {
        Err(Problem::Absent(why)) => led(leds).map_err(|problem| match problem {
            Problem::Absent(_) => Problem::Absent(why),
            other => other,
        }),
        read => read,
    }
}

/// Sets the brightness of `light` to `raw`, through what it goes through.
pub async fn set(bus: &Connection, light: &KeyboardLight, raw: u32) -> Result<()> {
    let raw = raw.min(light.max);
    match &light.through {
        Through::UPower => {
            let value = i32::try_from(raw).unwrap_or(i32::MAX);
            bus.call_method(Some(UPOWER), PATH, Some(INTERFACE), "SetBrightness", &value)
                .await?;
        }
        Through::Logind(name) => {
            bus.call_method(
                Some(backlight::SERVICE),
                backlight::SESSION,
                Some(backlight::INTERFACE),
                "SetBrightness",
                &("leds", name.as_str(), raw),
            )
            .await?;
        }
    }
    Ok(())
}

/// `steps` steps of the keyboard backlight, up when positive, down when negative, set at once.
/// Returns it lit anew, and whether a step pushed against the end (`crate::press`).
pub async fn step_from(bus: &Connection, leds: &Path, steps: i32) -> Result<(KeyboardLight, bool)> {
    let light = read(bus, leds).await?;
    let (brightness, pushed) =
        crate::press(light.brightness, steps, |b, up| step(b, light.max, up));
    if brightness != light.brightness {
        set(bus, &light, brightness).await?;
    }
    Ok((
        KeyboardLight {
            brightness,
            ..light
        },
        pushed,
    ))
}

/// Turns the keyboard backlight off when it is lit, else back on at `restore`, or at half its
/// levels. Returns it lit anew, and the brightness turned off, to give back next time.
pub async fn toggle(
    bus: &Connection,
    leds: &Path,
    restore: Option<u32>,
) -> Result<(KeyboardLight, Option<u32>)> {
    let light = read(bus, leds).await?;
    let (brightness, turned_off) = if light.brightness > 0 {
        (0, Some(light.brightness))
    } else {
        let back = restore.filter(|r| (1..=light.max).contains(r));
        (back.unwrap_or_else(|| half(light.max)), None)
    };
    set(bus, &light, brightness).await?;
    Ok((
        KeyboardLight {
            brightness,
            ..light
        },
        turned_off,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_follow_the_levels_of_the_machine() {
        // Three levels: one at a time.
        assert_eq!(step(0, 3, true), 1);
        assert_eq!(step(2, 3, true), 3);
        assert_eq!(step(3, 3, true), 3);
        assert_eq!(step(1, 3, false), 0);
        assert_eq!(step(0, 3, false), 0);
        // 255: by tenths, from wherever it is.
        assert_eq!(step(0, 255, true), 26);
        assert_eq!(step(30, 255, true), 51);
        assert_eq!(step(30, 255, false), 26);
        assert_eq!(step(250, 255, true), 255);
        assert_eq!(step(255, 255, true), 255);
        // On and off gives back half the levels.
        assert_eq!(half(3), 2);
        assert_eq!(half(255), 128);
    }

    #[test]
    fn the_percent_and_the_raw_value_meet() {
        let light = KeyboardLight {
            brightness: 2,
            max: 3,
            through: Through::UPower,
        };
        assert_eq!(light.percent(), 67);
        assert_eq!(light.raw(100), 3);
        assert_eq!(light.raw(0), 0);
        assert_eq!(light.raw(50), 2);
    }

    #[test]
    fn the_led_is_found_by_its_name() {
        let dir = std::env::temp_dir().join(format!("leds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, value) in [("input3::capslock", "0"), ("tpacpi::kbd_backlight", "1")] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("brightness"), value).unwrap();
            std::fs::write(dir.join(name).join("max_brightness"), "2\n").unwrap();
        }
        let light = led(&dir).unwrap();
        assert_eq!(
            light.through,
            Through::Logind("tpacpi::kbd_backlight".into())
        );
        assert_eq!((light.brightness, light.max), (1, 2));
        std::fs::remove_dir_all(dir.join("tpacpi::kbd_backlight")).unwrap();
        assert!(matches!(led(&dir), Err(Problem::Absent(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
