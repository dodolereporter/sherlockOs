//! Headless end-to-end scenarios: each test drives the application with the keyboard on a
//! temporary folder, runs the tasks it starts as iced would, and checks the disk. The trash
//! lives in the temporary folder, never in the author's own.

use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use futures::StreamExt;
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::{Event, Pixels, Settings, Task};
use iced_runtime::Action;
use iced_test::simulator;
use sherlock_ui::color::{self, Palette, Theme};
use sherlock_ui::{metrics, theme, typography};

use crate::app::{App, Config, Location, Message};
use crate::places;
use crate::testing::TempDir;
use crate::trash::Trash;

/// A temporary home folder: `projet/` with `src/`, `a.txt` and `.cache`.
struct Bench {
    tmp: TempDir,
    app: App,
}

impl Bench {
    fn new(name: &str) -> Self {
        let tmp = TempDir::new(name);
        let project = tmp.path().join("projet");
        fs::create_dir_all(project.join("src")).expect("folders");
        fs::write(project.join("a.txt"), "contenu de a").expect("write");
        fs::write(project.join(".cache"), "").expect("write");
        let config = Config {
            home: tmp.path().to_path_buf(),
            start: project,
            favorites: places::favorites(tmp.path(), None),
            trash: Some(Trash::at(&tmp.path().join("donnees/Trash"))),
            measure: None,
            maximized: false,
            devices: None,
            watch_devices: false,
            xdg: None,
            desktops: Vec::new(),
            locales: Vec::new(),
        };
        let (mut app, task) = App::new(config);
        drain(&mut app, task);
        Self { tmp, app }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.tmp.path().join(relative)
    }

    fn key(&mut self, key: Key, modifiers: Modifiers) {
        let Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            text,
            repeat,
            ..
        }) = simulator::press_key(key, None)
        else {
            unreachable!("press_key returns a key press");
        };
        let event = keyboard::Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers,
            text,
            repeat,
        };
        self.send(Message::Key(event));
    }

    fn press(&mut self, key: Named) {
        self.key(Key::Named(key), Modifiers::empty());
    }

    fn ctrl(&mut self, letter: &str) {
        self.key(Key::Character(letter.into()), Modifiers::CTRL);
    }

    fn send(&mut self, message: Message) {
        let task = self.app.update(message);
        drain(&mut self.app, task);
    }

    fn names(&self) -> Vec<&str> {
        self.app.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn selected(&self) -> Vec<&str> {
        self.app
            .selection
            .iter()
            .map(|&i| self.app.entries[i].name.as_str())
            .collect()
    }

    /// Puts the cursor on the named item, with the keyboard.
    fn select(&mut self, name: &str) {
        self.press(Named::Home);
        while self.selected() != [name] {
            let before = self.app.cursor;
            self.press(Named::ArrowDown);
            assert_ne!(before, self.app.cursor, "{name} not found");
        }
    }
}

