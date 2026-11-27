//! The sound page's view of PipeWire, through `pactl` (`system_services::audio`): the outputs
//! and the microphones, the default of each, and their volumes. Each call runs off the
//! interface thread; the tests give it a fake `pactl`.

use system_services::Problem;
use system_services::audio::{Device, Endpoint, Pactl};

/// The devices of one direction: what is there, which one is the default, its volume.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Side {
    pub devices: Vec<Endpoint>,
    pub default: String,
    pub volume: u8,
}

/// What the page shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Audio {
    pub output: Side,
    pub input: Side,
}

impl Audio {
    /// Shows `change` before `pactl` makes it, so that the page answers at once.
    pub fn show(&mut self, change: &Change) {
        let (Change::Default(device, _) | Change::Volume(device, _)) = change;
        let side = match device {
            Device::Output => &mut self.output,
            Device::Microphone => &mut self.input,
        };
        match change {
            Change::Default(_, name) => side.default = name.clone(),
            Change::Volume(_, v) => side.volume = *v,
        }
    }
}

fn side(pactl: &Pactl, device: Device) -> Result<Side, Problem> {
    let devices = pactl.endpoints(device)?;
    let default = pactl.default_name(device)?;
    // No microphone: no volume to read.
    let volume = match devices.is_empty() {
        true => 0,
        false => pactl.sound(device)?.volume.min(100) as u8,
    };
    Ok(Side {
        devices,
        default,
        volume,
    })
}

/// Reads both directions.
pub fn read(pactl: &Pactl) -> Result<Audio, Problem> {
    Ok(Audio {
        output: side(pactl, Device::Output)?,
        input: side(pactl, Device::Microphone)?,
    })
}

/// A change asked by the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Default(Device, String),
    Volume(Device, u8),
}

/// Makes the change, then reads everything again: the device chosen may come with its own
/// volume.
pub fn apply(pactl: &Pactl, change: &Change) -> Result<Audio, Problem> {
    match change {
        Change::Default(device, name) => pactl.set_default(*device, name)?,
        Change::Volume(device, percent) => pactl.set_volume_of(*device, *percent)?,
    }
    read(pactl)
}
