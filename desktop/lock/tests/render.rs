//! The lock screen drawn without a screen, with a fake PAM and a fake `systemctl`: never the
//! system's PAM, never a real password. The screenshots go to `target/tmp/lock/`, on the
//! pictures of the lock: each state (rest without the form, the form shown, typing, the
//! password shown, spinner, error) in the dark and the light theme on 1920 × 1080 and 1366 × 768
//! screens, the rest and the form on the four screens of the brief (1366 × 768, 1920 × 1080, 2560 × 1600 at
//! 1.5, 3840 × 2160 at 2, drawn at their logical size), and a second screen, which shows the
//! time only. In software, Sherlock is his still picture.

// In a test module: the helpers may unwrap like the tests (clippy.toml).
#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use iced::{Size, window};
    use iced_test::simulator::Simulator;
    use lock::app::{Clock, Lock, Message, Phase};
    use lock::form::{Input, Key};
    use lock::machine::{Machine, Runner};
    use lock::pam::{AuthError, Authenticator};
    use sherlock_3d::Mood;
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

    /// The lock on two screens of `size`, the first one under the pointer, at rest: the form
    /// that comes with the lock sent away.
    fn screen(size: (f32, f32)) -> Screen {
        let mut s = arriving(size);
        key(&mut s, Key::Escape, "\u{1b}");
        assert!(!s.lock.form_shown());
        s
    }

    /// The lock as it comes, the form with it.
    fn arriving(size: (f32, f32)) -> Screen {
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
        // The pictures of the lock, at the size of the snapshot (drawn at scale 2).
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/lock");
        let pixels = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
        let fitted = |name| lock::background::fitted(&assets.join(name), pixels).unwrap();
        let pictures = lock::background::Pictures {
            dark: fitted(lock::background::DARK),
            light: fitted(lock::background::LIGHT),
        };
        let _ = lock.update(Message::Background(pictures));
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

    /// Lays `frames` side by side, `columns` per row, each reduced to a quarter of its side,
    /// into the board `name`.
    fn board(frames: &[image::RgbaImage], columns: u32, name: &str) -> PathBuf {
        let (w, h) = (frames[0].width() / 4, frames[0].height() / 4);
        let rows = (frames.len() as u32).div_ceil(columns);
        let gap = 8;
        let mut sheet = image::RgbaImage::from_pixel(
            columns * (w + gap) + gap,
            rows * (h + gap) + gap,
            image::Rgba([16, 16, 16, 255]),
        );
        for (index, frame) in frames.iter().enumerate() {
            let small = image::imageops::resize(frame, w, h, image::imageops::FilterType::Triangle);
            let (x, y) = (index as u32 % columns, index as u32 / columns);
            image::imageops::overlay(
                &mut sheet,
                &small,
                i64::from(gap + x * (w + gap)),
                i64::from(gap + y * (h + gap)),
            );
        }
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("lock")
            .join(format!("{name}.png"));
        sheet.save(&path).unwrap();
        path
    }

    /// `picture` reduced to cover `size` and cut to it, centered: how the compositor draws it.
    fn cover(path: &std::path::Path, size: (u32, u32)) -> image::RgbaImage {
        let picture = lock::background::fitted(path, size).unwrap();
        let full = image::RgbaImage::from_raw(picture.width, picture.height, picture.rgba).unwrap();
        let (x, y) = ((full.width() - size.0) / 2, (full.height() - size.1) / 2);
        image::imageops::crop_imm(&full, x, y, size.0, size.1).to_image()
    }

    /// What the compositor shows while the curtain moves: `under`, and the curtain over it,
    /// down by `down` of the screen (`sherlock_ui::lock::curtain`, which the compositor draws
    /// with).
    fn curtained(
        under: &image::RgbaImage,
        curtain: &image::RgbaImage,
        down: f32,
    ) -> image::RgbaImage {
        let mut frame = under.clone();
        let top = -((1.0 - down) * curtain.height() as f32).round() as i64;
        image::imageops::overlay(&mut frame, curtain, 0, top);
        frame
    }

    /// The form comes with the pointer, and its animations end.
    fn show_form(s: &mut Screen) {
        let _ = s.lock.update(Message::Input(s.primary, Input::Pointer));
        assert!(s.lock.form_shown());
        s.lock.set_now(Instant::now() + Duration::from_secs(4));
    }

    fn key(s: &mut Screen, key: Key, text: &str) {
        let text = Some(Secret::new(text.to_owned()));
        let _ = s
            .lock
            .update(Message::Input(s.primary, Input::Key { key, text }));
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
        let themes = [(color::Theme::Dark, ""), (color::Theme::Light, "-light")];
        let sizes = [((1920.0, 1080.0), ""), ((1366.0, 768.0), "-1366x768")];
        for ((which, theme), (size, name)) in themes
            .into_iter()
            .flat_map(|t| sizes.into_iter().map(move |s| (t, s)))
        {
            let suffix = format!("{name}{theme}");
            color::set_active(which);
            let mut s = screen(size);
            snapshot(&s.lock, s.primary, size, &format!("rest{suffix}"));
            show_form(&mut s);
            snapshot(&s.lock, s.primary, size, &format!("form{suffix}"));
            snapshot(&s.lock, s.other, size, &format!("other-screen{suffix}"));

            let _ = s
                .lock
                .update(Message::Typed(Secret::new("motdepas".into())));
            snapshot(&s.lock, s.primary, size, &format!("typing{suffix}"));
            let _ = s.lock.update(Message::ShowPassword);
            snapshot(&s.lock, s.primary, size, &format!("peek{suffix}"));
            let _ = s.lock.update(Message::ShowPassword);

            let _ = s.lock.update(Message::Submit);
            assert!(matches!(s.lock.phase(), Phase::Checking(_)));
            assert_eq!(s.lock.password_len(), 0, "the field empties at once");
            s.lock.set_now(Instant::now() + Duration::from_millis(4300));
            snapshot(&s.lock, s.primary, size, &format!("spinner{suffix}"));

            checked(&mut s, Err(AuthError::Refused));
            assert!(matches!(s.lock.phase(), Phase::Refused(..)));
            // In the middle of the shake.
            s.lock.set_now(Instant::now() + Duration::from_millis(4350));
            snapshot(&s.lock, s.primary, size, &format!("error{suffix}"));
        }
        color::set_active(color::Theme::Dark);
        // The two other screens of the brief, in the same test: the theme is a global.
        for (size, name) in [
            ((2560.0 / 1.5, 1600.0 / 1.5), "2560x1600-scale-150"),
            ((1920.0, 1080.0), "3840x2160-scale-200"),
        ] {
            let mut s = screen(size);
            snapshot(&s.lock, s.primary, size, &format!("rest-{name}"));
            show_form(&mut s);
            snapshot(&s.lock, s.primary, size, &format!("form-{name}"));
            snapshot(&s.lock, s.other, size, &format!("other-screen-{name}"));
        }
    }

    /// The boards of the lock and of the unlock, at 1920 × 1080 in the dark theme, composed as
    /// the compositor shows them (lot D-24d): the last frame of the desktop, frozen, the curtain
    /// coming down over it, then the lock screen on the same picture, its elements one after the
    /// other (8 frames); once unlocked, the elements leaving in reverse order, then the curtain
    /// going up off the live desktop (6 frames). The desktop is a wallpaper of the desktop.
    #[test]
    #[ignore = "boards for the review: cargo test -p lock --test render -- --ignored boards"]
    fn boards_of_the_lock_and_the_unlock() {
        use sherlock_ui::lock::{CURTAIN, curtain};
        let size = (1920.0, 1080.0);
        let pixels = (1920, 1080);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let desktop = cover(&root.join("wallpapers/first-clue.jpg"), pixels);
        let lock_picture = cover(&root.join("lock").join(lock::background::DARK), pixels);
        let mut s = arriving(size);
        let shot = |s: &Screen, name: &str| {
            image::open(snapshot(&s.lock, s.primary, size, name))
                .unwrap()
                .to_rgba8()
        };
        let ms = Duration::from_millis;
        let mut lock_frames = Vec::new();
        // The curtain comes down; the lock screen is not shown before it is down.
        for t in [0, 130, 260] {
            lock_frames.push(curtained(&desktop, &lock_picture, curtain(ms(t), true)));
        }
        for t in [400, 480, 560, 680, 1100] {
            s.lock.set_elapsed(ms(t).max(CURTAIN));
            lock_frames.push(shot(&s, &format!("lock-{t}ms")));
        }
        assert_eq!(lock_frames.len(), 8);
        board(&lock_frames, 4, "board-lock");

        s.lock.set_elapsed(Duration::from_secs(3));
        let _ = s.lock.update(Message::Typed(Secret::new(FAKE.into())));
        let _ = s.lock.update(Message::Submit);
        checked(&mut s, Ok(()));
        let leaving = Duration::from_secs(3);
        let mut unlock_frames = Vec::new();
        for t in [0, 120, 260, 500] {
            s.lock.set_elapsed(leaving + ms(t));
            unlock_frames.push(shot(&s, &format!("unlock-{t}ms")));
        }
        // Unlocked: the curtain goes up off the live desktop.
        for t in [150, CURTAIN.as_millis() as u64] {
            unlock_frames.push(curtained(&desktop, &lock_picture, curtain(ms(t), false)));
        }
        board(&unlock_frames, 3, "board-unlock");
    }

    #[test]
    fn the_right_password_unlocks_and_the_wrong_one_empties_the_field() {
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
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
        // An empty field sends nothing to PAM, nor a field hidden with the form.
        let mut s = screen((1920.0, 1080.0));
        let _ = s.lock.update(Message::Submit);
        show_form(&mut s);
        let _ = s.lock.update(Message::Submit);
        assert_eq!(s.lock.phase(), Phase::Typing);
        assert!(s.pam.0.lock().unwrap().is_empty());
    }

    #[test]
    fn the_password_never_shows_in_a_message() {
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
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

    /// The form comes with the lock, its elements once the curtain of the compositor is down;
    /// Sherlock waves.
    #[test]
    fn the_form_comes_with_the_lock() {
        let s = arriving((1920.0, 1080.0));
        assert!(s.lock.form_shown());
        assert_eq!(s.lock.mood(), Mood::Wave);
    }

    /// While keys come in the field, Sherlock looks down at it, eyes closed or peeking; about
    /// 1.5 s after the last one, he looks ahead again.
    #[test]
    fn sherlock_looks_at_the_field_while_typing() {
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
        assert!(!s.lock.looks_at_field());
        let _ = s.lock.update(Message::Typed(Secret::new("ab".into())));
        assert!(s.lock.looks_at_field());
        assert_eq!(s.lock.mood(), Mood::EyesClosed);
        let _ = s.lock.update(Message::ShowPassword);
        assert!(s.lock.looks_at_field());
        assert_eq!(s.lock.mood(), Mood::Peek);
        s.lock.set_now(
            Instant::now()
                + Duration::from_secs(4)
                + sherlock_ui::lock::screen::TYPING_GAZE
                + Duration::from_millis(50),
        );
        assert!(!s.lock.looks_at_field());
        assert_eq!(s.lock.mood(), Mood::Peek, "the mood stays");
    }

    #[test]
    fn a_letter_brings_the_form_and_starts_the_password() {
        let mut s = screen((1920.0, 1080.0));
        assert!(!s.lock.form_shown());
        key(&mut s, Key::Other, "p");
        assert!(s.lock.form_shown());
        assert_eq!(s.lock.password_len(), 1, "the letter is not lost");
        assert_eq!(s.lock.mood(), Mood::EyesClosed);
        // Once shown, the keys go to the field itself, not twice.
        key(&mut s, Key::Other, "a");
        assert_eq!(s.lock.password_len(), 1);
    }

    #[test]
    fn space_and_enter_only_bring_the_form() {
        for (k, text) in [(Key::Space, " "), (Key::Enter, "\r")] {
            let mut s = screen((1920.0, 1080.0));
            key(&mut s, k, text);
            assert!(s.lock.form_shown());
            assert_eq!(s.lock.password_len(), 0);
            assert_eq!(s.lock.phase(), Phase::Typing, "Enter sends nothing to PAM");
            assert_eq!(s.lock.mood(), Mood::Wave);
        }
    }

    #[test]
    fn without_input_the_form_leaves_and_the_field_empties() {
        let mut s = screen((1920.0, 1080.0));
        key(&mut s, Key::Other, "p");
        let _ = s.lock.update(Message::ShowPassword);
        let quiet = sherlock_ui::lock::screen::FORM_TIMEOUT + Duration::from_secs(3);
        let boot = Duration::from_secs(100);
        let _ = s.lock.update(Message::Tick(Instant::now() + quiet, boot));
        assert!(!s.lock.form_shown());
        assert_eq!(s.lock.password_len(), 0);
        // Escape sends it away too.
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
        key(&mut s, Key::Escape, "\u{1b}");
        assert!(!s.lock.form_shown());
    }

    #[test]
    fn sherlock_follows_the_field() {
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
        assert_eq!(s.lock.mood(), Mood::Wave);
        let _ = s.lock.update(Message::Typed(Secret::new("ab".into())));
        assert_eq!(s.lock.mood(), Mood::EyesClosed);
        let _ = s.lock.update(Message::ShowPassword);
        assert_eq!(s.lock.mood(), Mood::Peek);
        let _ = s.lock.update(Message::ShowPassword);
        let _ = s.lock.update(Message::Typed(Secret::default()));
        assert_eq!(s.lock.mood(), Mood::Idle, "eyes open again, on the field");
        let _ = s.lock.update(Message::Typed(Secret::new(FAKE.into())));
        let _ = s.lock.update(Message::Submit);
        assert_eq!(s.lock.mood(), Mood::Think);
        checked(&mut s, Ok(()));
        assert_eq!(s.lock.mood(), Mood::Success);
    }

    #[test]
    fn nothing_typed_survives_the_sleep() {
        // « Veille » confirmed with the field filled and the password shown.
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
        let _ = s.lock.update(Message::Typed(Secret::new("presque".into())));
        let _ = s.lock.update(Message::ShowPassword);
        let _ = s.lock.update(Message::Machine(Machine::Sleep));
        std::thread::sleep(sherlock_ui::launcher::CONFIRM_DELAY);
        let _ = s.lock.update(Message::Machine(Machine::Sleep));
        assert_eq!(*s.runner.0.lock().unwrap(), [Machine::Sleep]);
        assert!(!s.lock.form_shown(), "the form leaves");
        assert_eq!(s.lock.password_len(), 0, "the field is empty at wake-up");
        assert_eq!(s.lock.mood(), Mood::Sleep);
        // Shown again, the eye is closed.
        show_form(&mut s);
        let _ = s.lock.update(Message::Typed(Secret::new("x".into())));
        assert_eq!(s.lock.mood(), Mood::EyesClosed, "not Peek: the eye closed");

        // The lid closed: the monotonic clock stops, the time since boot jumps.
        let mut s = screen((1920.0, 1080.0));
        show_form(&mut s);
        let now = Instant::now() + Duration::from_secs(4);
        let boot = Duration::from_secs(1000);
        let _ = s.lock.update(Message::Tick(now, boot));
        let _ = s.lock.update(Message::Typed(Secret::new("presque".into())));
        let _ = s.lock.update(Message::Tick(
            now + Duration::from_secs(1),
            boot + Duration::from_secs(1),
        ));
        assert!(s.lock.form_shown(), "a regular tick changes nothing");
        assert_eq!(s.lock.password_len(), 7);
        let woken = boot + Duration::from_secs(3600);
        let _ = s
            .lock
            .update(Message::Tick(now + Duration::from_secs(2), woken));
        assert!(!s.lock.form_shown());
        assert_eq!(s.lock.password_len(), 0, "the field is empty at wake-up");
    }

    #[test]
    fn the_first_key_never_shows_in_a_message() {
        let input = Input::Key {
            key: Key::Other,
            text: Some(Secret::new("q".into())),
        };
        let shown = format!("{:?}", Message::Input(iced::window::Id::unique(), input));
        assert!(shown.contains("***") && !shown.contains('q'), "{shown}");
    }

    #[test]
    fn the_rescue_mode_unlocks_with_pam_and_keeps_its_field() {
        let (pam, runner) = (
            Arc::new(FakePam::default()),
            Arc::new(FakeRunner::default()),
        );
        let mut lock = Lock::new("dodo".into(), pam.clone(), runner.clone()).rescue();
        let id = window::Id::unique();
        let _ = lock.update(Message::Surface(id));
        let _ = lock.update(Message::Resized(id, Size::new(1366.0, 768.0)));
        assert!(lock.form_shown(), "the field shows at once");
        lock.set_now(Instant::now() + Duration::from_secs(2));
        snapshot(&lock, id, (1366.0, 768.0), "rescue");
        // Escape and a long quiet do not send the field away.
        let escape = Input::Key {
            key: Key::Escape,
            text: None,
        };
        let _ = lock.update(Message::Input(id, escape));
        let quiet = Instant::now() + sherlock_ui::lock::screen::FORM_TIMEOUT * 2;
        let _ = lock.update(Message::Tick(quiet, Duration::from_secs(10)));
        assert!(lock.form_shown());
        let mut s = Screen {
            lock,
            primary: id,
            other: window::Id::unique(),
            pam,
            runner,
        };
        // A wrong password is refused, the right one unlocks: PAM as in the normal mode.
        let _ = s.lock.update(Message::Typed(Secret::new("faux".into())));
        let _ = s.lock.update(Message::Submit);
        checked(&mut s, Err(AuthError::Refused));
        assert_eq!(s.lock.password_len(), 0);
        s.pam.0.lock().unwrap().clear();
        let _ = s.lock.update(Message::Typed(Secret::new(FAKE.into())));
        let _ = s.lock.update(Message::Submit);
        checked(&mut s, Ok(()));
        assert!(matches!(s.lock.phase(), Phase::Leaving(_)));
    }
}