/// Runs the tasks as iced would, until none is left. Widget operations (focus, scrolling),
/// the clipboard and the window have no screen here.
fn drain(app: &mut App, task: Task<Message>) {
    let mut queue = vec![task];
    while let Some(task) = queue.pop() {
        let Some(stream) = iced_runtime::task::into_stream(task) else {
            continue;
        };
        let actions: Vec<_> = futures::executor::block_on(stream.collect());
        for action in actions {
            if let Action::Output(message) = action {
                queue.push(app.update(message));
            }
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read")
}

#[test]
fn keyboard_navigates_folders() {
    let mut bench = Bench::new("navigation");
    assert_eq!(bench.names(), ["src", "a.txt"]);
    bench.press(Named::ArrowDown);
    assert_eq!(bench.selected(), ["src"]);
    bench.press(Named::Enter);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    // Backspace goes up and keeps the folder we came from selected.
    bench.press(Named::Backspace);
    assert_eq!(bench.app.location, Location::Folder(bench.path("projet")));
    assert_eq!(bench.selected(), ["src"]);
    // Alt+Left goes back in history, Alt+Right goes forward again.
    bench.key(Key::Named(Named::ArrowLeft), Modifiers::ALT);
    assert_eq!(
        bench.app.location,
        Location::Folder(bench.path("projet/src"))
    );
    bench.key(Key::Named(Named::ArrowRight), Modifiers::ALT);
    assert_eq!(bench.app.location, Location::Folder(bench.path("projet")));
    // Ctrl+H shows hidden files, then hides them.
    bench.ctrl("h");
    assert_eq!(bench.names(), ["src", ".cache", "a.txt"]);
    bench.ctrl("h");
    assert_eq!(bench.names(), ["src", "a.txt"]);
    // Search filters, Escape clears it.
    bench.send(Message::Search("A.T".into()));
    assert_eq!(bench.names(), ["a.txt"]);
    bench.press(Named::Escape);
    assert_eq!(bench.names(), ["src", "a.txt"]);
}

#[test]
fn copy_paste_then_cut_paste_with_the_keyboard() {
    let mut bench = Bench::new("clipboard");
    bench.select("a.txt");
    bench.ctrl("c");
    bench.select("src");
    bench.press(Named::Enter);
    bench.ctrl("v");
    assert_eq!(read(&bench.path("projet/src/a.txt")), "contenu de a");
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");
    // The pasted copy is selected.
    assert_eq!(bench.selected(), ["a.txt"]);
    // Pasting a second time in the same place does not touch the first file.
    bench.ctrl("v");
    assert_eq!(bench.names(), ["a (copie).txt", "a.txt"]);

    // Cutting into a folder that already has this name: refused, nothing is overwritten.
    bench.select("a.txt");
    fs::write(bench.path("projet/src/a.txt"), "modifié").expect("write");
    bench.ctrl("x");
    bench.press(Named::Backspace);
    bench.ctrl("v");
    let notice = bench.app.notice.clone().expect("error message");
    assert!(notice.error, "{notice:?}");
    assert_eq!(read(&bench.path("projet/src/a.txt")), "modifié");
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");

    // Cutting then pasting elsewhere moves.
    bench.select("src");
    bench.press(Named::Enter);
    bench.select("a (copie).txt");
    bench.ctrl("x");
    bench.press(Named::Backspace);
    bench.ctrl("v");
    assert!(!bench.path("projet/src/a (copie).txt").exists());
    assert!(bench.path("projet/a (copie).txt").exists());
    // A cut item is pasted only once.
    bench.ctrl("v");
    assert_eq!(bench.names(), ["src", "a (copie).txt", "a.txt"]);

    // Ctrl+D duplicates alongside.
    bench.select("a.txt");
    bench.ctrl("d");
    assert!(bench.path("projet/a (copie 2).txt").exists());
}

#[test]
fn rename_and_create_a_folder_with_the_keyboard() {
    let mut bench = Bench::new("rename");
    bench.select("a.txt");
    bench.press(Named::F2);
    assert_eq!(
        bench.app.renaming.as_ref().map(|(_, n)| n.as_str()),
        Some("a.txt")
    );
    bench.send(Message::RenameInput("notes.txt".into()));
    bench.send(Message::RenameSubmit);
    assert!(bench.app.renaming.is_none());
    assert_eq!(read(&bench.path("projet/notes.txt")), "contenu de a");
    assert_eq!(bench.selected(), ["notes.txt"]);

    // A taken name is refused with a message.
    bench.press(Named::F2);
    bench.send(Message::RenameInput("src".into()));
    bench.send(Message::RenameSubmit);
    assert!(bench.app.notice.as_ref().is_some_and(|n| n.error));
    assert!(bench.path("projet/notes.txt").exists());

    // Ctrl+Shift+N creates a folder and immediately offers to name it.
    bench.key(
        Key::Character("N".into()),
        Modifiers::CTRL | Modifiers::SHIFT,
    );
    assert!(bench.path("projet/Nouveau dossier").is_dir());
    assert_eq!(bench.selected(), ["Nouveau dossier"]);
    assert!(bench.app.renaming.is_some());
    bench.press(Named::Escape);
    assert!(bench.app.renaming.is_none());
}

#[test]
fn trash_discards_restores_and_empties_with_confirmation() {
    let mut bench = Bench::new("trash");
    let trash = Trash::at(&bench.path("donnees/Trash"));
    bench.select("a.txt");
    bench.press(Named::Delete);
    assert!(!bench.path("projet/a.txt").exists());
    assert_eq!(trash.list().expect("trash").len(), 1);
    assert_eq!(bench.names(), ["src"]);

    // In the trash, the right-click menu restores, with the keyboard.
    bench.send(Message::Go(Location::Trash));
    assert_eq!(bench.names(), ["a.txt"]);
    bench.select("a.txt");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    assert!(bench.app.menu.is_some());
    bench.press(Named::ArrowDown);
    bench.press(Named::Enter);
    assert!(bench.app.menu.is_none());
    assert_eq!(read(&bench.path("projet/a.txt")), "contenu de a");
    assert!(bench.names().is_empty());

    // Emptying asks for confirmation: Escape cancels, nothing is deleted.
    bench.send(Message::Go(Location::Folder(bench.path("projet"))));
    bench.select("a.txt");
    bench.press(Named::Delete);
    bench.send(Message::Go(Location::Trash));
    bench.send(Message::Run(crate::app::Command::EmptyTrash));
    assert!(bench.app.confirm_empty);
    bench.press(Named::Escape);
    assert!(!bench.app.confirm_empty);
    assert_eq!(trash.list().expect("trash").len(), 1);
    bench.send(Message::Run(crate::app::Command::EmptyTrash));
    bench.send(Message::ConfirmEmpty);
    assert!(trash.list().expect("trash").is_empty());
    assert!(bench.names().is_empty());
}

#[test]
fn right_click_menu_is_driven_by_the_keyboard() {
    let mut bench = Bench::new("menu");
    bench.select("a.txt");
    bench.key(Key::Named(Named::ContextMenu), Modifiers::empty());
    let menu = bench.app.menu.clone().expect("menu open");
    assert!(menu.on_selection);
    // Open, then Open with: the right arrow opens the submenu.
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowDown);
    bench.press(Named::ArrowRight);
    assert!(bench.app.menu.as_ref().is_some_and(|m| m.submenu));
    bench.press(Named::ArrowLeft);
    assert!(bench.app.menu.as_ref().is_some_and(|m| !m.submenu));
    bench.press(Named::Escape);
    assert!(bench.app.menu.is_none());
    // The menu stays inside the window.
    bench.send(Message::Cursor(iced::Point::new(10_000.0, 10_000.0)));
    bench.send(Message::RightClick(None));
    let menu = bench.app.menu.clone().expect("menu open");
    assert!(!menu.on_selection);
    assert!(menu.at.x + metrics::CONTEXT_MENU_WIDTH <= bench.app.size.width);
}

/// A folder like the mock-up's: a git repository with a modified file, a new one and an
/// ignored folder.
fn mockup_folder() -> (TempDir, PathBuf) {
    let tmp = TempDir::new("mockup");
    let dir = tmp.path().join("Projets/compositeur");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args([
                "-c",
                "user.name=Essai",
                "-c",
                "user.email=essai@example.invalid",
            ])
            .args(args)
            .output()
            .expect("git is installed")
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    for sub in ["src", "docs", "essais", "target"] {
        fs::create_dir_all(dir.join(sub)).expect("folder");
    }
    for (name, content) in [
        (".gitignore", "target/\n"),
        ("Cargo.toml", "[package]\nname = \"compositeur\"\n"),
        ("Cargo.lock", "# Cargo.lock\n"),
        ("justfile", "verifier:\n    cargo test\n"),
        ("README.md", "# Compositeur\n"),
        ("rust-toolchain.toml", "[toolchain]\n"),
        ("src/main.rs", "fn main() {}\n"),
        ("docs/index.md", "# Docs\n"),
        ("essais/a.rs", "\n"),
    ] {
        fs::write(dir.join(name), content).expect("write");
    }
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "initial"]);
    fs::write(dir.join("src/main.rs"), "fn main() { todo!() }\n").expect("write");
    fs::write(dir.join("target/sortie"), "").expect("write");
    let docker = "FROM rust:1 AS build\nWORKDIR /src\nCOPY . .\nRUN cargo build --release\n";
    fs::write(dir.join("Dockerfile"), docker).expect("write");
    (tmp, dir)
}

