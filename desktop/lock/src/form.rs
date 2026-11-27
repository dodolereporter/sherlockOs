//! The form of the lock screen comes on demand, as on macOS (lot D-25b). At rest, the screen
//! shows only the background, the date and the time; a key, a click or the pointer bring the
//! bubble of Sherlock, the name, the password pill and the machine buttons, and after
//! `FORM_TIMEOUT` without input they leave again, the field emptied.
//!
//! The key that brings the form is not lost: a letter goes into the field; Space and Enter only
//! bring it. Escape sends it away. Pure logic, without iced: the tests drive it directly.
//!
//! `mood` says what Sherlock shows in his bubble, from the state of the form and of the field.

use std::time::Duration;

use sherlock_3d::Mood;
use sherlock_ui::lock::screen::{FORM_TIMEOUT, SORRY};
use system_services::secret::Secret;

use crate::app::Phase;

/// A jump of the time since boot between two ticks of a second beyond which the machine
/// slept: the form leaves and the field empties at wake-up.
pub const WAKE_GAP: Duration = Duration::from_secs(5);

/// An input that may bring the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The pointer moved, or a button was pressed.
    Pointer,
    /// A key, with the text it types, if any: maybe the first character of the password, so a
    /// `Secret` (`Debug` prints `***`, wiped when dropped).
    Key { key: Key, text: Option<Secret> },
}

/// The keys the form treats apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Space,
    Enter,
    Escape,
    Other,
}

/// What an input does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing changes but the time of the last input.
    Nothing,
    /// The form comes; `typed` goes into the field.
    Shown { typed: Option<Secret> },
    /// The form leaves (Escape).
    Hidden,
}

/// Whether the form shows, since when, and when the last input came. Times are counted from the
/// start of the lock screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Form {
    shown: bool,
    changed: Option<Duration>,
    last_input: Duration,
}

impl Form {
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// When the form last came or left; `None` if it never came.
    pub fn changed(&self) -> Option<Duration> {
        self.changed
    }

    /// An input at `now`.
    pub fn input(&mut self, now: Duration, input: Input) -> Outcome {
        self.last_input = now;
        match (self.shown, input) {
            (
                true,
                Input::Key {
                    key: Key::Escape, ..
                },
            ) => {
                self.set(false, now);
                Outcome::Hidden
            }
            (true, _) => Outcome::Nothing,
            (
                false,
                Input::Key {
                    key: Key::Escape, ..
                },
            ) => Outcome::Nothing,
            (false, Input::Pointer) => {
                self.set(true, now);
                Outcome::Shown { typed: None }
            }
            (false, Input::Key { key, text }) => {
                self.set(true, now);
                // Space and Enter only bring the form; a letter, a digit or a sign starts the
                // password. Control characters (Tab, Backspace…) type nothing.
                let typed = text.filter(|t| {
                    key == Key::Other && !t.is_empty() && !t.as_str().chars().any(char::is_control)
                });
                Outcome::Shown { typed }
            }
        }
    }

    /// The clock at `now`: the form leaves after `FORM_TIMEOUT` without input, unless `busy`
    /// (PAM checking, the screen leaving). Returns `true` when it just left.
    pub fn tick(&mut self, now: Duration, busy: bool) -> bool {
        if self.shown && !busy && now.saturating_sub(self.last_input) >= FORM_TIMEOUT {
            self.set(false, now);
            return true;
        }
        false
    }

    /// Sends the form away at `now` (the machine goes to sleep, or wakes up).
    pub fn hide(&mut self, now: Duration) {
        if self.shown {
            self.set(false, now);
        }
    }

    fn set(&mut self, shown: bool, now: Duration) {
        self.shown = shown;
        self.changed = Some(now);
    }
}

/// What the field and the screen are doing, for Sherlock's mood.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub phase: Phase,
    /// Nothing typed.
    pub empty: bool,
    /// The eye shows the password.
    pub peek: bool,
    /// Something was typed since the form came.
    pub typed: bool,
    /// The machine goes to sleep.
    pub asleep: bool,
}

/// What Sherlock shows at `now`:
///
/// - the form just came: he waves (`Wave`), then rests;
/// - something is typed: eyes closed (`EyesClosed`), as long as the field is not empty;
/// - the eye shows the password: one eye open (`Peek`);
/// - the field emptied again: eyes open (`Idle`), and his gaze goes to the field (`gaze`);
/// - PAM checks: `Think`; refused: `Sorry` for `SORRY`; accepted: `Success`;
/// - the machine goes to sleep: `Sleep`.
pub fn mood(field: Field, now: Duration) -> Mood {
    match field.phase {
        _ if field.asleep => Mood::Sleep,
        Phase::Leaving(_) => Mood::Success,
        Phase::Checking(_) => Mood::Think,
        Phase::Refused(at, _) if now.saturating_sub(at) < SORRY => Mood::Sorry,
        _ if field.peek => Mood::Peek,
        _ if !field.empty => Mood::EyesClosed,
        _ if !field.typed => Mood::Wave,
        _ => Mood::Idle,
    }
}

