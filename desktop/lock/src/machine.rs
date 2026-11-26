//! The machine buttons of the lock screen: « Veille », « Redémarrer », « Éteindre ». Each one
//! asks for a second click, as the session buttons of the bar do (`sherlock_ui::launcher::
//! CONFIRM_DELAY` at least between the two). The command runs through `systemctl`, directly,
//! without a shell; logind decides whether it is allowed. The tests use a fake runner.

use std::process::Command;
use std::time::{Duration, Instant};

use sherlock_ui::launcher::CONFIRM_DELAY;

/// What a machine button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    Sleep,
    Reboot,
    PowerOff,
}

impl Machine {
    pub const ALL: [Self; 3] = [Self::Sleep, Self::Reboot, Self::PowerOff];

    /// The `systemctl` verb.
    pub fn verb(self) -> &'static str {
        match self {
            Self::Sleep => "suspend",
            Self::Reboot => "reboot",
            Self::PowerOff => "poweroff",
        }
    }
}

/// What runs a machine command.
pub trait Runner: Send + Sync + 'static {
    fn run(&self, machine: Machine);
}

/// `systemctl <verb>`, waited for on a thread of its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct Systemctl;

impl Runner for Systemctl {
    fn run(&self, machine: Machine) {
        match Command::new("systemctl").arg(machine.verb()).spawn() {
            Ok(mut child) => {
                let _ = std::thread::Builder::new()
                    .name("systemctl".into())
                    .spawn(move || child.wait());
            }
            Err(err) => tracing::warn!(verb = machine.verb(), "systemctl: {err}"),
        }
    }
}

/// The button armed by a first click, and when.
#[derive(Debug, Clone, Copy, Default)]
pub struct Armed(Option<(Machine, Instant)>);

impl Armed {
    pub fn get(&self) -> Option<Machine> {
        self.0.map(|(machine, _)| machine)
    }

    /// A click on `machine` at `now`: `true` when it confirms the armed one, after the delay;
    /// otherwise the button is armed (again).
    pub fn click(&mut self, machine: Machine, now: Instant) -> bool {
        let confirms = matches!(self.0, Some((armed, at))
            if armed == machine && now.duration_since(at) >= CONFIRM_DELAY);
        self.0 = (!confirms).then_some((machine, now));
        confirms
    }

    pub fn disarm(&mut self) {
        self.0 = None;
    }

    /// Disarms a button armed more than `ARMED_FOR` before `now`.
    pub fn expire(&mut self, now: Instant) {
        if self
            .0
            .is_some_and(|(_, at)| now.duration_since(at) > ARMED_FOR)
        {
            self.0 = None;
        }
    }
}

/// How long an armed button waits for its second click.
pub const ARMED_FOR: Duration = Duration::from_secs(4);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_button_needs_a_second_click_after_the_delay() {
        let start = Instant::now();
        let mut armed = Armed::default();
        assert!(!armed.click(Machine::PowerOff, start));
        assert_eq!(armed.get(), Some(Machine::PowerOff));
        // A double click is too quick: it arms again.
        assert!(!armed.click(Machine::PowerOff, start + Duration::from_millis(100)));
        let later = start + Duration::from_millis(100) + CONFIRM_DELAY;
        assert!(armed.click(Machine::PowerOff, later));
        assert_eq!(armed.get(), None);
        // Another button arms itself instead.
        assert!(!armed.click(Machine::Sleep, later));
        assert!(!armed.click(Machine::Reboot, later + CONFIRM_DELAY));
        assert_eq!(armed.get(), Some(Machine::Reboot));
    }
}