/// Capture of the window, git status read, Dockerfile selected, menu and submenu open, in
/// `target/tmp/evidence-tiny-skia.png`. Compare it by eye with the mock-up: see
/// docs/evidence.md.
#[test]
fn window_renders_headless() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let (tmp, dir) = mockup_folder();
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: dir.clone(),
        favorites: places::favorites(tmp.path(), None),
        trash: Some(Trash::at(&tmp.path().join("donnees/Trash"))),
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (mut app, task) = App::new(config);
    drain(&mut app, task);
    let docker = app
        .entries
        .iter()
        .position(|e| e.name == "Dockerfile")
        .expect("Dockerfile listed");
    let (width, height) = metrics::REFERENCE_WINDOW;
    for message in [
        Message::Click(docker),
        Message::Cursor(iced::Point::new(450.0, 406.0)),
        Message::RightClick(Some(docker)),
        Message::Submenu(true),
    ] {
        let task = app.update(message);
        drain(&mut app, task);
    }
    app.settle();

    let settings = Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    };
    let mut ui = simulator::Simulator::with_size(settings.clone(), (width, height), app.view());
    let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
    // Like `CARGO_TARGET_TMPDIR`, which Cargo only gives to integration tests.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("captures folder");
    let path = dir.join("evidence.png");
    // Named after the renderer: `tiny-skia`, or `wgpu` with `ICED_TEST_BACKEND=wgpu`.
    let renderer = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let written = path.with_file_name(format!("evidence-{renderer}.png"));
    let _ = fs::remove_file(&written);
    assert!(snapshot.matches_image(&path).expect("writing the capture"));
    println!("capture: {}", written.display());

    // The open menu and its submenu are drawn whole: every entry row of both shows the menu
    // glass, never the black block a misdrawn shadow left (review 3), and the submenu stands
    // beside the menu without covering it.
    let picture = ::image::open(&written).expect("capture").to_rgb8();
    let scale = picture.width() as f32 / width;
    let menu = app.menu.clone().expect("menu open");
    let main_items = app.menu_items(menu.on_selection);
    let sub_items = app.submenu_items();
    let check = |left: f32, top: f32, items: &[crate::app::MenuItem], what: &str| {
        let mut y = top + metrics::MENU_PADDING + metrics::BORDER_WIDTH;
        for item in items {
            let row = match item {
                crate::app::MenuItem::Separator => {
                    2.0 * metrics::MENU_SEPARATOR_MARGIN.0 + metrics::BORDER_WIDTH
                }
                _ => metrics::MENU_ITEM_HEIGHT,
            };
            if !matches!(item, crate::app::MenuItem::Separator) {
                // Near the right edge of the row, where no text is drawn.
                let x = ((left + 8.0) * scale) as u32;
                let pixel = picture.get_pixel(x, ((y + row / 2.0) * scale) as u32);
                assert!(
                    pixel[2] > 40,
                    "{what}: row at {y} is too dark, {pixel:?}: the menu is not drawn whole"
                );
            }
            y += row;
        }
    };
    check(menu.at.x, menu.at.y, &main_items, "menu");
    let before = main_items
        .iter()
        .position(|item| *item == crate::app::MenuItem::OpenWith)
        .unwrap_or_default();
    let offset = App::menu_height(&main_items[..before])
        - 2.0 * (metrics::MENU_PADDING + metrics::BORDER_WIDTH);
    let sub_x = menu.at.x + metrics::CONTEXT_MENU_WIDTH + metrics::SUBMENU_GAP;
    assert!(
        sub_x >= menu.at.x + metrics::CONTEXT_MENU_WIDTH,
        "submenu overlaps the menu"
    );
    check(sub_x, menu.at.y + offset, &sub_items, "submenu");

    // The same window in the light theme, `evidence-light-<renderer>.png`, to compare with
    // FichiersClair.
    color::set_active(Theme::Light);
    let mut ui = simulator::Simulator::with_size(settings.clone(), (width, height), app.view());
    let snapshot = ui.snapshot(&theme::window_theme()).expect("capture");
    let light = written.with_file_name(format!("evidence-light-{renderer}.png"));
    let _ = fs::remove_file(&light);
    assert!(
        snapshot
            .matches_image(dir.join("evidence-light.png"))
            .expect("writing")
    );
    let picture = ::image::open(&light).expect("capture").to_rgb8();
    assert!(
        picture.get_pixel(4, picture.height() / 2)[0] > 200,
        "a light window"
    );
    color::set_active(Theme::Dark);

    // A click on a menu entry runs its command.
    let messages: Vec<_> = {
        let mut ui = simulator::Simulator::with_size(settings, (width, height), app.view());
        ui.click("Dupliquer").expect("entry found");
        ui.into_messages().collect()
    };
    assert!(
        matches!(
            messages.as_slice(),
            [Message::Run(crate::app::Command::Duplicate)]
        ),
        "{messages:?}"
    );
}

