//! The discreet line at the bottom of the lock and login screens (lot D-25i): the network, the
//! keyboard layout (« FR ») and the battery, as macOS shows them under its login window. Read
//! from the kernel's files only (`/sys/class/net`, `/proc/net/wireless`,
//! `/sys/class/power_supply`) and from the keyboard setting: no bus, nothing that can hang.
//! Every read is bounded, and they run on a thread of their own, every few seconds.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::futures::channel::mpsc;

/// How often the line is read again.
pub const EVERY: Duration = Duration::from_secs(5);
/// Network interfaces and power supplies looked at, at most.
const MAX_ENTRIES: usize = 32;
/// Largest small file read (`operstate`, `capacity`, a setting); `/proc/net/wireless` is read
/// up to `MAX_WIRELESS`.
const MAX_SMALL: u64 = 4096;
const MAX_WIRELESS: u64 = 16 * 1024;
/// The link quality of `/proc/net/wireless` that counts as a full signal (the usual maximum).
const FULL_QUALITY: f32 = 70.0;
/// Layout shown when nothing sets one, as the compositor does.
const DEFAULT_LAYOUT: &str = "fr";
/// System files that may hold the keyboard setting, as the compositor reads them.
const SYSTEM_KEYBOARD: [&str; 2] = ["/etc/vconsole.conf", "/etc/default/keyboard"];

/// The network, as the line shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Network {
    #[default]
    Offline,
    Wired,
    /// Wi-Fi, with its signal from 0 to 100.
    Wireless(u8),
}

/// A battery: its charge, and whether it charges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    pub percent: u8,
    pub charging: bool,
    /// Plugged in without charging (full, or held by a charge limit).
    pub plugged: bool,
}

/// What the line shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub network: Network,
    /// The keyboard layout, upper case: « FR », « US ».
    pub layout: String,
    /// None on a machine without battery: nothing is shown.
    pub battery: Option<Battery>,
}

/// Where the line is read from: the real files, or a fixture in the tests.
#[derive(Debug, Clone)]
pub struct Sources {
    pub sys: PathBuf,
    pub proc: PathBuf,
    /// `keyboard.conf` of the user, if there is a config dir.
    pub keyboard: Option<PathBuf>,
    /// The system keyboard files, the first one with a layout wins.
    pub system_keyboard: Vec<PathBuf>,
    /// `XKB_DEFAULT_LAYOUT`, which the compositor leaves to xkbcommon.
    pub env_layout: Option<String>,
}

impl Sources {
    /// The machine's own files and the setting of the user running the screen.
    pub fn machine() -> Self {
        let var = |key: &str| std::env::var_os(key);
        Self {
            sys: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
            keyboard: desktop_config::keyboard::config_file(var),
            system_keyboard: SYSTEM_KEYBOARD.iter().map(PathBuf::from).collect(),
            env_layout: var("XKB_DEFAULT_LAYOUT")
                .map(|v: OsString| v.to_string_lossy().into_owned()),
        }
    }

    /// Reads the line now. Blocking: on a thread of its own.
    pub fn read(&self) -> Status {
        Status {
            network: self.network(),
            layout: self.layout(),
            battery: self.battery(),
        }
    }

