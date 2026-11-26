//! The lock screen drawn without a screen, with a fake PAM and a fake `systemctl`: never the
//! system's PAM, never a real password. The screenshots go to `target/tmp/lock/`: each state
//! (rest, typing, error, spinner) in the dark and the light theme on a 1920 × 1080 screen, the
//! rest on the four screens of the brief (1366 × 768, 1920 × 1080, 2560 × 1600 at 1.5, 3840 ×
//! 2160 at 2, drawn at their logical size), and a second screen, which shows the time only.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use iced::{Size, window};
    use iced_test::simulator::Simulator;
    use lock::app::{Clock, Lock, Message, Phase};
    use lock::machine::{Machine, Runner};
    use lock::pam::{AuthError, Authenticator};
    use sherlock_ui::{color, theme};
    use system_services::secret::Secret;

    /// A fake password, never the author's.
    const FAKE: &str = "pas-le-vrai-mot-de-passe";

    /// A PAM that accepts `FAKE` only, and remembers who asked.
    #[derive(Default)]
    struct FakePam(Mutex<Vec<String>>);

    impl Authenticator for FakePam {
        fn check(&self, user: &str, password: Secret) -> Result<(), AuthError> {
            self.0.lock().unwrap().push(user.to_owned());
            match password.as_str() == FAKE {
                true => Ok(()),
                false => Err(AuthError::Refused),
            }
        }
    }

    #[derive(Default)]
    struct FakeRunner(Mutex<Vec<Machine>>);

    impl Runner for FakeRunner {
        fn run(&self, machine: Machine) {
            self.0.lock().unwrap().push(machine);
        }
    }

    struct Screen {
        lock: Lock,
        primary: window::Id,
        other: window::Id,
        pam: Arc<FakePam>,
        runner: Arc<FakeRunner>,
    }

    /// The lock on two screens of `size`, the first one under the pointer, at rest.
    fn screen(size: (f32, f32)) -> Screen {
        let (pam, runner) = (
            Arc::new(FakePam::default()),
            Arc::new(FakeRunner::default()),
        );
        let mut lock = Lock::new("dodo".into(), pam.clone(), runner.clone());
        let (primary, other) = (window::Id::unique(), window::Id::unique());
        for id in [primary, other] {
            let _ = lock.update(Message::Surface(id));
            let _ = lock.update(Message::Resized(id, Size::new(size.0, size.1)));
        }
        lock.set_clock(Clock {
            weekday: 4,
            day: 26,
            month: 11,
            hour: 22,
            minute: 47,
        });
        lock.set_now(Instant::now() + Duration::from_secs(2));
        Screen {
            lock,
            primary,
            other,
            pam,
            runner,
        }
    }

    fn snapshot(lock: &Lock, id: window::Id, size: (f32, f32), name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lock");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        let written = dir.join(format!("{name}-tiny-skia.png"));
        let _ = std::fs::remove_file(&written);
        let mut ui = Simulator::with_size(lock::settings(), size, lock.view(id));
        let snapshot = ui.snapshot(&theme::theme()).unwrap();
        assert!(snapshot.matches_image(&path).unwrap());
        assert!(
            std::fs::metadata(&written).unwrap().len() > 10_000,
            "{name}"
        );
        written
    }

    /// Waits for the fake PAM's answer, as the runtime would.
    fn checked(s: &mut Screen, expected: Result<(), AuthError>) {
        for _ in 0..100 {
            if s.pam.0.lock().unwrap().len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*s.pam.0.lock().unwrap(), ["dodo"]);
        let _ = s.lock.update(Message::Checked(expected));
    }

    #[test]
    fn the_states_render_in_both_themes() {
        let size = (1920.0, 1080.0);
        for (which, suffix) in [(color::Theme::Dark, ""), (color::Theme::Light, "-light")] {
            color::set_active(which);
            let mut s = screen(size);
            snapshot(&s.lock, s.primary, size, &format!("rest{suffix}"));
            snapshot(&s.lock, s.other, size, &format!("other-screen{suffix}"));

            let _ = s.lock.update(Message::Key);
            let _ = s
                .lock
                .update(Message::Typed(Secret::new("motdepas".into())));
            snapshot(&s.lock, s.primary, size, &format!("typing{suffix}"));

            let _ = s.lock.update(Message::Submit);
            assert!(matches!(s.lock.phase(), Phase::Checking(_)));
            assert_eq!(s.lock.password_len(), 0, "the field empties at once");
            s.lock.set_now(Instant::now() + Duration::from_millis(2300));
            snapshot(&s.lock, s.primary, size, &format!("spinner{suffix}"));

            checked(&mut s, Err(AuthError::Refused));
            assert!(matches!(s.lock.phase(), Phase::Refused(..)));
            // In the middle of the shake.
            s.lock.set_now(Instant::now() + Duration::from_millis(2350));
            snapshot(&s.lock, s.primary, size, &format!("error{suffix}"));
        }
        color::set_active(color::Theme::Dark);
        // The screen fits every size, in the same test: the theme is a global.
        for (size, name) in [
            ((1366.0, 768.0), "1366x768"),
            ((1920.0, 1080.0), "1920x1080"),
            ((2560.0 / 1.5, 1600.0 / 1.5), "2560x1600-scale-150"),
            ((1920.0, 1080.0), "3840x2160-scale-200"),
        ] {
            let s = screen(size);
            snapshot(&s.lock, s.primary, size, &format!("rest-{name}"));
            snapshot(&s.lock, s.other, size, &format!("other-screen-{name}"));
        }
    }

    #[test]
    fn the_right_password_unlocks_and_the_wrong_one_empties_the_field() {
        let mut s = screen((1920.0, 1080.0));
        let _ = s.lock.update(Message::Typed(Secret::new("faux".into())));
        let _ = s.lock.update(Message::Submit);
        checked(&mut s, Err(AuthError::Refused));
        assert_eq!(s.lock.password_len(), 0);
        // Typing again leaves the refusal.
        let _ = s.lock.update(Message::Typed(Secret::new(FAKE.into())));
        assert_eq!(s.lock.phase(), Phase::Typing);
        s.pam.0.lock().unwrap().clear();
        let _ = s.lock.update(Message::Submit);
        checked(&mut s, Ok(()));
        assert!(matches!(s.lock.phase(), Phase::Leaving(_)));
        // An empty field sends nothing to PAM.
        let mut s = screen((1920.0, 1080.0));
        let _ = s.lock.update(Message::Submit);
        assert_eq!(s.lock.phase(), Phase::Typing);
        assert!(s.pam.0.lock().unwrap().is_empty());
    }

    #[test]
    fn the_password_never_shows_in_a_message() {
        let s = screen((1920.0, 1080.0));
        let mut ui =
            Simulator::with_size(lock::settings(), (1920.0, 1080.0), s.lock.view(s.primary));
        let _ = ui.click(sherlock_ui::lock::screen::PASSWORD);
        let _ = ui.typewrite(FAKE);
        let messages: Vec<String> = ui.into_messages().map(|m| format!("{m:?}")).collect();
        assert!(messages.iter().any(|m| m == "Typed(***)"), "{messages:?}");
        assert!(messages.iter().all(|m| !m.contains("vrai")), "{messages:?}");
    }

    #[test]
    fn the_field_follows_the_screen_of_the_pointer() {
        let mut s = screen((1920.0, 1080.0));
        assert_eq!(s.lock.primary(), Some(s.primary));
        let _ = s.lock.update(Message::Here(s.other));
        assert_eq!(s.lock.primary(), Some(s.other));
    }

    #[test]
    fn a_machine_button_asks_for_a_second_click() {
        let mut s = screen((1920.0, 1080.0));
        let _ = s.lock.update(Message::Machine(Machine::PowerOff));
        assert_eq!(s.lock.armed(), Some(Machine::PowerOff));
        assert!(s.runner.0.lock().unwrap().is_empty());
        std::thread::sleep(sherlock_ui::launcher::CONFIRM_DELAY);
        let _ = s.lock.update(Message::Machine(Machine::PowerOff));
        assert_eq!(*s.runner.0.lock().unwrap(), [Machine::PowerOff]);
    }

    /// The toolkit unlocks on every clean exit: only PAM's acceptance may lead to `UnLock` and
    /// `iced::exit()`. A refusal, too many tries or PAM unavailable never leave the screen, even
    /// long after; and the source holds a single exit, in the branch of the acceptance.
    #[test]
    fn only_the_acceptance_of_pam_leaves_the_lock() {
        for error in [
            AuthError::Refused,
            AuthError::TooManyTries,
            AuthError::Unavailable("no PAM".into()),
        ] {
            let mut s = screen((1920.0, 1080.0));
            let _ = s.lock.update(Message::Checked(Err(error)));
            let later = Instant::now() + Duration::from_secs(60);
            let _ = s.lock.update(Message::Frame(later));
            assert!(matches!(s.lock.phase(), Phase::Refused(..)));
        }
        let mut s = screen((1920.0, 1080.0));
        let _ = s.lock.update(Message::Checked(Ok(())));
        assert!(matches!(s.lock.phase(), Phase::Leaving(_)));

        let sources = [
            include_str!("../src/app.rs"),
            include_str!("../src/lib.rs"),
            include_str!("../src/main.rs"),
            include_str!("../src/machine.rs"),
            include_str!("../src/pam.rs"),
        ];
        let count = |what: &str| {
            sources
                .iter()
                .map(|s| s.matches(what).count())
                .sum::<usize>()
        };
        assert_eq!(count("(iced::exit())"), 1, "a single clean exit");
        assert_eq!(count("Message::UnLock"), 1, "a single unlock");
        let app = sources[0];
        let exit = app.find("(iced::exit())").unwrap();
        let leaving = app[..exit].rfind("Phase::Leaving(at)").unwrap();
        assert!(
            exit - leaving < 1200,
            "the exit is in the branch of the acceptance"
        );
    }
}