#[test]
fn a_new_theme_fades_the_window() {
    let _theme = crate::testing::in_theme(Theme::Dark);
    let mut bench = Bench::new("theme");
    let _ = bench.app.update(Message::Theme(Theme::Dark));
    let start = std::time::Instant::now();
    let _ = bench.app.update(Message::Theme(Theme::Light));
    assert!(
        bench.app.animating(bench.app.now()),
        "frames are asked for the fade"
    );
    let fade = sherlock_ui::motion::THEME_FADE;
    let _ = bench.app.update(Message::Frame(start + fade / 3));
    assert_ne!(color::palette(), &Palette::DARK);
    assert_ne!(color::palette(), &Palette::LIGHT);
    let _ = bench.app.update(Message::Frame(start + fade * 2));
    assert_eq!(color::palette(), &Palette::LIGHT);
    assert_eq!(theme::palette(&theme::window_theme()), &Palette::LIGHT);
    color::set_active(Theme::Dark);
}

/// Cost of one frame with 5,000 entries: view, layout, then software rendering. Run with
/// `cargo test --release -p evidence frame_cost -- --ignored --nocapture`.
#[test]
#[ignore = "measurement, not a test"]
fn frame_cost() {
    use std::time::Instant;
    let tmp = TempDir::new("cost");
    let dir = tmp.path().join("grand");
    fs::create_dir_all(&dir).expect("folder");
    for i in 0..5000 {
        fs::write(dir.join(format!("fichier-{i:05}.txt")), "x").expect("write");
    }
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: dir.clone(),
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (mut app, task) = App::new(config);
    drain(&mut app, task);
    app.settle();
    let settings = Settings {
        fonts: typography::FONTS.map(Cow::Borrowed).to_vec(),
        default_font: typography::UI,
        default_text_size: Pixels(typography::UI_SIZE),
        ..Settings::default()
    };
    let (width, height) = metrics::REFERENCE_WINDOW;
    {
        let mut warm =
            simulator::Simulator::with_size(settings.clone(), (width, height), app.view());
        let _ = warm.snapshot(&theme::window_theme());
    }
    let mut report = String::new();
    for offset in [0.0, 24.0, 48.0, 4000.0] {
        app.scroll.0 = offset;
        let t0 = Instant::now();
        let view = app.view();
        let t1 = Instant::now();
        let mut ui = simulator::Simulator::with_size(settings.clone(), (width, height), view);
        let t2 = Instant::now();
        let _ = ui.snapshot(&theme::window_theme()).expect("capture");
        let t3 = Instant::now();
        drop(ui);
        let line = format!(
            "offset {offset}: view {:?}, layout {:?}, render {:?}\n",
            t1 - t0,
            t2 - t1,
            t3 - t2
        );
        print!("{line}");
        report.push_str(&line);
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&dir).expect("folder");
    fs::write(dir.join("frame_cost.txt"), report).expect("write");
    if false {}
}

