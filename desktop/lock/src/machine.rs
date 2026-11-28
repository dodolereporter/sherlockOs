//! The machine buttons of the lock screen: « Veille », « Redémarrer », « Éteindre ». Each one
//! asks for a second click, as the session buttons of the bar do (`sherlock_ui::launcher::
//! CONFIRM_DELAY` at least between the two). The command runs through `systemctl`, directly,
//! without a shell; logind decides whether it is allowed. The tests use a fake runner.

use std::process::Command;
use std::time::{Duration, Instant};

use sherlock_ui::launcher::CONFIRM_DELAY;
use sherlock_ui::lock::screen::{DEPART_FADE, DEPART_GIVE_UP, DEPART_WAVE};

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

/// A restart or a power off on its way (lot D-25i): Sherlock waves, the screen fades to black,
/// then the command runs; the screen comes back if the machine is still there long after.
/// Times are since the start of the screen, as its other animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Departure {
    pub machine: Machine,
    at: Duration,
    ran: bool,
}

impl Departure {
    pub fn new(machine: Machine, at: Duration) -> Self {
        Self {
            machine,
            at,
            ran: false,
        }
    }

    /// How black the screen is at `now`, from 0 to 1.
    pub fn black(&self, now: Duration) -> f32 {
        let since = now.saturating_sub(self.at);
        let fading = since.saturating_sub(DEPART_WAVE).as_secs_f32() / DEPART_FADE.as_secs_f32();
        sherlock_ui::motion::Ease::InQuad.apply(fading.clamp(0.0, 1.0))
    }

    /// `true` once, when the screen is black: the command runs then.
    pub fn due(&mut self, now: Duration) -> bool {
        let due = !self.ran && now >= self.at + DEPART_WAVE + DEPART_FADE;
        self.ran |= due;
        due
    }

    /// Still something to draw at `now`: Sherlock's wave and the fade.
    pub fn animating(&self, now: Duration) -> bool {
        now <= self.at + DEPART_WAVE + DEPART_FADE + Duration::from_millis(50)
    }

    /// The machine is still there long after the command: the screen comes back.
    pub fn given_up(&self, now: Duration) -> bool {
        now >= self.at + DEPART_WAVE + DEPART_FADE + DEPART_GIVE_UP
    }

    /// When `given_up` turns true, for a wake-up timer.
    pub fn give_up_in(&self, now: Duration) -> Duration {
        (self.at + DEPART_WAVE + DEPART_FADE + DEPART_GIVE_UP).saturating_sub(now)
    }
}

/// Whether a confirmed machine button leaves with Sherlock's wave and the fade to black.
pub fn departs(machine: Machine) -> bool {
    matches!(machine, Machine::Reboot | Machine::PowerOff)
}

/// Waits `delay` on a thread of its own (never in the tests, which only call `update`).
pub async fn after(delay: Duration) {
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("timer".into())
        .spawn(move || {
            std::thread::sleep(delay);
            let _ = sender.send(());
        });
    if spawned.is_ok() {
        let _ = receiver.await;
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

    #[test]
    fn a_departure_waves_fades_to_black_runs_once_and_gives_up() {
        let at = Duration::from_secs(10);
        let mut departure = Departure::new(Machine::PowerOff, at);
        assert_eq!(
            departure.black(at + DEPART_WAVE / 2),
            0.0,
            "Sherlock waves first"
        );
        assert!(!departure.due(at + DEPART_WAVE));
        let black = at + DEPART_WAVE + DEPART_FADE;
        assert_eq!(departure.black(black), 1.0);
        assert!(departure.due(black));
        assert!(!departure.due(black + Duration::from_secs(1)), "once");
        assert!(!departure.animating(black + Duration::from_secs(1)));
        assert!(!departure.given_up(black + Duration::from_secs(1)));
        assert!(departure.given_up(black + DEPART_GIVE_UP));
        assert!(departs(Machine::Reboot) && !departs(Machine::Sleep));
    }
}