    /// A wired link up wins over Wi-Fi, as the bar shows it; only the interfaces of a device
    /// (`device` in sysfs) count, not the bridges and tunnels of containers.
    fn network(&self) -> Network {
        let Ok(dir) = std::fs::read_dir(self.sys.join("class/net")) else {
            return Network::Offline;
        };
        let mut wireless = None;
        for entry in dir.flatten().take(MAX_ENTRIES) {
            let path = entry.path();
            let up = small(&path.join("operstate")).is_some_and(|s| s.trim() == "up");
            if !up || !path.join("device").exists() {
                continue;
            }
            if !path.join("wireless").exists() && !path.join("phy80211").exists() {
                return Network::Wired;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            wireless = Some(self.signal(&name));
        }
        wireless.map_or(Network::Offline, Network::Wireless)
    }

    /// The signal of `interface` from the link quality of `/proc/net/wireless`; full when the
    /// file says nothing.
    fn signal(&self, interface: &str) -> u8 {
        let text = bounded(&self.proc.join("net/wireless"), MAX_WIRELESS).unwrap_or_default();
        text.lines()
            .find_map(|line| {
                let (name, rest) = line.split_once(':')?;
                if name.trim() != interface {
                    return None;
                }
                let quality: f32 = rest
                    .split_whitespace()
                    .nth(1)?
                    .trim_end_matches('.')
                    .parse()
                    .ok()?;
                Some((quality / FULL_QUALITY * 100.0).clamp(0.0, 100.0).round() as u8)
            })
            .unwrap_or(100)
    }

    /// The layout as the compositor picks it: the user's setting, `XKB_DEFAULT_LAYOUT`, the
    /// system's, then French. The first of a list (`us,fr`) is the one at login.
    fn layout(&self) -> String {
        let set = self
            .keyboard
            .as_deref()
            .and_then(small)
            .and_then(|text| desktop_config::keyboard::Config::parse(&text).ok())
            .and_then(|config| config.layout);
        let system = || {
            self.system_keyboard.iter().find_map(|file| {
                let text = small(file)?;
                text.lines().find_map(|line| {
                    let value = line.trim().strip_prefix("XKBLAYOUT=")?;
                    let value = value.trim_matches(['"', '\''].as_slice()).trim();
                    (!value.is_empty()).then(|| value.to_owned())
                })
            })
        };
        let layout = set
            .or_else(|| self.env_layout.clone().filter(|v| !v.is_empty()))
            .or_else(system)
            .unwrap_or_else(|| DEFAULT_LAYOUT.to_owned());
        shown_layout(&layout)
    }

    /// The machine's battery (`type` Battery, not a mouse's: `scope` Device is left out).
    fn battery(&self) -> Option<Battery> {
        let dir = std::fs::read_dir(self.sys.join("class/power_supply")).ok()?;
        let mut online = false;
        let mut found = None;
        for entry in dir.flatten().take(MAX_ENTRIES) {
            let path = entry.path();
            let kind = small(&path.join("type")).unwrap_or_default();
            if kind.trim() == "Mains" {
                online |= small(&path.join("online")).is_some_and(|s| s.trim() == "1");
                continue;
            }
            let device = small(&path.join("scope")).is_some_and(|s| s.trim() == "Device");
            if kind.trim() != "Battery" || device || found.is_some() {
                continue;
            }
            let Some(percent) = small(&path.join("capacity"))
                .and_then(|s| s.trim().parse::<u8>().ok())
                .map(|p| p.min(100))
            else {
                continue;
            };
            let state = small(&path.join("status")).unwrap_or_default();
            found = Some((percent, state.trim().to_owned()));
        }
        let (percent, state) = found?;
        let charging = state == "Charging";
        Some(Battery {
            percent,
            charging,
            plugged: !charging && (online || state == "Full" || state == "Not charging"),
        })
    }
}

/// The first layout of a list, upper case, at most three letters: « FR ».
pub fn shown_layout(layout: &str) -> String {
    layout
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(3)
        .collect::<String>()
        .to_uppercase()
}

/// The line, each time it changes; the thread stops when the receiver goes.
pub fn watch(sources: Sources) -> mpsc::UnboundedReceiver<Status> {
    let (sender, receiver) = mpsc::unbounded();
    let spawned = std::thread::Builder::new()
        .name("status".into())
        .spawn(move || {
            let mut last = None;
            loop {
                let now = sources.read();
                if last.as_ref() != Some(&now) {
                    last = Some(now.clone());
                    if sender.unbounded_send(now).is_err() {
                        return;
                    }
                }
                std::thread::sleep(EVERY);
                if sender.is_closed() {
                    return;
                }
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("status: {err}");
    }
    receiver
}

fn small(path: &Path) -> Option<String> {
    bounded(path, MAX_SMALL)
}

fn bounded(path: &Path, max: u64) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(max)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_line_is_read_from_the_kernel_files_and_the_setting() {
        let root = std::env::temp_dir().join(format!("lock-status-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sources = Sources {
            sys: root.join("sys"),
            proc: root.join("proc"),
            keyboard: Some(root.join("config/keyboard.conf")),
            system_keyboard: vec![root.join("vconsole.conf")],
            env_layout: None,
        };
        // Nothing: offline, French, no battery.
        assert_eq!(
            sources.read(),
            Status {
                network: Network::Offline,
                layout: "FR".into(),
                battery: None,
            }
        );
        let net = root.join("sys/class/net");
        // A bridge up without device does not count; Wi-Fi up with its quality.
        write(&net.join("docker0/operstate"), "up\n");
        write(&net.join("wlp2s0/operstate"), "up\n");
        std::fs::create_dir_all(net.join("wlp2s0/device")).unwrap();
        std::fs::create_dir_all(net.join("wlp2s0/wireless")).unwrap();
        write(
            &root.join("proc/net/wireless"),
            "Inter-| sta-|   Quality\n face | tus | link level noise\nwlp2s0: 0000   35.  -56.  -256\n",
        );
        assert_eq!(sources.read().network, Network::Wireless(50));
        // A cable plugged in wins.
        write(&net.join("enp3s0/operstate"), "up\n");
        std::fs::create_dir_all(net.join("enp3s0/device")).unwrap();
        assert_eq!(sources.read().network, Network::Wired);

        write(
            &root.join("vconsole.conf"),
            "KEYMAP=us\nXKBLAYOUT=\"us,fr\"\n",
        );
        assert_eq!(sources.read().layout, "US");
        write(&root.join("config/keyboard.conf"), "layout = de\n");
        assert_eq!(sources.read().layout, "DE");

        let power = root.join("sys/class/power_supply");
        // A mouse's battery is not the machine's.
        write(&power.join("hidpp_battery_0/type"), "Battery\n");
        write(&power.join("hidpp_battery_0/scope"), "Device\n");
        write(&power.join("hidpp_battery_0/capacity"), "12\n");
        write(&power.join("BAT0/type"), "Battery\n");
        write(&power.join("BAT0/capacity"), "83\n");
        write(&power.join("BAT0/status"), "Charging\n");
        let battery = sources.read().battery;
        assert_eq!(
            battery,
            Some(Battery {
                percent: 83,
                charging: true,
                plugged: false,
            })
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_layout_is_shown_short() {
        assert_eq!(shown_layout("fr"), "FR");
        assert_eq!(shown_layout("us,fr"), "US");
        assert_eq!(shown_layout(" ch ,fr"), "CH");
        assert_eq!(shown_layout("<b>evil</b>"), "BEV");
    }
}