#[test]
fn devices_mount_open_unmount_and_eject_through_their_source() {
    use crate::devices::tests::FakeSource;
    use crate::devices::{DeviceKind, Volume};
    use std::sync::{Arc, Mutex};
    let tmp = TempDir::new("devices");
    let volume = |id: &str, name: &str, ejectable| Volume {
        id: id.into(),
        name: name.into(),
        kind: if ejectable {
            DeviceKind::UsbKey
        } else {
            DeviceKind::Internal
        },
        mount_point: None,
        size: 16_000_000_000,
        free: None,
        locked: false,
        ejectable,
    };
    let source = Arc::new(FakeSource {
        volumes: Mutex::new(vec![
            volume("/sda1", "CLE", true),
            volume("/sdb1", "disque occupé", false),
            Volume {
                locked: true,
                ..volume("/sdc1", "Coffre", true)
            },
        ]),
        root: tmp.path().join("media"),
    });
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: tmp.path().to_path_buf(),
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: Some(source.clone()),
        watch_devices: false,
        xdg: None,
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (app, task) = App::new(config);
    let mut bench = Bench { tmp, app };
    drain(&mut bench.app, task);
    assert_eq!(bench.app.volumes.len(), 3);

    // A click on an unmounted key mounts it, then opens it.
    bench.send(Message::OpenDevice("/sda1".into()));
    let point = bench.path("media/CLE");
    assert_eq!(bench.app.location, Location::Folder(point.clone()));
    assert!(bench.app.volumes[0].mount_point.is_some());

    // Its menu offers to open, unmount and eject it.
    bench.send(Message::DeviceMenu("/sda1".into()));
    let labels: Vec<_> = bench
        .app
        .menu_items(false)
        .into_iter()
        .filter_map(|item| match item {
            crate::app::MenuItem::Action { label, .. } => Some(label),
            _ => None,
        })
        .collect();
    assert_eq!(labels, ["Ouvrir", "Démonter", "Éjecter"]);
    bench.press(Named::Escape);

    // Ejecting leaves the volume first, then it disappears.
    bench.send(Message::EjectDevice("/sda1".into()));
    assert_eq!(bench.app.location, Location::Folder(bench.path("")));
    assert!(bench.app.volumes.iter().all(|v| v.id != "/sda1"));
    assert_eq!(bench.app.notice.as_ref().map(|n| n.error), Some(false));

    // A locked volume and a busy one each give a short message.
    bench.send(Message::OpenDevice("/sdc1".into()));
    assert!(bench.app.notice.as_ref().is_some_and(|n| n.error));
    bench.send(Message::OpenDevice("/sdb1".into()));
    bench.send(Message::DeviceMenu("/sdb1".into()));
    bench.send(Message::Run(crate::app::Command::DeviceUnmount));
    let notice = bench.app.notice.clone().expect("message");
    assert!(notice.error);
    assert_eq!(notice.text, sherlock_ui::texts::evidence::DEVICE_BUSY);
}