/// Sherlock looks at the field when it is empty again after typing, while he rests.
pub fn gaze(field: Field, now: Duration) -> bool {
    field.empty && field.typed && mood(field, now) == Mood::Idle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Refusal;

    fn key(key: Key, text: &str) -> Input {
        Input::Key {
            key,
            text: Some(Secret::new(text.to_owned())),
        }
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    #[test]
    fn a_letter_brings_the_form_and_goes_into_the_field() {
        let mut form = Form::default();
        assert!(!form.shown());
        let typed = Some(Secret::new("a".to_owned()));
        assert_eq!(
            form.input(at(10), key(Key::Other, "a")),
            Outcome::Shown { typed }
        );
        assert!(form.shown());
        // Once shown, the field gets the keys itself.
        assert_eq!(form.input(at(20), key(Key::Other, "b")), Outcome::Nothing);
    }

    #[test]
    fn space_and_enter_only_bring_the_form() {
        for (k, text) in [(Key::Space, " "), (Key::Enter, "\r")] {
            let mut form = Form::default();
            assert_eq!(
                form.input(at(10), key(k, text)),
                Outcome::Shown { typed: None }
            );
        }
        // A key that types nothing, or a control character, brings it too.
        let mut form = Form::default();
        let tab = key(Key::Other, "\t");
        assert_eq!(form.input(at(10), tab), Outcome::Shown { typed: None });
        let mut form = Form::default();
        let shift = Input::Key {
            key: Key::Other,
            text: None,
        };
        assert_eq!(form.input(at(10), shift), Outcome::Shown { typed: None });
    }

    #[test]
    fn the_pointer_brings_the_form_and_escape_sends_it_away() {
        let mut form = Form::default();
        assert_eq!(form.input(at(10), key(Key::Escape, "")), Outcome::Nothing);
        assert!(!form.shown());
        assert_eq!(
            form.input(at(20), Input::Pointer),
            Outcome::Shown { typed: None }
        );
        assert_eq!(form.input(at(30), key(Key::Escape, "")), Outcome::Hidden);
        assert!(!form.shown());
        assert_eq!(form.changed(), Some(at(30)));
    }

    #[test]
    fn without_input_the_form_leaves_unless_pam_is_checking() {
        let mut form = Form::default();
        form.input(at(0), Input::Pointer);
        let late = FORM_TIMEOUT + at(1);
        assert!(!form.tick(FORM_TIMEOUT - at(1), false));
        assert!(!form.tick(late, true), "busy: it stays");
        assert!(form.shown());
        assert!(form.tick(late, false));
        assert!(!form.shown());
        assert!(!form.tick(late + at(1), false), "already gone");
        // An input brings it back and starts the wait again.
        form.input(late + at(5), Input::Pointer);
        assert!(!form.tick(late + FORM_TIMEOUT, false));
    }

    fn field(phase: Phase) -> Field {
        Field {
            phase,
            empty: true,
            peek: false,
            typed: false,
            asleep: false,
        }
    }

    #[test]
    fn sherlock_follows_the_field() {
        let now = at(10_000);
        let rest = field(Phase::Typing);
        assert_eq!(mood(rest, now), Mood::Wave);
        let typing = Field {
            empty: false,
            typed: true,
            ..rest
        };
        assert_eq!(mood(typing, now), Mood::EyesClosed);
        assert_eq!(
            mood(
                Field {
                    peek: true,
                    ..typing
                },
                now
            ),
            Mood::Peek
        );
        let emptied = Field {
            empty: true,
            ..typing
        };
        assert_eq!(mood(emptied, now), Mood::Idle);
        assert!(gaze(emptied, now));
        assert!(!gaze(typing, now));
        assert_eq!(mood(field(Phase::Checking(now)), now), Mood::Think);
        let refused = field(Phase::Refused(now, Refusal::Wrong));
        assert_eq!(mood(refused, now + at(100)), Mood::Sorry);
        assert_eq!(
            mood(
                Field {
                    typed: true,
                    ..refused
                },
                now + SORRY
            ),
            Mood::Idle
        );
        assert_eq!(mood(field(Phase::Leaving(now)), now), Mood::Success);
        assert_eq!(
            mood(
                Field {
                    asleep: true,
                    ..rest
                },
                now
            ),
            Mood::Sleep
        );
    }
}
