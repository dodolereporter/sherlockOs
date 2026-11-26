//! The sources of D-22b together: the order of the sections, the limits, the calculator's
//! copy, the confirmation of the session commands (run by a fake session: no real command
//! ever starts), the files of a fake personal folder, the icons of a fake theme, and the
//! panel's rendering without a screen.

// Test module: clippy allows unwrap and expect there, as in unit tests.
#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use iced::keyboard;
    use iced_test::simulator::Simulator;
    use launcher::app::{self, Clipboard, Launcher, Message};
    use launcher::apps::Apps;
    use launcher::ask::Ask;
    use launcher::files::{self, Files};
    use launcher::source::{Action, Command, Power, Query, Source, Tile};
    use launcher::system::{NoSession, Session, System, Systemd};
    use mime_apps::{Catalog, Dirs, MimeDb};
    use sherlock_ui::launcher::{self as look, text as texts};
    use sherlock_ui::theme;

    /// A temporary folder under `CARGO_TARGET_TMPDIR`, removed at the end of the test.
    struct TempDir(PathBuf);

    impl TempDir {
        /// A new folder for every call: the tests run in parallel.
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("sources-{name}-{}-{id}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temporary folder");
            Self(path)
        }

        fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("folder");
            std::fs::write(&path, "").expect("written");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/data")
    }

    fn apps() -> Apps {
        let data = fixtures();
        let dirs = Dirs {
            config_home: data.join("config"),
            config_dirs: Vec::new(),
            data_home: data.clone(),
            data_dirs: Vec::new(),
        };
        let terminal = Some(vec!["test-terminal".into(), "-e".into()]);
        let icons = icon_theme::Lookup::new(&[data], None, None).context("Applications");
        Apps::load(&dirs, &["fr".into()], terminal).with_icons(icons)
    }

    /// A catalog that opens text files with « Éditeur » (`example-editor %F`) and folders with
    /// « Navigateur » (`example-files %U`).
    fn catalog() -> Catalog {
        let entry = |id: &str, text: &str| {
            mime_apps::entry::parse(id, Path::new("/apps"), text, &[]).expect("entry")
        };
        let mut mime = MimeDb::default();
        mime.add_globs("50:text/plain:*.txt\n");
        Catalog {
            applications: vec![
                entry(
                    "editor.desktop",
                    "[Desktop Entry]\nType=Application\nName=Éditeur\nExec=example-editor %F\nMimeType=text/plain;\n",
                ),
                entry(
                    "files.desktop",
                    "[Desktop Entry]\nType=Application\nName=Navigateur\nExec=example-files %U\nMimeType=inode/directory;\n",
                ),
            ],
            mime,
            ..Catalog::default()
        }
    }

    /// A fake personal folder: accents, hidden and heavy folders, a link, deep folders.
    fn home() -> TempDir {
        let tmp = TempDir::new("home");
        tmp.file("Été/Résumé final.txt");
        tmp.file("Projets/compositeur/src/dock.rs");
        tmp.file("Projets/compositeur/Dockerfile");
        tmp.file("Projets/front/node_modules/dock-lib/index.js");
        tmp.file("Projets/compositeur/target/dock-build.txt");
        tmp.file(".cache/dock-cache.txt");
        tmp.file("Projets/.git/dock-object");
        tmp.file("a/b/c/dock-level4.txt");
        tmp.file("a/b/c/d/dock-level5.txt");
        tmp.file("rapport-rendu.txt");
        tmp.file("Notes/eteindre-le-serveur.txt");
        tmp.file("Notes/deconnexion-wifi.txt");
        let _ = std::os::unix::fs::symlink(tmp.0.join("Projets"), tmp.0.join("lien"));
        tmp
    }

    /// Records the starts and the session commands; runs nothing.
    #[derive(Default)]
    struct Record {
        started: Mutex<Vec<Command>>,
        session: Mutex<Vec<Power>>,
    }

    impl Session for Record {
        fn run(&self, power: Power) -> Result<(), launcher::launch::LaunchError> {
            if let Ok(mut done) = self.session.lock() {
                done.push(power);
            }
            Ok(())
        }
    }

    struct SessionOf(Arc<Record>);

    impl Session for SessionOf {
        fn run(&self, power: Power) -> Result<(), launcher::launch::LaunchError> {
            self.0.run(power)
        }
    }

    fn launcher_on(home: &Path, clipboard: Clipboard) -> (Launcher, Arc<Record>) {
        let record = Arc::new(Record::default());
        let starts = Arc::clone(&record);
        let spawn: app::Spawn = Arc::new(move |command: &Command| {
            if let Ok(mut started) = starts.started.lock() {
                started.push(command.clone());
            }
            Ok(1)
        });
        let ask = Ask::new(Some(vec!["test-terminal".into(), "-e".into()]), None);
        let sources = launcher::arrange(apps(), Files::ready(home.to_path_buf(), catalog()), ask);
        let launcher = Launcher::new(sources, spawn)
            .with_session(Arc::new(SessionOf(Arc::clone(&record))))
            .with_clipboard(clipboard);
        (launcher, record)
    }

    fn sections(launcher: &Launcher) -> Vec<&'static str> {
        launcher.sections.iter().map(|s| s.title).collect()
    }

    fn titles(launcher: &Launcher) -> Vec<String> {
        launcher.hits().map(|h| h.title.clone()).collect()
    }

    fn enter(launcher: &mut Launcher) {
        let _ = launcher.update(Message::Key(app::Key::Enter));
    }

    #[test]
    fn the_sections_follow_the_board_and_sherlock_comes_last() {
        let home = home();
        let (mut launcher, _) = launcher_on(&home.0, Clipboard::Toolkit);
        let _ = launcher.update(Message::Query("re".into()));
        assert_eq!(
            sections(&launcher),
            [
                texts::APPLICATIONS,
                texts::SYSTEM,
                texts::FILES,
                texts::SHERLOCK
            ]
        );
        assert_eq!(
            titles(&launcher).last().map(String::as_str),
            Some(texts::ASK)
        );
        let _ = launcher.update(Message::Query("2 + 2".into()));
        assert_eq!(sections(&launcher), [texts::CALCULATOR, texts::SHERLOCK]);
        assert_eq!(titles(&launcher)[0], "4");
        // Nothing matches: Sherlock alone, disabled, « Bientôt ».
        let _ = launcher.update(Message::Query("zzzz".into()));
        let hits: Vec<_> = launcher.hits().cloned().collect();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].subtitle.as_deref(), Some(texts::SOON));
        assert_eq!(hits[0].action, Action::Disabled);
        enter(&mut launcher);
        assert!(!launcher.closing, "a disabled entry does nothing");
        // An empty query shows nothing at all.
        for empty in ["", "   "] {
            let _ = launcher.update(Message::Query(empty.into()));
            assert_eq!(launcher.hits().count(), 0);
            assert_eq!(launcher.height(), look::panel_height(0, 0));
        }
    }

    #[test]
    fn five_results_a_section_and_room_left_for_sherlock() {
        let tmp = TempDir::new("many");
        for i in 0..12 {
            tmp.file(&format!("notes/kite-{i:02}.txt"));
        }
        let (mut launcher, _) = launcher_on(&tmp.0, Clipboard::Toolkit);
        let _ = launcher.update(Message::Query("k".into()));
        for section in &launcher.sections {
            assert!(
                section.hits.len() <= look::SECTION_RESULTS,
                "{}",
                section.title
            );
        }
        assert_eq!(launcher.hits().count(), look::MAX_RESULTS);
        assert_eq!(sections(&launcher).last(), Some(&texts::SHERLOCK));
        assert!(launcher.height() <= look::panel_height(look::MAX_SECTIONS, look::MAX_RESULTS));
    }

    /// Past the delay a confirmation needs.
    fn wait_confirmation_delay() {
        std::thread::sleep(look::CONFIRM_DELAY + std::time::Duration::from_millis(50));
    }

    /// The message an Enter press gives, held down (`repeat`) or not.
    fn enter_key(repeat: bool) -> Option<Message> {
        let named = keyboard::Key::Named(keyboard::key::Named::Enter);
        let event = iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: named.clone(),
            modified_key: named,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat,
        });
        app::on_event(
            event,
            iced::event::Status::Captured,
            iced::window::Id::unique(),
        )
    }

    #[test]
    fn log_out_reboot_and_power_off_wait_for_a_second_enter() {
        let home = home();
        for (query, power) in [
            ("eteindre", Power::PowerOff),
            ("redem", Power::Reboot),
            ("se deco", Power::LogOut),
        ] {
            let (mut launcher, record) = launcher_on(&home.0, Clipboard::Toolkit);
            let _ = launcher.update(Message::Query(query.into()));
            assert_eq!(
                launcher.hits().next().map(|h| h.action.clone()),
                Some(Action::Session(power))
            );
            enter(&mut launcher);
            assert_eq!(launcher.armed(), Some(0));
            assert!(!launcher.closing);
            // Moving away disarms it: the next Enter asks again.
            let _ = launcher.update(Message::Key(app::Key::Down));
            let _ = launcher.update(Message::Key(app::Key::Up));
            assert_eq!(launcher.armed(), None);
            enter(&mut launcher);
            // Too soon: a bounce or a quick second press confirms nothing.
            enter(&mut launcher);
            assert!(record.session.lock().expect("record").is_empty(), "{query}");
            let mut ui = Simulator::with_size(
                launcher::settings(),
                (look::WIDTH, launcher.height()),
                launcher.view_panel(),
            );
            assert!(
                ui.find(texts::CONFIRM).is_ok(),
                "{query}: the line asks to confirm"
            );
            drop(ui);
            wait_confirmation_delay();
            enter(&mut launcher);
            assert_eq!(record.session.lock().expect("record").as_slice(), [power]);
            assert!(launcher.closing);
            // Nothing went through the program starter.
            assert!(record.started.lock().expect("record").is_empty());
        }
        // Locking goes at the first Enter: it loses nothing.
        let (mut launcher, record) = launcher_on(&home.0, Clipboard::Toolkit);
        let _ = launcher.update(Message::Query("verrouiller".into()));
        enter(&mut launcher);
        assert_eq!(
            record.session.lock().expect("record").as_slice(),
            [Power::Lock]
        );
    }

    #[test]
    fn a_held_enter_never_confirms() {
        // A repeated Enter is no key for the launcher at all.
        assert!(enter_key(true).is_none());
        assert!(matches!(
            enter_key(false),
            Some(Message::Key(app::Key::Enter))
        ));
        let home = home();
        let (mut launcher, record) = launcher_on(&home.0, Clipboard::Toolkit);
        let _ = launcher.update(Message::Query("eteindre".into()));
        let _ = launcher.update(enter_key(false).expect("a launcher key"));
        assert_eq!(launcher.armed(), Some(0));
        // Held past the compositor's repeat delay and the confirmation delay.
        wait_confirmation_delay();
        for _ in 0..20 {
            if let Some(message) = enter_key(true) {
                let _ = launcher.update(message);
            }
        }
        assert!(record.session.lock().expect("record").is_empty());
        assert!(!launcher.closing);
    }

    #[test]
    fn a_double_click_never_confirms() {
        let home = home();
        let (mut launcher, record) = launcher_on(&home.0, Clipboard::Toolkit);
        let _ = launcher.update(Message::Query("redemarrer".into()));
        let _ = launcher.update(Message::Pick(0));
        let _ = launcher.update(Message::Pick(0));
        assert_eq!(launcher.armed(), Some(0));
        assert!(record.session.lock().expect("record").is_empty());
        // A deliberate second click, later, does.
        wait_confirmation_delay();
        let _ = launcher.update(Message::Pick(0));
        assert_eq!(
            record.session.lock().expect("record").as_slice(),
            [Power::Reboot]
        );
    }

    #[test]
    fn session_commands_need_their_name_or_a_whole_keyword() {
        let home = home();
        let (mut launcher, _) = launcher_on(&home.0, Clipboard::Toolkit);
        let session_rows = |launcher: &Launcher| {
            launcher
                .hits()
                .filter(|h| matches!(h.action, Action::Session(_)))
                .count()
        };
        // Looks like a command, is none: no system row.
        for query in ["logo", "déco", "quit", "eteind pc", "shut", "dre"] {
            let _ = launcher.update(Message::Query(query.into()));
            assert_eq!(session_rows(&launcher), 0, "{query}");
            assert!(!sections(&launcher).contains(&texts::SYSTEM), "{query}");
        }
        // By its name: before the files.
        let _ = launcher.update(Message::Query("Éteindre".into()));
        assert_eq!(
            sections(&launcher),
            [texts::SYSTEM, texts::FILES, texts::SHERLOCK]
        );
        // By a whole keyword only: after the files.
        let _ = launcher.update(Message::Query("deconnexion".into()));
        assert_eq!(
            sections(&launcher),
            [texts::FILES, texts::SYSTEM, texts::SHERLOCK]
        );
        let _ = launcher.update(Message::Query("logout".into()));
        assert_eq!(sections(&launcher), [texts::SYSTEM, texts::SHERLOCK]);
    }

    #[test]
    fn session_commands_are_built_never_run() {
        let session = Systemd {
            session_id: Some("c2".into()),
        };
        let args = |power| -> (OsString, Vec<OsString>) {
            let command = session.command(power);
            (command.program, command.args)
        };
        assert_eq!(
            args(Power::Lock),
            ("loginctl".into(), vec!["lock-session".into(), "c2".into()])
        );
        assert_eq!(
            args(Power::LogOut),
            (
                "loginctl".into(),
                vec!["terminate-session".into(), "c2".into()]
            )
        );
        assert_eq!(
            args(Power::Reboot),
            ("systemctl".into(), vec!["reboot".into()])
        );
        assert_eq!(
            args(Power::PowerOff),
            ("systemctl".into(), vec!["poweroff".into()])
        );
        // Without a session id, the caller's: an empty argument, as loginctl documents it.
        let own = Systemd { session_id: None }.command(Power::LogOut);
        assert_eq!(
            own.args,
            [OsString::from("terminate-session"), OsString::new()]
        );
        // A launcher given no session refuses every command and stays open.
        let mut launcher = Launcher::new(
            vec![Box::new(System::default())],
            Arc::new(|_: &Command| Ok(1)),
        );
        let _ = launcher.update(Message::Query("verrou".into()));
        enter(&mut launcher);
        assert!(!launcher.closing);
        assert!(NoSession.run(Power::PowerOff).is_err());
        // Scattered letters never offer a session command.
        let terms = |q: &str| Query::new(q);
        assert!(System::default().search(&terms("tdr"), 5).is_empty());
        assert!(System::default().search(&terms("dre"), 5).is_empty());
    }

    #[test]
    fn the_result_is_copied_by_wl_copy_or_by_iced() {
        let home = home();
        let (mut launcher, record) = launcher_on(&home.0, Clipboard::WlCopy("wl-copy".into()));
        let _ = launcher.update(Message::Query("(1,5 + 1,5) * 2".into()));
        enter(&mut launcher);
        let started = record.started.lock().expect("record").clone();
        assert_eq!(started.len(), 1);
        assert_eq!(started[0].program, "wl-copy");
        assert_eq!(started[0].args, [OsString::from("--"), OsString::from("6")]);
        assert!(launcher.closing);
        // Without wl-copy: iced's clipboard, and the launcher stays open to keep the selection.
        let (mut launcher, record) = launcher_on(&home.0, Clipboard::Toolkit);
        let _ = launcher.update(Message::Query("7 / 2".into()));
        enter(&mut launcher);
        assert_eq!(launcher.copied, Some(0));
        assert!(!launcher.closing);
        assert!(record.started.lock().expect("record").is_empty());
        let mut ui = Simulator::with_size(
            launcher::settings(),
            (look::WIDTH, launcher.height()),
            launcher.view_panel(),
        );
        assert!(ui.find("3,5").is_ok() && ui.find(texts::COPIED).is_ok());
    }

    #[test]
    fn files_are_found_by_name_and_opened_with_their_application() {
        let home = home();
        let found = Files::ready(home.0.clone(), catalog());
        let names = |query: &str| -> Vec<(String, Option<String>)> {
            found
                .search(&Query::new(query), 5)
                .into_iter()
                .map(|h| (h.title, h.subtitle))
                .collect()
        };
        // Accents ignored in the query, kept in the name and the place.
        assert_eq!(
            names("resume"),
            [("Résumé final.txt".to_owned(), Some("Été".to_owned()))]
        );
        // Hidden folders, heavy folders and level 5 left out; a link listed, not entered.
        // Equal scores: the shallowest first, then by name.
        assert_eq!(
            names("dock"),
            [
                (
                    "Dockerfile".to_owned(),
                    Some("Projets, compositeur".to_owned())
                ),
                ("dock-level4.txt".to_owned(), Some("a, b, c".to_owned())),
                (
                    "dock.rs".to_owned(),
                    Some("Projets, compositeur, src".to_owned())
                ),
            ]
        );
        assert_eq!(
            names("lien"),
            [("lien".to_owned(), Some(texts::HOME.to_owned()))]
        );
        assert!(names("kcod").is_empty(), "scattered letters do not count");
        // A file opens with the application of its type, the path given as it is.
        let hit = found.search(&Query::new("résumé"), 1).remove(0);
        assert_eq!(hit.tile, Tile::Glyph(look::Glyph::File));
        let Action::Start(command) = hit.action else {
            panic!("an application opens it");
        };
        assert_eq!(command.program, "example-editor");
        assert_eq!(
            command.args,
            [home.0.join("Été/Résumé final.txt").into_os_string()]
        );
        // A folder opens with the folder application; a type nobody opens is disabled.
        let folder = found.search(&Query::new("projets"), 1).remove(0);
        assert_eq!(folder.tile, Tile::Glyph(look::Glyph::Folder));
        assert!(matches!(folder.action, Action::Start(c) if c.program == "example-files"));
        assert_eq!(
            found.search(&Query::new("dockerfile"), 1)[0].action,
            Action::Disabled
        );
    }

    #[test]
    fn the_walk_stops_at_its_limit_and_runs_in_the_background() {
        let tmp = TempDir::new("limit");
        for i in 0..files::MAX_ENTRIES + 10 {
            tmp.file(&format!("many/{i:05}"));
        }
        assert_eq!(files::walk(&tmp.0).len(), files::MAX_ENTRIES);
        // Another device: its folders are listed, never entered.
        let home = home();
        let elsewhere = files::walk_on(&home.0, u64::MAX);
        assert!(elsewhere.iter().all(|e| e.depth == 1), "{elsewhere:?}");
        assert!(
            elsewhere
                .iter()
                .any(|e| e.path.ends_with("Projets") && e.is_dir)
        );
        // In the background: the source answers once the receiver fired.
        let (source, walked) = Files::start(home.0.clone(), Catalog::default);
        let count = iced::futures::executor::block_on(walked).expect("walked");
        assert_eq!(Some(count), source.entries().map(<[_]>::len));
        assert!(!source.search(&Query::new("dock"), 5).is_empty());
    }

    #[test]
    fn applications_show_their_theme_icon() {
        let hits = apps().search(&Query::new("kitty"), 1);
        let Tile::App(Some(icon)) = &hits[0].tile else {
            panic!("kitty has an icon: {:?}", hits[0].tile);
        };
        assert!(
            icon.ends_with("icons/hicolor/scalable/apps/kitty.svg"),
            "{icon:?}"
        );
        // No icon: the initial.
        assert_eq!(
            apps().search(&Query::new("kate"), 1)[0].tile,
            Tile::App(None)
        );
    }

    #[test]
    fn evidence_shows_its_icon_from_our_theme() {
        // Evidence's own desktop entry, and the theme of sherlock-ui beside the fixtures,
        // whose hicolor index describes the folders as the system's does.
        let tmp = TempDir::new("evidence");
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::fs::create_dir_all(tmp.0.join("applications")).expect("folder");
        let entry = tmp.0.join("applications/evidence.desktop");
        std::fs::copy(manifest.join("../evidence/evidence.desktop"), entry).expect("copied");
        let dirs = Dirs {
            config_home: fixtures().join("config"),
            config_dirs: Vec::new(),
            data_home: tmp.0.clone(),
            data_dirs: Vec::new(),
        };
        let ours = manifest.join("../sherlock-ui");
        let icons =
            icon_theme::Lookup::new(&[ours, fixtures()], None, None).context("Applications");
        let apps = Apps::load(&dirs, &["fr".into()], None).with_icons(icons);
        let hits = apps.search(&Query::new("evidence"), 1);
        let Tile::App(Some(icon)) = &hits[0].tile else {
            panic!("Evidence has an icon: {:?}", hits[0].tile);
        };
        assert!(
            icon.ends_with("sherlock-ui/icons/hicolor/scalable/apps/evidence.svg"),
            "{icon:?}"
        );
    }

    #[test]
    fn the_panel_renders_every_kind_of_result() {
        let home = home();
        let (mut launcher, _) = launcher_on(&home.0, Clipboard::Toolkit);
        for (query, name) in [
            ("re", "launcher-sections"),
            ("12,5 * 4", "launcher-calculator"),
            ("kit", "launcher-icon"),
        ] {
            let _ = launcher.update(Message::Query(query.into()));
            let size = (look::WIDTH, launcher.height());
            let mut ui = Simulator::with_size(launcher::settings(), size, launcher.view_panel());
            let snapshot = ui.snapshot(&theme::theme()).expect("rendered");
            let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
            let written = path.with_file_name(format!("{name}-tiny-skia.png"));
            let _ = std::fs::remove_file(&written);
            let _ = snapshot.matches_image(&path);
            assert!(
                std::fs::metadata(&written)
                    .expect("screenshot written")
                    .len()
                    > 2_000
            );
        }
        // The keys the launcher acts on stay the same.
        let tab = iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(keyboard::key::Named::Tab),
            modified_key: keyboard::Key::Named(keyboard::key::Named::Tab),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        });
        assert!(
            app::on_event(
                tab,
                iced::event::Status::Ignored,
                iced::window::Id::unique()
            )
            .is_none()
        );
    }
}
