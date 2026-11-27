//! The login screen drawn without a screen, with a fake greetd, a fake memory and a fake
//! `systemctl`: never the system's greetd or PAM, never a real password. The screenshots go to
//! `target/tmp/greeter/`, on the pictures of the lock: at rest, the choice of a user, typing and
//! an error, in the dark and the light theme, on 1920 × 1080 and 1366 × 768 screens. In software,
//! Sherlock is his still picture.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use greeter::app::{Greeter, Login, Message, Remember, Step};
    use greeter::login::{Outcome, Report};
    use greeter::memory::Memory;
    use greeter::sessions::Session;
    use greeter::users::User;
    use iced::{Size, window};
    use iced_test::simulator::Simulator;
    use lock::app::{Clock, Phase, Refusal};
    use lock::machine::{Machine, Runner};
    use sherlock_3d::Mood;
    use sherlock_ui::{color, theme};
    use system_services::secret::Secret;

    /// A fake password, never the author's.
    const FAKE: &str = "pas-le-vrai-mot-de-passe";

    /// greetd that accepts `FAKE` for any user, and remembers who asked for which session.
    #[derive(Default)]
    struct FakeGreetd(Mutex<Vec<(String, String)>>);

    impl Login for FakeGreetd {
        fn log_in(
            &self,
            user: &str,
            password: Secret,
            session: &Session,
        ) -> Result<Report, String> {
            self.0
                .lock()
                .unwrap()
                .push((user.to_owned(), session.id.clone()));
            let outcome = match password.as_str() == FAKE {
                true => Outcome::Started,
                false => Outcome::Refused,
            };
            Ok(Report {
                outcome,
                notes: Vec::new(),
            })
        }
    }

    #[derive(Default)]
    struct FakeMemory(Mutex<Option<Memory>>);

    impl Remember for FakeMemory {
        fn save(&self, memory: &Memory) {
            *self.0.lock().unwrap() = Some(memory.clone());
        }
    }

    #[derive(Default)]
    struct FakeRunner(Mutex<Vec<Machine>>);

    impl Runner for FakeRunner {
        fn run(&self, machine: Machine) {
            self.0.lock().unwrap().push(machine);
        }
    }

    fn user(name: &str, real: Option<&str>, uid: u32) -> User {
        User {
            name: name.into(),
            real_name: real.map(Into::into),
            uid,
        }
    }

    fn session(id: &str, name: &str) -> Session {
        Session {
            id: id.into(),
            name: name.into(),
            command: vec![format!("/usr/bin/{id}")],
            desktops: Vec::new(),
        }
    }

    struct Screen {
        greeter: Greeter,
        primary: window::Id,
        greetd: Arc<FakeGreetd>,
        memory: Arc<FakeMemory>,
    }

    /// Three users, the last one to log in being `invite`, on two screens of `size`; with the
    /// pictures of the lock behind for the screenshots (slow to decode in a debug build).
    fn screen(size: (f32, f32)) -> Screen {
        screen_with(size, false)
    }

    fn screen_with(size: (f32, f32), pictures: bool) -> Screen {
        let users = vec![
            user("dodo", Some("Dodo"), 1000),
            user("invite", Some("Invité"), 1001),
            user("camille", None, 1002),
        ];
        let sessions = vec![
            session("sherlockos", "SherlockOS"),
            session("plasma", "Plasma (Wayland)"),
        ];
        let mut memory = Memory::default();
        memory.logged_in("dodo", "plasma");
        memory.logged_in("invite", "sherlockos");
        let (greetd, remember) = (
            Arc::new(FakeGreetd::default()),
            Arc::new(FakeMemory::default()),
        );
        let mut greeter = Greeter::new(
            users,
            sessions,
            memory,
            greetd.clone(),
            remember.clone(),
            Arc::new(FakeRunner::default()),
        );
        let (primary, other) = (window::Id::unique(), window::Id::unique());
        for id in [primary, other] {
            let _ = greeter.update(Message::Surface(id));
            let _ = greeter.update(Message::Resized(id, Size::new(size.0, size.1)));
        }
        greeter.set_clock(Clock {
            weekday: 5,
            day: 27,
            month: 11,
            hour: 8,
            minute: 12,
        });
        if pictures {
            show_pictures(&mut greeter, size);
        }
        greeter.set_now(Instant::now() + Duration::from_secs(3));
        Screen {
            greeter,
            primary,
            greetd,
            memory: remember,
        }
    }

    fn show_pictures(greeter: &mut Greeter, size: (f32, f32)) {
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lock");
        let pixels = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
        let fitted = |name| lock::background::fitted(&assets.join(name), pixels).unwrap();
        let pictures = lock::background::Pictures {
            dark: fitted(lock::background::DARK),
            light: fitted(lock::background::LIGHT),
        };
        let _ = greeter.update(Message::Background(pictures));
    }

    fn snapshot(greeter: &Greeter, id: window::Id, size: (f32, f32), name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("greeter");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.png"));
        let written = dir.join(format!("{name}-tiny-skia.png"));
        let _ = std::fs::remove_file(&written);
        let mut ui = Simulator::with_size(lock::settings(), size, greeter.view(id));
        let snapshot = ui.snapshot(&theme::theme()).unwrap();
        assert!(snapshot.matches_image(&path).unwrap());
        assert!(
            std::fs::metadata(&written).unwrap().len() > 10_000,
            "{name}"
        );
        written
    }

    fn later(s: &mut Screen, seconds: u64) {
        s.greeter
            .set_now(Instant::now() + Duration::from_secs(seconds));
    }

    #[test]
    fn the_last_user_is_selected_with_the_session_they_chose() {
        let mut s = screen((1920.0, 1080.0));
        assert_eq!(s.greeter.selected(), 1);
        assert_eq!(s.greeter.session().unwrap().id, "sherlockos");
        // The arrows go through the users, each with their session.
        let _ = s.greeter.update(Message::Key(Some(Step::Left)));
        assert_eq!(s.greeter.selected(), 0);
        assert_eq!(s.greeter.session().unwrap().id, "plasma");
        let _ = s.greeter.update(Message::Key(Some(Step::Left)));
        assert_eq!(s.greeter.selected(), 0);
        // Past the last user: « Autre… ».
        for _ in 0..5 {
            let _ = s.greeter.update(Message::Key(Some(Step::Right)));
        }
        assert_eq!(s.greeter.selected(), 3);
        // Once something is typed, the arrows stay in the field.
        let _ = s.greeter.update(Message::Select(1));
        let _ = s.greeter.update(Message::Typed(Secret::new("abc".into())));
        let _ = s.greeter.update(Message::Key(Some(Step::Left)));
        assert_eq!(s.greeter.selected(), 1);
        assert_eq!(s.greeter.password_len(), 3);
    }

    #[test]
    fn the_right_password_logs_in_remembers_and_leaves() {
        let mut s = screen((1920.0, 1080.0));
        let _ = s.greeter.update(Message::Typed(Secret::new(FAKE.into())));
        assert_eq!(s.greeter.mood(), Mood::EyesClosed);
        let _ = s.greeter.update(Message::Submit);
        assert!(matches!(s.greeter.phase(), Phase::Checking(_)));
        assert_eq!(s.greeter.mood(), Mood::Think);
        assert_eq!(s.greeter.password_len(), 0);
        // What the thread of greetd answers.
        let answer = FakeGreetd::default().log_in(
            "invite",
            Secret::new(FAKE.into()),
            &session("sherlockos", "SherlockOS"),
        );
        let _ = s.greeter.update(Message::LoggedIn(answer));
        assert!(matches!(s.greeter.phase(), Phase::Leaving(_)));
        assert_eq!(s.greeter.mood(), Mood::Success);
        let saved = s.memory.0.lock().unwrap().clone().unwrap();
        assert_eq!(saved.last_user.as_deref(), Some("invite"));
        assert_eq!(saved.session("invite"), Some("sherlockos"));
        // The screen asked greetd on a thread of its own, for that user and session.
        let asked = || s.greetd.0.lock().unwrap().clone();
        for _ in 0..200 {
            if !asked().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(asked(), [("invite".to_owned(), "sherlockos".to_owned())]);
    }

    #[test]
    fn a_wrong_password_shakes_and_keeps_the_user() {
        let mut s = screen((1920.0, 1080.0));
        let _ = s.greeter.update(Message::Typed(Secret::new("faux".into())));
        let _ = s.greeter.update(Message::Submit);
        let answer = Ok(Report {
            outcome: Outcome::Refused,
            notes: Vec::new(),
        });
        let _ = s.greeter.update(Message::LoggedIn(answer));
        assert!(matches!(
            s.greeter.phase(),
            Phase::Refused(_, Refusal::Wrong)
        ));
        assert_eq!(s.greeter.mood(), Mood::Sorry);
        assert_eq!(s.greeter.selected(), 1);
        assert!(s.memory.0.lock().unwrap().is_none());
        // greetd unreachable: said under the pill.
        let _ = s.greeter.update(Message::Typed(Secret::new("x".into())));
        let _ = s.greeter.update(Message::Submit);
        let _ = s.greeter.update(Message::LoggedIn(Err("no socket".into())));
        assert!(matches!(
            s.greeter.phase(),
            Phase::Refused(_, Refusal::Unavailable)
        ));
        assert_eq!(s.greeter.note(), Some(sherlock_ui::greeter::FAILED));
    }

    #[test]
    fn another_user_needs_a_name() {
        let mut s = screen((1920.0, 1080.0));
        let _ = s.greeter.update(Message::Select(3));
        let _ = s.greeter.update(Message::Typed(Secret::new(FAKE.into())));
        let _ = s.greeter.update(Message::Submit);
        assert_eq!(s.greeter.phase(), Phase::Typing);
        assert_eq!(s.greeter.note(), Some(sherlock_ui::greeter::NO_USER));
        let _ = s.greeter.update(Message::OtherName("  marie ".into()));
        let _ = s.greeter.update(Message::Submit);
        assert!(matches!(s.greeter.phase(), Phase::Checking(_)));
    }

    /// Two screenshots in the CI, whose time is counted: the rest in the dark theme at
    /// 1920 × 1080, the error in the light one at 1366 × 768.
    #[test]
    fn screenshots_at_rest_and_on_error() {
        let mut s = screen_with((1920.0, 1080.0), true);
        snapshot(
            &s.greeter,
            s.primary,
            (1920.0, 1080.0),
            "ci-rest-dark-1920x1080",
        );
        color::set_active(color::Theme::Light);
        let size = (1366.0, 768.0);
        s = screen_with(size, true);
        let _ = s.greeter.update(Message::Typed(Secret::new("faux".into())));
        let _ = s.greeter.update(Message::Submit);
        let refused = Ok(Report {
            outcome: Outcome::Refused,
            notes: Vec::new(),
        });
        let _ = s.greeter.update(Message::LoggedIn(refused));
        snapshot(&s.greeter, s.primary, size, "ci-error-light-1366x768");
        color::set_active(color::Theme::Dark);
    }

    /// The screen at rest in both themes, at 1920 × 1080: the bubble of Sherlock follows the
    /// theme (lot D-25d).
    #[test]
    #[ignore = "screenshots for the review: cargo test -p greeter --test render -- --ignored rest"]
    fn rest_in_both_themes() {
        let size = (1920.0, 1080.0);
        for (theme_now, name) in [(color::Theme::Dark, "dark"), (color::Theme::Light, "light")] {
            color::set_active(theme_now);
            let s = screen_with(size, true);
            snapshot(
                &s.greeter,
                s.primary,
                size,
                &format!("rest-{name}-1920x1080"),
            );
        }
        color::set_active(color::Theme::Dark);
    }

    /// Every state, both themes, both sizes: 24 screenshots for the review, too slow for the
    /// CI in a debug build.
    #[test]
    #[ignore = "screenshots for the review: cargo test -p greeter --test render -- --ignored"]
    fn screenshots_of_each_state() {
        let sizes = [(1920.0, 1080.0), (1366.0, 768.0)];
        let mut written = Vec::new();
        for theme_now in [color::Theme::Dark, color::Theme::Light] {
            color::set_active(theme_now);
            let name = match theme_now {
                color::Theme::Dark => "dark",
                color::Theme::Light => "light",
            };
            for size in sizes {
                let tag = format!("{name}-{}x{}", size.0, size.1);
                let mut s = screen_with(size, true);
                written.push(snapshot(
                    &s.greeter,
                    s.primary,
                    size,
                    &format!("rest-{tag}"),
                ));
                // The choice of a user: the row half way to the next one, then there.
                let _ = s.greeter.update(Message::Key(Some(Step::Right)));
                s.greeter
                    .set_now(Instant::now() + Duration::from_millis(3080));
                let id = s.primary;
                written.push(snapshot(&s.greeter, id, size, &format!("choosing-{tag}")));
                later(&mut s, 5);
                written.push(snapshot(&s.greeter, id, size, &format!("chosen-{tag}")));
                let _ = s
                    .greeter
                    .update(Message::Typed(Secret::new("motdepasse".into())));
                written.push(snapshot(&s.greeter, id, size, &format!("typing-{tag}")));
                let _ = s.greeter.update(Message::Submit);
                let refused = Ok(Report {
                    outcome: Outcome::Refused,
                    notes: Vec::new(),
                });
                let _ = s.greeter.update(Message::LoggedIn(refused));
                // Mid-shake.
                later(&mut s, 5);
                s.greeter
                    .set_now(Instant::now() + Duration::from_millis(5100));
                written.push(snapshot(&s.greeter, id, size, &format!("error-{tag}")));
                // « Autre… » and the other sessions open.
                let _ = s.greeter.update(Message::Select(3));
                let _ = s.greeter.update(Message::Others);
                later(&mut s, 8);
                written.push(snapshot(&s.greeter, id, size, &format!("other-{tag}")));
            }
        }
        color::set_active(color::Theme::Dark);
        assert_eq!(written.len(), 2 * 2 * 6);
    }
}