#[test]
fn open_with_lists_installed_applications_and_remembers_a_default() {
    let tmp = TempDir::new("open-with");
    let xdg = mime_apps::Dirs {
        config_home: tmp.path().join("xdg/config"),
        config_dirs: Vec::new(),
        data_home: tmp.path().join("xdg/data"),
        data_dirs: vec![tmp.path().join("xdg/share")],
    };
    let write = |relative: &str, content: &str| {
        let path = tmp.path().join("xdg").join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("folder");
        fs::write(path, content).expect("write");
    };
    write(
        "share/mime/globs2",
        "50:text/plain:*.txt\n50:text/markdown:*.md\n",
    );
    // `true` stands in for the editors: opening runs it and nothing happens.
    for (id, name) in [("kate", "Kate"), ("code", "Visual Studio Code")] {
        write(
            &format!("share/applications/{id}.desktop"),
            &format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=true %F\nMimeType=text/plain;\n"
            ),
        );
    }
    write(
        "share/applications/viewer.desktop",
        "[Desktop Entry]\nType=Application\nName=Images\nExec=true %U\nMimeType=image/png;\n",
    );
    let project = tmp.path().join("projet");
    fs::create_dir_all(&project).expect("folder");
    fs::write(project.join("notes.txt"), "notes").expect("write");
    fs::write(project.join("readme.md"), "# readme").expect("write");
    let config = Config {
        home: tmp.path().to_path_buf(),
        start: project,
        favorites: places::favorites(tmp.path(), None),
        trash: None,
        measure: None,
        maximized: false,
        devices: None,
        watch_devices: false,
        xdg: Some(xdg.clone()),
        desktops: Vec::new(),
        locales: Vec::new(),
    };
    let (app, task) = App::new(config);
    let mut bench = Bench { tmp, app };
    drain(&mut bench.app, task);
    bench.select("notes.txt");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Submenu(true));
    let names: Vec<&str> = bench
        .app
        .open_with
        .iter()
        .map(|a| a.name.as_str())
        .collect();
    assert_eq!(names, ["Visual Studio Code", "Kate"]);

    // The "always" switch keeps the menu open; choosing an application then writes it as the
    // default for the type, and the next open uses it.
    bench.send(Message::Run(crate::app::Command::ToggleAlways));
    assert!(bench.app.always && bench.app.menu.is_some());
    bench.send(Message::OpenWithApp("code.desktop".into()));
    let list = fs::read_to_string(xdg.config_home.join("mimeapps.list")).expect("mimeapps.list");
    assert!(
        list.contains("[Default Applications]\ntext/plain=code.desktop;"),
        "{list}"
    );
    let catalog = bench.app.catalog.clone().expect("catalog");
    assert_eq!(
        catalog.default_for("text/plain").map(|a| a.id.as_str()),
        Some("code.desktop")
    );

    // The switch went back off with that choice: opening another type once writes nothing.
    assert!(!bench.app.always);
    bench.select("readme.md");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Submenu(true));
    assert!(!bench.app.always);
    bench.send(Message::OpenWithApp("kate.desktop".into()));
    let after = fs::read_to_string(xdg.config_home.join("mimeapps.list")).expect("mimeapps.list");
    assert_eq!(
        after, list,
        "a choice without the switch must not touch mimeapps.list"
    );
    // Turning the switch on, then closing the menu, turns it off again.
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Submenu(true));
    bench.send(Message::Run(crate::app::Command::ToggleAlways));
    assert!(bench.app.always);
    bench.press(Named::Escape);
    assert!(!bench.app.always);

    // « Autre application… » opens the chooser of every application; Escape closes it.
    bench.select("notes.txt");
    bench.key(Key::Named(Named::F10), Modifiers::SHIFT);
    bench.send(Message::Run(crate::app::Command::ChooseApp));
    assert!(bench.app.chooser.is_some());
    bench.press(Named::Escape);
    assert!(bench.app.chooser.is_none());
}
